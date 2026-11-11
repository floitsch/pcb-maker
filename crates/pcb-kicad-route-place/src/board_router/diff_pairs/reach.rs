// Copyright (C) 2026 Toit contributors.

//! Whether a track of a pair's width gets from a pad to a point, or out of
//! a window round a pin: a flood fill on a fine grid (`grid_reaches`).

use super::*;
use pcb_router as core;

/// `grid_reaches`' grid pitch (half the router's lattice).
pub(super) const REACH_PITCH: f64 = 0.05;
/// Round a narrow pad the router necks its tracks down this far (its
/// `neck_reach`, the default).
pub(super) const REACH_NECK: f64 = 1.5;
/// The room round the pad (and the goal) `grid_reaches` looks in.
pub(super) const REACH_MARGIN: f64 = 1.5;

/// Whether a track of the pair's width can go from the pad of `terminal`
/// (of `polarity`) to `to` on `layers`, past the obstacles `near` and the
/// `extra` shapes (with their clearance; a name each for the diagnostics):
/// `grid_reaches` (`PCB_ROUTER_PAIRS_REACH`'s `grid` verdict).
pub(super) fn stub_reaches(
    board: &core::Board,
    near: &[usize],
    pair: &Pair,
    (terminal, polarity): (&core::Terminal, usize),
    to: Point,
    layers: core::LayerMask,
    extra: &[(core::Shape, f64, &str)],
) -> bool {
    let nets = [pair.positive, pair.negative];
    grid_reaches(
        board,
        near,
        pair,
        (terminal.pad, terminal.anchor),
        (Some(nets[polarity]), Some(nets[1 - polarity])),
        Some(to),
        layers,
        extra,
    )
}

/// At a pair point across a pin (`PointKind::Pin`): the first pin between
/// the pads (another net's copper on the line between their centres) that
/// gets no way out (`grid_reaches` to its window's border) past straight
/// stubs from the pads to their ends and the run's lead at `found`: a
/// ground pin between a fine-pitch part's D+ and D- whose only way out lay
/// between them (MSign's UQFN USB muxes).
pub(super) fn walled_pin(
    board: &core::Board,
    pair: &Pair,
    class: &core::RuleClass,
    (p, n): (&core::Terminal, &core::Terminal),
    found: &Launch,
) -> Option<String> {
    let nets = [pair.positive, pair.negative];
    let half = pair.width / 2.0;
    let stubs = [
        core::Shape::Capsule {
            start: p.anchor,
            end: found.stub_ends[0],
            radius: half,
        },
        core::Shape::Capsule {
            start: n.anchor,
            end: found.stub_ends[1],
            radius: half,
        },
        core::Shape::Capsule {
            start: found.reserved,
            end: found.at,
            radius: class.trace_width / 2.0,
        },
    ];
    let extra = [
        (stubs[0].clone(), pair.clearance, "the positive stub"),
        (stubs[1].clone(), pair.clearance, "the negative stub"),
        (
            stubs[2].clone(),
            class.clearance.max(pair.clearance),
            "the run's lead",
        ),
    ];
    scanned(board.obstacles.len());
    board
        .obstacles
        .iter()
        .enumerate()
        .filter(|(_, obstacle)| {
            obstacle.kind == core::ObstacleKind::Copper
                && obstacle.layers & p.layers & n.layers & found.layers != 0
                && obstacle.net.is_none_or(|net| !nets.contains(&net))
                && obstacle.shape.distance_to_segment(p.anchor, n.anchor) < EPSILON
        })
        .find(|(index, obstacle)| {
            let bounds = obstacle.shape.aabb();
            let centre = scale(add(bounds.minimum, bounds.maximum), 0.5);
            let near = obstacles_near(board, centre, REACH_MARGIN + 1.0);
            !grid_reaches(
                board,
                &near,
                pair,
                (*index, centre),
                (obstacle.net, None),
                None,
                found.layers,
                &extra,
            )
        })
        .map(|(_, obstacle)| obstacle.label.clone())
}

