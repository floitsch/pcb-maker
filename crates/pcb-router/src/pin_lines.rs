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
    // Between pads of no net: the track of the class most nets have (the
    // board's default), not a neck class the router adds.
    let mut members = vec![0usize; board.classes.len()];
    for net in &board.nets {
        if let Some(count) = members.get_mut(net.class) {
            *count += 1;
        }
    }
    let common = (0..board.classes.len())
        .max_by_key(|class| (members[*class], std::cmp::Reverse(*class)))
        .map(|class| board.classes[class].trace_width);
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
        // Every pad whose box, grown by the gap, shares a bucket with this
        // pad's box.
        let mut candidates: Vec<usize> = Vec::new();
        for x in (pad.bounds.minimum[0] / cell).floor() as i64
            ..=(pad.bounds.maximum[0] / cell).floor() as i64
        {
            for y in (pad.bounds.minimum[1] / cell).floor() as i64
                ..=(pad.bounds.maximum[1] / cell).floor() as i64
            {
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
                // Between pads of no net: the common class's track.
                let Some(width) = pad
                    .widths
                    .iter()
                    .chain(them.widths.iter())
                    .copied()
                    .reduce(f64::min)
                    .or(common)
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

/// Which pins' lines the lattice gets (`Config::pin_lines`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lines {
    /// None: the regular lattice.
    Off,
    /// The pads' straight exits out of their rows. (Lines for the tracks
    /// of tight channels between pads were measured and left out:
    /// docs/router.md, "Lattice lines at pin rows (2026-11-07)".)
    Exits,
}

/// The router's lattice for `board`: the regular one (`Grid::choose` of
/// `pitches`) with the lines `which` pins ask for (`warp`).
pub fn lattice(board: &Board, pitches: &[f64], which: Lines) -> crate::grid::Grid {
    let mut grid = crate::grid::Grid::choose(board, pitches);
    // A lattice the maps cannot hold is refused before anything is
    // allocated; the lines are not looked for.
    if which == Lines::Off || grid.check_size().is_err() {
        return grid;
    }
    if let (Some(warp), _) = warp(board, &grid, which) {
        grid.nx = warp.lines[0].len();
        grid.ny = warp.lines[1].len();
        grid.origin = [warp.lines[0][0], warp.lines[1][0]];
        grid.warp = Some(std::sync::Arc::new(warp));
    }
    grid
}

/// `PCB_ROUTER_PIN_LINES`: `0` (off) or `1` (the exits' lines) in place
/// of the setting.
pub fn lines_override() -> Option<Lines> {
    match std::env::var("PCB_ROUTER_PIN_LINES").ok()?.as_str() {
        "0" | "off" => Some(Lines::Off),
        "1" | "exits" => Some(Lines::Exits),
        _ => None,
    }
}

/// A line is not added closer than this many pitches to another; a
/// regular line that near a needed position moves onto it instead.
pub const MIN_GAP: f64 = 0.25;

/// A part's band (its rows' lines at its phase) reaches this far beyond
/// its outermost exit line, mm: its tracks leave the rows and turn inside
/// a piece of the regular lattice.
pub const BAND_MARGIN: f64 = 1.0;

/// At most this share of an axis's lines is added (exits first, then the
/// channels by their slack).
pub const MAX_ADDED: f64 = 0.1;

/// What `warp` did, for the reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Placed {
    /// Lines the pins asked for that the regular lattice holds already.
    pub held: usize,
    pub added: usize,
    pub moved: usize,
    /// Bands of a part's rows at the part's phase (`BAND_MARGIN`), and the
    /// regular lines they shifted.
    pub bands: usize,
    pub shifted: usize,
    /// Bands not taken because the pads in them vote for the lattice's
    /// phase.
    pub outvoted: usize,
    /// Asked for and neither held, added nor moved (too close to a line
    /// another pin holds, or past the cap).
    pub left: usize,
}