/// A flood fill on a grid of `REACH_PITCH`: a track of the pair's width from the
/// pad `pad` (an obstacle, its anchor `from`) of net `own` (`other` kept
/// at the pair's gap) to `goal`, or where there is none, out of the window
/// round the pad (`REACH_MARGIN`): a pin that does not get out is walled in.
#[allow(clippy::too_many_arguments)]
pub(super) fn grid_reaches(
    board: &core::Board,
    near: &[usize],
    pair: &Pair,
    (pad_index, from): (usize, Point),
    (own, other): (Option<core::NetId>, Option<core::NetId>),
    goal: Option<Point>,
    layers: core::LayerMask,
    extra: &[(core::Shape, f64, &str)],
) -> bool {
    let to = goal.unwrap_or(from);
    let mut grid = ReachGrid::new(board, pair, (pad_index, from), to);
    for &index in near {
        let obstacle = &board.obstacles[index];
        if obstacle.layers & layers == 0
            || index == pad_index
            || (own.is_some() && obstacle.net == own)
        {
            continue;
        }
        // The pad's mask opening is left by its stub.
        if obstacle.kind == core::ObstacleKind::Keepout
            && obstacle.label.starts_with("solder mask opening")
            && obstacle.shape.contains(from)
        {
            continue;
        }
        let mut keep = keep_from(board, obstacle, pair.clearance);
        if other.is_some() && obstacle.net == other {
            keep = keep.min(pair.gap);
        }
        grid.sources.push(obstacle.label.clone());
        let source = grid.sources.len() as u32;
        grid.block(&obstacle.shape, keep, source);
    }
    for (shape, keep, label) in extra {
        grid.sources.push(label.to_string());
        let source = grid.sources.len() as u32;
        grid.block(shape, *keep, source);
    }
    // The board's edge.
    grid.sources.push("the edge".into());
    let edge_source = grid.sources.len() as u32;
    for (a, b) in core::geometry::polygon_edges(&board.outline) {
        let edge = core::Shape::Capsule {
            start: a,
            end: b,
            radius: 0.0,
        };
        grid.block(&edge, board.edge_clearance, edge_source);
    }
    tested(grid.tests + board.outline.len());
    grid.flood(board, (pad_index, from), goal)
}

/// `grid_reaches`' grid: the window round the pad and the goal, per cell
/// what blocks it (0 free, else an index into `sources` from 1), and the
/// distance tests made.
struct ReachGrid {
    low: Point,
    high: Point,
    columns: usize,
    rows: usize,
    blocked: Vec<u32>,
    sources: Vec<String>,
    tests: usize,
    /// Half the track's width, and round a pad narrower than the track and
    /// its clearance (`narrow`, its anchor `from`) half the neck width.
    half: f64,
    narrow: bool,
    from: Point,
    neck_half: f64,
}

impl ReachGrid {
    /// The grid round the pad `pad_index` (its anchor `from`) and `to`,
    /// every cell free.
    fn new(board: &core::Board, pair: &Pair, (pad_index, from): (usize, Point), to: Point) -> Self {
        let half = pair.width / 2.0;
        let low = [
            from[0].min(to[0]) - REACH_MARGIN,
            from[1].min(to[1]) - REACH_MARGIN,
        ];
        let high = [
            from[0].max(to[0]) + REACH_MARGIN,
            from[1].max(to[1]) + REACH_MARGIN,
        ];
        let columns = ((high[0] - low[0]) / REACH_PITCH).ceil() as usize + 1;
        let rows = ((high[1] - low[1]) / REACH_PITCH).ceil() as usize + 1;
        // Per cell: 0 free, else what blocks it (`sources`, from 1).
        let blocked = vec![0u32; columns * rows];
        // Round a pad narrower than the track and its clearance the router
        // routes at the neck width (its `neck_reach` of the pad's anchor).
        let pad_bounds = board.obstacles[pad_index].shape.aabb();
        let narrow = (pad_bounds.maximum[0] - pad_bounds.minimum[0])
            .min(pad_bounds.maximum[1] - pad_bounds.minimum[1])
            < pair.width + pair.clearance;
        let neck_half = board.neck_width.min(pair.width) / 2.0;
        ReachGrid {
            low,
            high,
            columns,
            rows,
            blocked,
            sources: Vec::new(),
            tests: 0,
            half,
            narrow,
            from,
            neck_half,
        }
    }

    /// The centre of a cell.
    fn cell(&self, column: usize, row: usize) -> Point {
        [
            self.low[0] + column as f64 * REACH_PITCH,
            self.low[1] + row as f64 * REACH_PITCH,
        ]
    }

    /// Whether a track at `at` is necked (round a narrow pad).
    fn necked(&self, at: Point) -> bool {
        self.narrow
            && (at[0] - self.from[0]).abs() <= REACH_NECK
            && (at[1] - self.from[1]).abs() <= REACH_NECK
    }