/// Groups of lines on one axis whose slack intervals meet: their common
/// interval, whether any is an exit, and the narrowest slack.
fn groups(lines: &[PinLine], axis: usize) -> Vec<(f64, f64, bool)> {
    let mut intervals: Vec<(f64, f64, bool)> = lines
        .iter()
        .filter(|line| line.axis == axis)
        .map(|line| {
            (
                line.at - line.slack,
                line.at + line.slack,
                line.kind == LineKind::Exit,
            )
        })
        .collect();
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut out: Vec<(f64, f64, bool)> = Vec::new();
    for (low, high, exit) in intervals {
        match out.last_mut() {
            Some(group) if low <= group.1 + ON_LINE => {
                group.0 = group.0.max(low);
                group.1 = group.1.min(high);
                group.2 |= exit;
            }
            _ => out.push((low, high, exit)),
        }
    }
    out
}

/// The lattice `grid` (regular) with lines where `board`'s pins ask for
/// them (`which`): per axis, a needed line the regular lattice misses is
/// added between its neighbours, or a regular line closer than `MIN_GAP`
/// pitches is moved onto it; a regular line that holds a needed one stays.
/// `None` where nothing changes.
pub fn warp(
    board: &Board,
    grid: &crate::grid::Grid,
    which: Lines,
) -> (Option<crate::grid::Warp>, [Placed; 2]) {
    let mut placed = [Placed::default(); 2];
    if which == Lines::Off || grid.warp.is_some() {
        return (None, placed);
    }
    let mut lines = pin_lines(board, grid.pitch);
    lines.retain(|line| line.kind == LineKind::Exit);
    let min_gap = MIN_GAP * grid.pitch;
    let counts = [grid.nx, grid.ny];
    let mut result: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
    let mut changed = false;
    for axis in 0..2 {
        // (value, pinned, regular)
        let mut axis_lines: Vec<(f64, bool, bool)> = (0..counts[axis])
            .map(|index| (grid.origin[axis] + index as f64 * grid.pitch, false, true))
            .collect();
        // Bands first: a part whose exits on this axis all lie at one
        // phase of the pitch that the lattice misses gets the lattice of
        // that phase over its rows and `BAND_MARGIN` around them (its
        // lines shifted together, a piece of the regular lattice inside;
        // only the band's two edges are a step that is not the pitch).
        // A band that meets another of a different phase is left to the
        // lines one by one below.
        let pitch_nm = (grid.pitch * 1.0e6).round() as i64;
        let origin_nm = (grid.origin[axis] * 1.0e6).round() as i64;
        let mut parts: std::collections::BTreeMap<&str, Vec<&PinLine>> = Default::default();
        for line in lines
            .iter()
            .filter(|line| line.axis == axis && line.kind == LineKind::Exit)
        {
            parts.entry(part_of(&line.label)).or_default().push(line);
        }
        let mut bands: Vec<(i64, i64, i64, usize)> = Vec::new();
        // The narrow pads' anchors on this axis (nm), as `Grid::choose`
        // counts them: a pad narrower than 0.35 mm across the axis.
        let narrow: Vec<(i64, ())> = board
            .nets
            .iter()
            .flat_map(|net| net.terminals.iter())
            .filter(|terminal| {
                let bounds = board.obstacles[terminal.pad].shape.aabb();
                bounds.maximum[axis] - bounds.minimum[axis] < 0.35
            })
            .map(|terminal| ((terminal.anchor[axis] * 1.0e6).round() as i64, ()))
            .collect();
        for part_lines in parts.values() {
            // (A part the lattice holds is a band of no shift: it keeps
            // others' bands off its rows.)
            if part_lines.len() < MIN_PART_PADS {
                continue;
            }
            let residue = |line: &PinLine| {
                ((line.at * 1.0e6).round() as i64 - origin_nm).rem_euclid(pitch_nm)
            };
            let first = residue(part_lines[0]);
            let tolerance = |line: &PinLine| (line.slack * 1.0e6).floor() as i64;
            let one_phase = part_lines.iter().all(|line| {
                let offset = (residue(line) - first).rem_euclid(pitch_nm);
                offset.min(pitch_nm - offset) <= tolerance(line).min(tolerance(part_lines[0]))
            });
            if !one_phase {
                continue;
            }
            // The shift of the regular lines onto that phase, within half
            // a pitch either way; none where the regular lattice holds
            // every exit of the part within its slack already (a band
            // there only moved routes: docs/router.md, "Lattice lines
            // where the rows are open (2026-11-08)").
            let held = part_lines
                .iter()
                .all(|line| regular_has(grid.origin[axis], grid.pitch, line.at, line.slack));
            let shift = if held {
                0
            } else if first * 2 > pitch_nm {
                first - pitch_nm
            } else {
                first
            };
            let low = part_lines
                .iter()
                .map(|line| (line.at * 1.0e6).round() as i64)
                .min()
                .unwrap_or(0)
                - (BAND_MARGIN * 1.0e6) as i64;
            let high = part_lines
                .iter()
                .map(|line| (line.at * 1.0e6).round() as i64)
                .max()
                .unwrap_or(0)
                + (BAND_MARGIN * 1.0e6) as i64;
            // The pads in the band vote for its phase as the whole board's
            // narrow pads vote for the lattice's (`Grid::choose`): the band
            // is taken only where more of them lie at the part's phase than
            // at the lattice's (a panel's copies, a row among passives
            // aligned with the lattice, keep the lattice).
            if shift != 0 {
                let (mut at_part, mut at_lattice) = (0usize, 0usize);
                for &(anchor_nm, _) in narrow
                    .iter()
                    .filter(|(anchor_nm, _)| (low..=high).contains(anchor_nm))
                {
                    let residue = (anchor_nm - origin_nm).rem_euclid(pitch_nm);
                    let near = |phase: i64| {
                        let offset = (residue - phase).rem_euclid(pitch_nm);
                        offset.min(pitch_nm - offset) <= 100
                    };
                    at_part += near(first) as usize;
                    at_lattice += near(0) as usize;
                }
                if at_part <= at_lattice {
                    placed[axis].outvoted += 1;
                    continue;
                }
            }
            bands.push((low, high, shift, part_lines.len()));
        }
        // The most lines first; a band meeting an accepted one of another
        // phase (or ending closer than the gap to one) is not taken, one
        // of its phase merges.
        bands.sort_by(|a, b| b.3.cmp(&a.3).then(a.0.cmp(&b.0)));
        let mut accepted: Vec<(i64, i64, i64)> = Vec::new();
        let gap_nm = (min_gap * 1.0e6) as i64 + pitch_nm;
        for (low, high, shift, _) in bands {
            let meets =
                |other: &(i64, i64, i64)| low <= other.1 + gap_nm && other.0 <= high + gap_nm;
            if accepted
                .iter()
                .any(|other| meets(other) && other.2 != shift)
            {
                continue;
            }
            let (mut low, mut high) = (low, high);
            accepted.retain(|other| {
                if meets(other) {
                    low = low.min(other.0);
                    high = high.max(other.1);
                    false
                } else {
                    true
                }
            });
            accepted.push((low, high, shift));
        }
        accepted.retain(|band| band.2 != 0);
        for (low, high, shift) in &accepted {
            for (index, line) in axis_lines.iter_mut().enumerate() {
                let at = origin_nm + index as i64 * pitch_nm;
                if (*low..=*high).contains(&at) {
                    *line = ((at + shift) as f64 / 1.0e6, true, false);
                    placed[axis].shifted += 1;
                }
            }
        }
        placed[axis].bands = accepted.len();
        let mut wanted = groups(&lines, axis);
        let within = |axis_lines: &[(f64, bool, bool)], low: f64, high: f64| {
            let first = axis_lines.partition_point(|line| line.0 < low - ON_LINE);
            (first < axis_lines.len() && axis_lines[first].0 <= high + ON_LINE).then_some(first)
        };
        // Those the regular lattice holds pin their line.
        wanted.retain(|&(low, high, _)| match within(&axis_lines, low, high) {
            Some(index) => {
                axis_lines[index].1 = true;
                placed[axis].held += 1;
                false
            }
            None => true,
        });
        // Exits first, then the narrowest.
        wanted.sort_by(|a, b| b.2.cmp(&a.2).then((a.1 - a.0).total_cmp(&(b.1 - b.0))));
        let cap = ((counts[axis] as f64 * MAX_ADDED).floor() as usize).max(4);
        for (low, high, _) in wanted {
            if let Some(index) = within(&axis_lines, low, high) {
                axis_lines[index].1 = true;
                placed[axis].held += 1;
                continue;
            }
            let target = round_nm((low + high) / 2.0);
            let above = axis_lines.partition_point(|line| line.0 < target);
            if above == 0 || above >= axis_lines.len() {
                placed[axis].left += 1;
                continue;
            }
            let below = above - 1;
            let (gap_below, gap_above) =
                (target - axis_lines[below].0, axis_lines[above].0 - target);
            if gap_below >= min_gap && gap_above >= min_gap {
                if placed[axis].added < cap {
                    axis_lines.insert(above, (target, true, false));
                    placed[axis].added += 1;
                } else {
                    placed[axis].left += 1;
                }
                continue;
            }
            // Move the nearer line onto it, where it is a regular line no
            // pin holds and its own neighbours stay `min_gap` away.
            let nearer = if gap_below <= gap_above { below } else { above };
            let (value, pinned, regular) = axis_lines[nearer];
            let room = (nearer == 0 || target - axis_lines[nearer - 1].0 >= min_gap)
                && (nearer + 1 >= axis_lines.len() || axis_lines[nearer + 1].0 - target >= min_gap);
            if !pinned && regular && room && (value - target).abs() < min_gap + ON_LINE {
                axis_lines[nearer] = (target, true, false);
                placed[axis].moved += 1;
            } else {
                placed[axis].left += 1;
            }
        }
        changed |= placed[axis].added + placed[axis].moved + placed[axis].shifted > 0;
        result[axis] = axis_lines.into_iter().map(|line| line.0).collect();
    }
    if !changed {
        return (None, placed);
    }
    let warp = crate::grid::Warp::new(
        result,
        grid.pitch,
        [placed[0].added, placed[1].added],
        [
            placed[0].moved + placed[0].shifted,
            placed[1].moved + placed[1].shifted,
        ],
    );
    (Some(warp), placed)
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
    let regular = crate::grid::Grid {
        warp: None,
        ..crate::grid::Grid::choose(board, &[grid.pitch])
    };
    let grid = if grid.warp.is_some() { &regular } else { grid };
    let lines = pin_lines(board, grid.pitch);
    let missed = missing(&lines, grid);
    let added = positions(&missed, 0.005);
    {
        let which = Lines::Exits;
        let (warp, placed) = warp(board, grid, which);
        eprintln!(
            "pin lines: pitch {} lattice {} x {}: {which:?}: x {:?}, y {:?}; lattice {} x {}",
            grid.pitch,
            grid.nx,
            grid.ny,
            placed[0],
            placed[1],
            warp.as_ref().map_or(grid.nx, |warp| warp.lines[0].len()),
            warp.as_ref().map_or(grid.ny, |warp| warp.lines[1].len()),
        );
    }
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
            warp: None,
        };
        assert_eq!(missing(&lines, &grid(0.0)).len(), 5);
        assert!(missing(&lines, &grid(0.03)).is_empty());
    }

    #[test]
    fn a_channel_of_two_necked_tracks_asks_for_two_lines() {
        // The laptop keyboard's F23: a gap of 0.75 mm between two pads
        // holds two 0.127 mm tracks at 0.127 mm clearance, 0.254 mm apart
        // in a band of 0.369 mm, which a 0.125 mm lattice cannot hold.
        // (Four pads: a part of fewer has no row.)
        let mut board = row_board([3.0, 3.0], 4, 0.75 + 0.3, [0.3, 0.3], 0.2, 0.127);
        board.neck_width = 0.127;
        let lines = pin_lines(&board, 0.125);
        let channel: Vec<&PinLine> = lines
            .iter()
            .filter(|line| line.kind == LineKind::Channel && line.axis == 0 && line.label == "U1.1")
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
            warp: None,
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

    #[test]
    fn a_warped_lattice_keeps_its_regular_lines_and_indexes_by_its_tables() {
        // Exact-fit rows at a phase the lattice misses: the warp moves or
        // adds lines onto every pad centre and leaves the other lines.
        let mut board = row_board([2.033, 3.0], 6, 0.4, [0.2, 0.8], 0.2, 0.2);
        // A longer row of another part at phase 0 decides the regular
        // lattice's phase.
        let other = row_board([6.6, 7.0], 10, 0.4, [0.2, 0.8], 0.2, 0.2);
        for (index, mut net) in other.nets.into_iter().enumerate() {
            let mut obstacle = other.obstacles[index].clone();
            obstacle.label = format!("U2.{}", index + 1);
            obstacle.net = Some(board.nets.len() as u32);
            board.obstacles.push(obstacle);
            net.terminals[0].pad = board.obstacles.len() - 1;
            net.terminals[0].label = format!("U2.{}", index + 1);
            board.nets.push(net);
        }
        let grid = crate::grid::Grid::choose(&board, &[0.1]);
        assert!(
            (grid.origin[0] - 2.0)
                .rem_euclid(0.1)
                .min((2.0 - grid.origin[0]).rem_euclid(0.1))
                < 1.0e-9
        );
        let regular = crate::grid::Grid {
            warp: None,
            ..grid.clone()
        };
        let lattice = lattice(&board, &[0.1], Lines::Exits);
        let warp = lattice.warp.clone().expect("lines added");
        for index in 0..6 {
            let at = 2.033 + 0.4 * index as f64;
            assert!(
                warp.lines[0].iter().any(|line| (line - at).abs() < 1.0e-9),
                "no line at {at}"
            );
        }
        // The row's band (its exits and `BAND_MARGIN` around) is the
        // lattice at the row's phase; outside it every line is a regular
        // one, to the bit.
        let regular_lines: Vec<f64> = (0..regular.nx)
            .map(|index| regular.origin[0] + index as f64 * regular.pitch)
            .collect();
        let band = 2.033 - BAND_MARGIN - 1.0e-6..=2.033 + 2.0 + BAND_MARGIN + 1.0e-6;
        for line in &warp.lines[0] {
            if band.contains(line) {
                let phase = (line - 2.033).rem_euclid(0.1);
                assert!(
                    phase.min(0.1 - phase) < 1.0e-9,
                    "{line} off the row's phase"
                );
            } else {
                assert!(
                    regular_lines.contains(line),
                    "{line} moved outside the band"
                );
            }
        }
        assert_eq!(warp.lines[0].len(), regular.nx);
        assert_eq!(lattice.nx, warp.lines[0].len());
        // The tables: centres, the nearest node, ranges and fractions.
        let first = warp.lines[0]
            .iter()
            .position(|line| (line - 2.033).abs() < 1.0e-9)
            .unwrap();
        let cell = lattice.nearest_node([2.034, 3.0]).unwrap();
        assert_eq!(lattice.xy(cell).0, first);
        assert!((lattice.center(first, 0)[0] - 2.033).abs() < 1.0e-9);
        assert!((lattice.index_of(0, 2.033) - first as f64).abs() < 1.0e-9);
        let (x0, _, x1, _) = lattice
            .node_range(crate::geometry::Aabb {
                minimum: [2.0, 0.0],
                maximum: [2.1, 1.0],
            })
            .unwrap();
        assert!(warp.lines[0][x0] >= 2.0 && warp.lines[0][x1] <= 2.1);
        assert!(warp.lines[0][x0 - 1] < 2.0 && warp.lines[0][x1 + 1] > 2.1);
        // Steps: none shorter than the gap, none longer than a pitch and a
        // half (a band's edge); far from the row the lattice is regular.
        assert!(warp.min_step >= MIN_GAP * 0.1 - 1.0e-9);
        assert!(warp.max_step <= 0.1 * 1.5 + 1.0e-9);
        assert!(lattice.uniform_around(5, 5, 3));
        // Inside the band a piece of the regular lattice; at its edge not.
        assert!(lattice.uniform_around(first, 30, 3));
        let edge = warp.lines[0]
            .iter()
            .position(|line| (line - 5.033).abs() < 1.0e-9)
            .unwrap();
        assert!(!lattice.uniform_around(edge, 30, 1));
        // Off, or a board whose rows the lattice holds: no warp.
        assert!(super::lattice(&board, &[0.1], Lines::Off).warp.is_none());
        let aligned = row_board([2.0, 3.0], 6, 0.4, [0.2, 0.8], 0.2, 0.2);
        assert!(
            super::lattice(&aligned, &[0.1], Lines::Exits)
                .warp
                .is_none()
        );
    }

    #[test]
    fn a_row_the_lattice_holds_within_its_slack_gets_no_band() {
        // 0.4 mm pitch, 0.2 mm pads, 0.2 mm track at 0.17 mm clearance: each
        // exit has 0.03 mm of slack. A row 0.01 mm off the lattice's phase is
        // held (a band there only moved routes: x1934's Hub and
        // split72_right at 0.1 mm); 0.033 mm off it is not.
        let held = row_board([2.01, 3.0], 6, 0.4, [0.2, 0.8], 0.2, 0.17);
        let grid = crate::grid::Grid {
            origin: [0.0, 0.0],
            pitch: 0.1,
            nx: 100,
            ny: 100,
            warp: None,
        };
        let lines = pin_lines(&held, 0.1);
        assert!(lines.iter().any(|line| line.kind == LineKind::Exit));
        assert!(missing(&lines, &grid).is_empty());
        let (warp, placed) = warp(&held, &grid, Lines::Exits);
        assert!(warp.is_none(), "{placed:?}");
        assert_eq!(placed[0].shifted, 0);
        let off = row_board([2.033, 3.0], 6, 0.4, [0.2, 0.8], 0.2, 0.17);
        let (warp, placed) = super::warp(&off, &grid, Lines::Exits);
        assert!(warp.is_some());
        assert_eq!(placed[0].bands, 1, "{placed:?}");
    }

    /// Two exact-fit rows (0.4 mm pitch, 0.2 mm pads, track and clearance)
    /// of two parts at phases 0.033 mm apart, one layer: the longer row
    /// decides the regular lattice's phase, and the other's pads leave only
    /// along their centre lines, each to a pad of its net fanned out
    /// below. Without pin lines the misaligned row's pads stay open; with
    /// the exits' lines (the row's band) every net routes, and the routes
    /// keep every clearance.
    #[test]
    fn an_exact_fit_row_off_the_lattice_routes_with_its_band() {
        let mut board = row_board([2.033, 3.0], 6, 0.4, [0.2, 0.8], 0.2, 0.2);
        board.layer_count = 1;
        board.outline = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 9.0], [0.0, 9.0]];
        for obstacle in &mut board.obstacles {
            obstacle.layers = 1;
        }
        for net in &mut board.nets {
            net.terminals[0].layers = 1;
        }
        let other = row_board([6.0, 1.0], 8, 0.4, [0.2, 0.8], 0.2, 0.2);
        for (index, mut net) in other.nets.into_iter().enumerate() {
            let mut obstacle = other.obstacles[index].clone();
            obstacle.label = format!("U2.{}", index + 1);
            obstacle.net = Some(board.nets.len() as u32);
            obstacle.layers = 1;
            board.obstacles.push(obstacle);
            net.terminals[0].pad = board.obstacles.len() - 1;
            net.terminals[0].label = format!("U2.{}", index + 1);
            net.terminals[0].layers = 1;
            board.nets.push(net);
        }
        // Each U1 pad's net goes to a pad 1 mm apart from the next, 4 mm
        // below the row.
        for index in 0..6 {
            let at = [0.8 + index as f64 * 1.2, 7.5];
            board.obstacles.push(Obstacle {
                shape: rect(at, [0.6, 0.6]),
                layers: 1,
                kind: ObstacleKind::Copper,
                net: Some(index as u32),
                clearance: 0.0,
                clearance_override: None,
                plated: false,
                blocks_tracks: true,
                blocks_vias: true,
                label: format!("P{index}.1"),
            });
            board.nets[index].terminals.push(Terminal {
                anchor: at,
                layers: 1,
                pad: board.obstacles.len() - 1,
                contact: None,
                label: format!("P{index}.1"),
            });
        }
        let open_with = |which: Lines| {
            let config = crate::Config {
                pitches: vec![0.1],
                pin_lines: which,
                ..crate::Config::default()
            };
            let result = crate::route(&board, &config);
            let violations = crate::verify(&board, &result.routes);
            assert!(violations.is_empty(), "{which:?}: {violations:?}");
            assert_eq!(result.grid.warp.is_some(), which == Lines::Exits);
            (0..6)
                .filter(|net| result.status[*net] != crate::NetStatus::Routed)
                .count()
        };
        assert!(open_with(Lines::Off) > 0);
        assert_eq!(open_with(Lines::Exits), 0);
    }
}