    /// Blocks the cells within `keep` and half the track's width of `shape`
    /// (by `source`).
    fn block(&mut self, shape: &core::Shape, keep: f64, source: u32) {
        let (low, high, rows, columns) = (self.low, self.high, self.rows, self.columns);
        let reach = keep + self.half;
        let bounds = shape.aabb();
        if bounds.maximum[0] + reach < low[0]
            || bounds.minimum[0] - reach > high[0]
            || bounds.maximum[1] + reach < low[1]
            || bounds.minimum[1] - reach > high[1]
        {
            return;
        }
        let first = |value: f64, origin: f64| {
            ((value - reach - origin) / REACH_PITCH).floor().max(0.0) as usize
        };
        let last = |value: f64, origin: f64, count: usize| {
            (((value + reach - origin) / REACH_PITCH).ceil().max(0.0) as usize).min(count - 1)
        };
        for row in first(bounds.minimum[1], low[1])..=last(bounds.maximum[1], low[1], rows) {
            for column in
                first(bounds.minimum[0], low[0])..=last(bounds.maximum[0], low[0], columns)
            {
                let index = row * columns + column;
                if self.blocked[index] == 0 {
                    self.tests += 1;
                    let at = self.cell(column, row);
                    let width = if self.necked(at) {
                        self.neck_half
                    } else {
                        self.half
                    };
                    if shape.distance_to_point(at) < keep + width {
                        self.blocked[index] = source;
                    }
                }
            }
        }
    }

    /// A flood fill from the pad's free cells: whether it reaches `goal`
    /// (within `LAUNCH_CONTACT`), or without one the window's border.
    fn flood(
        &self,
        board: &core::Board,
        (pad_index, from): (usize, Point),
        goal: Option<Point>,
    ) -> bool {
        let (columns, rows) = (self.columns, self.rows);
        let to = goal.unwrap_or(from);
        let pad = &board.obstacles[pad_index].shape;
        let mut queue = std::collections::VecDeque::new();
        let mut seen = vec![false; columns * rows];
        for row in 0..rows {
            for column in 0..columns {
                let index = row * columns + column;
                // The pad's own cells, blocked or not: a track leaves a pad
                // from inside it (the router's pad exits).
                if pad.contains(self.cell(column, row)) {
                    seen[index] = true;
                    queue.push_back((column, row));
                }
            }
        }
        if queue.is_empty() {
            // A pad smaller than a cell: the cell at its anchor.
            let column =
                (((from[0] - self.low[0]) / REACH_PITCH).round() as usize).min(columns - 1);
            let row = (((from[1] - self.low[1]) / REACH_PITCH).round() as usize).min(rows - 1);
            seen[row * columns + column] = true;
            queue.push_back((column, row));
        }
        let seeds = queue.len();
        let mut closest = f64::INFINITY;
        let mut walls: BTreeSet<u32> = BTreeSet::new();
        while let Some((column, row)) = queue.pop_front() {
            let distance = core::geometry::distance(self.cell(column, row), to);
            let out = goal.is_none()
                && (column == 0 || row == 0 || column + 1 == columns || row + 1 == rows);
            if out || (goal.is_some() && distance <= LAUNCH_CONTACT) {
                return true;
            }
            closest = closest.min(distance);
            for (dc, dr) in [
                (1i64, 0i64),
                (-1, 0),
                (0, 1),
                (0, -1),
                (1, 1),
                (1, -1),
                (-1, 1),
                (-1, -1),
            ] {
                let (c, r) = (column as i64 + dc, row as i64 + dr);
                if c < 0 || r < 0 || c >= columns as i64 || r >= rows as i64 {
                    continue;
                }
                let index = r as usize * columns + c as usize;
                if self.blocked[index] != 0 {
                    walls.insert(self.blocked[index]);
                    continue;
                }
                if !seen[index] {
                    seen[index] = true;
                    queue.push_back((c as usize, r as usize));
                }
            }
        }
        if debug() {
            eprintln!(
                "diff pair reach {}: {seeds} seeds, closest {closest:.3} of {:.3}; walls {:?}",
                board.obstacles[pad_index].label,
                core::geometry::distance(from, to),
                walls
                    .iter()
                    .map(|wall| self.sources[*wall as usize - 1].as_str())
                    .collect::<Vec<_>>()
            );
        }
        false
    }
}
