// Copyright (C) 2026 Toit contributors.

//! Launches: where a link's coupled run starts at a pair point, searched
//! outward from between the pads (`launch`), with a crossover in its lead
//! where asked for, and the room a crossover leaves its neighbours.

use super::*;
use pcb_router as core;

/// The launch's search: steps outward from between the pads, along
/// directions turned from the pads' normal by these angles (degrees).
pub(super) const LAUNCH_STEP: f64 = 0.05;
const LAUNCH_REACH: f64 = 4.0;
const LAUNCH_ANGLES: [f64; 13] = [
    0.0, 15.0, -15.0, 30.0, -30.0, 45.0, -45.0, 60.0, -60.0, 75.0, -75.0, 90.0, -90.0,
];
/// What a launch turned a right angle from the pads' normal costs, in
/// millimetres of stub.
const ANGLE_COST: f64 = 1.0;
/// Room beyond the clearance around a launch point, so that lattice nodes
/// next to it are legal.
pub(super) const LAUNCH_MARGIN: f64 = 0.15;
/// The nodes a coupled search starts from: within this of the launch.
pub(super) const LAUNCH_CONTACT: f64 = 0.15;

/// What a launch's run start has in front of it (`launch`): a run whose
/// stubs go straight to it, a crossover in its lead (`crossover`), or stubs
/// that cross on their way to it, which the router routes with a via
/// (a crossed end without a crossover): the start then keeps room for
/// that via beyond the pads (`CROSSING_ROOM`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Lead {
    Straight,
    Crossover,
    Crossing,
}

/// A launch: where a link's coupled run starts at a pair point, and where
/// its net starts in the route.
#[derive(Clone, Debug)]
pub(super) struct Launch {
    /// Where the link's net starts: the end of the run's straight lead.
    pub at: Point,
    /// Where the run starts: the pads' stubs (ordinary routing of the
    /// pair's nets) end beside it, an offset to either side.
    pub start: Point,
    /// Where the link's net's copper begins on the lead: its track clear
    /// of the stubs' ends' copper, which reaches `ends` past the start.
    pub reserved: Point,
    pub ends: Point,
    /// The run's direction away from the pads.
    pub outward: Point,
    /// The layer (a bit) the run starts on.
    pub layers: core::LayerMask,
    /// The side of the run the positive pad is on (1 left, -1 right).
    pub side: f64,
    /// Whether straight stubs from the pads to the stubs' ends keep every
    /// clearance (`stubs_clear`): the link carries them, the router does
    /// not route them.
    pub fixed: bool,
    /// Where those stubs start on the pads (positive, negative).
    pub stubs: [Point; 2],
    /// Where the stubs end (positive, negative), on the pads' sides.
    pub stub_ends: [Point; 2],
    /// A crossover in the lead (`crossover`; asked for at a crossed end):
    /// the run past it has the nets on the other sides.
    pub crossover: Option<Crossover>,
    /// With a crossover: how far its copper keeps beyond the clearance from
    /// the neighbouring pairs' launches (`neighbour_room`; infinite where
    /// none is near, and without a crossover).
    pub room: f64,
}

/// Where a neighbouring pair's link end launches (`launch` on the board
/// before any link's copper): its straight stubs from the pads to the
/// stubs' ends and its run's lead. A crossover keeps its clearance from
/// them (`neighbour_room`): else it pushes that pair's stubs out of their
/// gap, or its launch away from the pads (oculink_port's TX pairs beside
/// the RX crossovers, 2026-10-25).
#[derive(Clone, Debug)]
pub(super) struct Neighbour {
    pub pair: usize,
    pub layers: core::LayerMask,
    /// Capsules (start, end, radius, clearance): the two stubs and the
    /// leads from their ends (the pair's width and clearance) and the lead
    /// (the link's envelope with the launch's margin for lattice nodes, the
    /// link class's clearance), as `launch` checks them.
    pub copper: [(Point, Point, f64, f64); 5],
}

/// How far the copper of a crossover (`found`) and the run's lead past it
/// (`lead`: a capsule on `layers`, the link's envelope) keep beyond their
/// clearance from the launches of the `neighbours` (the least margin;
/// infinite where none is near; below zero where they reach into one).
pub(super) fn neighbour_room(
    found: &Crossover,
    lead: (Point, Point, f64),
    layers: core::LayerMask,
    pair: &Pair,
    neighbours: &[&Neighbour],
) -> f64 {
    let mut room = f64::INFINITY;
    for neighbour in neighbours {
        for &(start, end, radius, clearance) in &neighbour.copper {
            let keep = pair.clearance.max(clearance);
            if neighbour.layers & layers != 0 {
                let distance = core::geometry::segment_segment_distance(lead.0, lead.1, start, end);
                room = room.min(distance - lead.2 - radius - keep);
            }
            for &(layer, from, to) in found.segments.iter().flatten() {
                if neighbour.layers & (1 << layer) == 0 {
                    continue;
                }
                let distance = core::geometry::segment_segment_distance(from, to, start, end);
                room = room.min(distance - pair.width / 2.0 - radius - keep);
            }
            for &via in found.vias.iter().flatten() {
                let distance = core::geometry::point_segment_distance(via, start, end);
                let need = (pair.via_diameter / 2.0 + keep)
                    .max(pair.via_drill / 2.0 + pair.hole_clearance);
                room = room.min(distance - radius - need);
            }
        }
    }
    room
}

/// The straight lead of a run from its start to its launch, at least, in
/// offsets (`Pair::offset`): a turn of the search at the launch leaves the
/// offset tracks room up to the miter limit (`offset_line`), away from the
/// pads' stubs.
const LEAD: f64 = 2.5;

/// The room a crossover leaves the neighbouring pairs' launches beyond its
/// clearance, wanted: this many of its pair's gap plus clearance
/// (`neighbour_room`); a crossover with less costs up to `ROOM_COST` (mm,
/// more than the launch search's reach: the room comes first).
const ROOM: f64 = 1.0;
const ROOM_COST: f64 = 5.0;

/// Where the part of a terminal (`REF.pad`) sits: the middle of its pads'
/// copper, when it has more than the two given. For a pair point of two
/// parts (`series_parts`: the pads `p` and `n` of two two-pad parts side
/// by side), the middle of both parts' pads: the launch leaves the parts on
/// the pads' side, not through the bodies towards their other pads (a
/// pair row's source side launched toward a far end beside the row, across
/// the parts and past the other pair's pads: right_io's C150 and C151).
fn part_middle(board: &core::Board, label: &str, other: &str) -> Option<Point> {
    let reference = |label: &str| label.rsplit_once('.').map(|(part, _)| part.to_string());
    let mut prefixes = vec![format!("{}.", reference(label)?)];
    if let Some(second) = reference(other)
        && format!("{second}.") != prefixes[0]
    {
        prefixes.push(format!("{second}."));
    }
    let (mut sum, mut count) = ([0.0, 0.0], 0usize);
    scanned(board.obstacles.len());
    for obstacle in &board.obstacles {
        if obstacle.kind == core::ObstacleKind::Copper
            && prefixes.iter().any(|prefix| {
                obstacle
                    .label
                    .strip_prefix(prefix.as_str())
                    .is_some_and(|pad| !pad.contains(' '))
            })
        {
            let bounds = obstacle.shape.aabb();
            sum = add(sum, scale(add(bounds.minimum, bounds.maximum), 0.5));
            count += 1;
        }
    }
    (count > 2).then(|| scale(sum, 1.0 / count as f64))
}

/// Where a straight stub of `width` from the pad of `terminal` to `end`
/// starts: where the line from the pad's anchor to the end leaves the pad,
/// less half the width (its end within the pad), so that only what lies
/// beyond the pad is the stub's (a card edge's finger reaches under the
/// edge's mask opening; the stub leaves its inner end).
fn stub_start(board: &core::Board, terminal: &core::Terminal, end: Point, width: f64) -> Point {
    let shape = &board.obstacles[terminal.pad].shape;
    let (anchor, length) = (
        terminal.anchor,
        core::geometry::distance(terminal.anchor, end),
    );
    if length < EPSILON || !shape.contains(anchor) {
        return anchor;
    }
    let direction = unit(sub(end, anchor));
    // The exit, to 0.01 mm.
    let (mut inside, mut outside) = (0.0, length);
    if shape.contains(end) {
        return anchor;
    }
    while outside - inside > 0.01 {
        let middle = (inside + outside) / 2.0;
        if shape.contains(add(anchor, scale(direction, middle))) {
            inside = middle;
        } else {
            outside = middle;
        }
    }
    add(anchor, scale(direction, (inside - width / 2.0).max(0.0)))
}

/// Where the stubs cross on their way to the run's start (a crossed end
/// without a crossover, `Lead::Crossing`): how far beyond the pads' stubs'
/// starts, along the run, the start lies at least, in vias with their
/// clearance on both sides and a track: room for the via the router
/// crosses with (pairs6: crossed ends launched 0.65 mm from the pads, no
/// path for their stubs in the negotiation).
pub(super) const CROSSING_ROOM: f64 = 1.0;

/// A launch for a link at the pair point of pads `p` and `n` toward
/// `toward`: from between the pads along the pads' normal away from their
/// part (toward the link's other end where the part does not say), or
/// turned from it by up to a right angle, the first run start where the
/// stubs' ends beside it, the run's straight lead and the envelope `class`
/// at the lead's end are clear; of those the one closest to the pads (a
/// turn costs `ANGLE_COST` a right angle). The stubs themselves are the
/// route's: the pair's nets' ordinary routing from the pads to the ends.
/// With `cross`, a start with a crossover in its lead, which keeps its
/// clearance from the `neighbours`' launches (`neighbour_room`); where it
/// leaves them less than `ROOM` (the pair's gap plus its clearance) the
/// search goes on along the direction, and the room costs (`ROOM_COST`).
#[allow(clippy::too_many_arguments)]
pub(super) fn launch(
    board: &core::Board,
    pair: &Pair,
    class: &core::RuleClass,
    p: &core::Terminal,
    n: &core::Terminal,
    toward: Point,
    want_fixed: bool,
    lead_kind: Lead,
    neighbours: &[Neighbour],
) -> Option<Launch> {
    let search = LaunchSearch::new(
        board,
        pair,
        class,
        (p, n),
        toward,
        (want_fixed, lead_kind),
        neighbours,
    )?;
    // The best launch, and the best whose stubs the link carries
    // (`stubs_clear`), which wins where there is one.
    let mut best: Option<(f64, Launch)> = None;
    let mut best_fixed: Option<(f64, Launch)> = None;
    for angle in LAUNCH_ANGLES {
        search.along(angle, &mut best, &mut best_fixed);
    }
    let best = best_fixed.or(best);
    if debug() {
        eprintln!(
            "diff pair launch {} / {}: {:?}",
            p.label,
            n.label,
            best.as_ref().map(|(_, launch)| launch)
        );
    }
    best.map(|(_, launch)| launch)
}

/// A launch's search at one pair point (`launch`): what every direction
/// and step of it reads.
struct LaunchSearch<'a> {
    board: &'a core::Board,
    pair: &'a Pair,
    class: &'a core::RuleClass,
    p: &'a core::Terminal,
    n: &'a core::Terminal,
    want_fixed: bool,
    lead_kind: Lead,
    /// Whether the start has a crossover in its lead (`Lead::Crossover`).
    cross: bool,
    /// Between the pads, and the pads' normal away from their part.
    middle: Point,
    normal: Point,
    /// The one layer (a bit) the stubs' ends and the run start are on.
    layers: core::LayerMask,
    /// Half the envelope's width, the pair's offset.
    half: f64,
    d: f64,
    /// How far the stubs' ends' copper reaches along the lead, where the
    /// link's copper begins on it, and the lead's length.
    ends_reach: f64,
    reserve: f64,
    lead: f64,
    /// `CROSSING_ROOM` in millimetres.
    crossing_room: f64,
    /// The obstacles near the pads, the steps along a direction, and the
    /// neighbours' launches a crossover here may reach.
    near: Vec<usize>,
    steps: usize,
    neighbours: Vec<&'a Neighbour>,
}

impl<'a> LaunchSearch<'a> {
    /// The search at the pads `p` and `n`, or `None` where they share no
    /// layer of the pair's.
    fn new(
        board: &'a core::Board,
        pair: &'a Pair,
        class: &'a core::RuleClass,
        (p, n): (&'a core::Terminal, &'a core::Terminal),
        toward: Point,
        (want_fixed, lead_kind): (bool, Lead),
        neighbours: &'a [Neighbour],
    ) -> Option<Self> {
        let cross = lead_kind == Lead::Crossover;
        let middle = scale(add(p.anchor, n.anchor), 0.5);
        let mut normal = left(unit(sub(n.anchor, p.anchor)));
        let away = part_middle(board, &p.label, &n.label)
            .map(|part| dot(normal, sub(middle, part)))
            .filter(|along| along.abs() > 0.1);
        if away.unwrap_or_else(|| dot(normal, sub(toward, middle))) < 0.0 {
            normal = scale(normal, -1.0);
        }
        // One layer: the stubs' ends and the run start on it.
        let layers = p.layers & n.layers & pair.layers;
        if layers == 0 {
            return None;
        }
        let layers = layers.isolate_lowest_one();
        let half = class.trace_width / 2.0;
        let d = pair.offset();
        // The stubs' ends' copper reaches a track width along the lead; the
        // link's copper keeps its clearance from it; the lead is at least
        // `LEAD` offsets long.
        let ends_reach = pair.width;
        let apart = pair.width / 2.0 + class.clearance + half;
        let reserve = ends_reach + (apart * apart - d * d).max(0.0).sqrt() + LAUNCH_STEP;
        let lead = (LEAD * d).max(reserve + 2.0 * LAUNCH_STEP);
        let crossing_room = CROSSING_ROOM * (pair.via_diameter + 2.0 * pair.clearance + pair.width);
        let near = obstacles_near(
            board,
            middle,
            LAUNCH_REACH
                + lead
                + half
                + LAUNCH_MARGIN
                + class.clearance.max(board.hole_clearance)
                + 2.0,
        );
        let steps = (LAUNCH_REACH / LAUNCH_STEP) as usize;
        // The neighbours' launches a crossover here may reach.
        let neighbours: Vec<&Neighbour> = neighbours
            .iter()
            .filter(|neighbour| {
                neighbour.copper.iter().any(|&(start, end, radius, _)| {
                    core::geometry::point_segment_distance(middle, start, end)
                        <= LAUNCH_REACH + 2.0 * lead + radius + 2.0
                })
            })
            .collect();
        Some(LaunchSearch {
            board,
            pair,
            class,
            p,
            n,
            want_fixed,
            lead_kind,
            cross,
            middle,
            normal,
            layers,
            half,
            d,
            ends_reach,
            reserve,
            lead,
            crossing_room,
            near,
            steps,
            neighbours,
        })
    }

    /// The search along the normal turned by `angle`: the first clear
    /// start along the direction, then the first whose stubs the link
    /// carries; a crossover that leaves its neighbours less than the room
    /// wanted looks further along. Each better than `best` (and
    /// `best_fixed`, of those the link carries the stubs of) replaces it.
    fn along(
        &self,
        angle: f64,
        best: &mut Option<(f64, Launch)>,
        best_fixed: &mut Option<(f64, Launch)>,
    ) {
        let (pair, want_fixed, cross) = (self.pair, self.want_fixed, self.cross);
        let outward = turned(self.normal, angle);
        let side = if dot(left(outward), sub(self.p.anchor, self.n.anchor)) >= 0.0 {
            1.0
        } else {
            -1.0
        };
        let room_wanted = ROOM * (pair.gap + pair.clearance);
        let cost_of = |found: &Launch| {
            let crowded = if cross {
                ROOM_COST * (1.0 - found.room.min(room_wanted) / room_wanted)
            } else {
                0.0
            };
            core::geometry::distance(self.middle, found.start)
                + ANGLE_COST * angle.abs() / 90.0
                + crowded
        };
        let mut step = 0;
        while step <= self.steps {
            let Some(found) = self.clear_at((outward, side), step) else {
                step += 1;
                continue;
            };
            let cost = cost_of(&found);
            if best.as_ref().is_none_or(|(known, _)| cost < known - 1.0e-9) {
                *best = Some((cost, found.clone()));
            }
            if cross && found.room < room_wanted {
                if want_fixed
                    && found.fixed
                    && best_fixed
                        .as_ref()
                        .is_none_or(|(known, _)| cost < known - 1.0e-9)
                {
                    *best_fixed = Some((cost, found));
                }
                step += 1;
                continue;
            }
            if !want_fixed || best_fixed.as_ref().is_some_and(|(known, _)| *known <= cost) {
                break;
            }
            if found.fixed {
                *best_fixed = Some((cost, found));
                break;
            }
            step += 1;
        }
    }

    /// The launch whose run starts `step` steps out along `outward` (the
    /// positive pad on `side`), if it is clear.
    fn clear_at(&self, (outward, side): (Point, f64), step: usize) -> Option<Launch> {
        let start = add(self.middle, scale(outward, step as f64 * LAUNCH_STEP));
        if self.cross {
            return self.crossed_at((outward, side), start);
        }
        self.straight_at((outward, side), start)
    }

    /// A start without a crossover: the envelope clear at the launch and
    /// along the reserved lead, the stubs' ends' leads clear; the stubs the
    /// link's where they keep every clearance.
    fn straight_at(&self, (outward, side): (Point, f64), start: Point) -> Option<Launch> {
        let (board, pair, near) = (self.board, self.pair, &self.near);
        let (class, layers, half) = (self.class, self.layers, self.half);
        let (lead, reserve) = (self.lead, self.reserve);
        let at = add(start, scale(outward, lead));
        let reserved = add(start, scale(outward, reserve));
        // The envelope at the launch, with room for lattice nodes, and
        // along the reserved lead.
        if clear_layers(
            board,
            near,
            at,
            half + LAUNCH_MARGIN,
            class.clearance,
            layers,
        ) == 0
        {
            return None;
        }
        let samples = ((lead - reserve) / LAUNCH_STEP).ceil() as usize;
        if (0..samples).any(|sample| {
            let point = add(reserved, scale(outward, sample as f64 * LAUNCH_STEP));
            clear_layers(board, near, point, half, class.clearance, layers) == 0
        }) {
            return None;
        }
        let e = pair.ends_offset();
        let ends = [
            add(start, scale(left(outward), side * e)),
            add(start, scale(left(outward), -side * e)),
        ];
        let lead_of = |end: Point| (end, add(end, scale(outward, reserve)));
        if !(end_clear(board, near, pair, lead_of(ends[0]), layers)
            && end_clear(board, near, pair, lead_of(ends[1]), layers))
        {
            return None;
        }
        // Straight stubs the link carries, and the envelope clear along
        // the whole lead (the link's copper from the run's start: no
        // stub of the router's needs to reach the ends).
        let stubs = [
            stub_start(board, self.p, ends[0], pair.width),
            stub_start(board, self.n, ends[1], pair.width),
        ];
        // Stubs that cross keep room for the router's via between the
        // pads and the start.
        if self.lead_kind == Lead::Crossing
            && stubs
                .iter()
                .any(|&stub| dot(sub(start, stub), outward) < self.crossing_room)
        {
            return None;
        }
        let fixed = self.want_fixed
            && stubs_clear(board, near, pair, stubs, ends, layers)
            && (0..=(reserve / LAUNCH_STEP).ceil() as usize).all(|sample| {
                let point = add(start, scale(outward, sample as f64 * LAUNCH_STEP));
                clear_layers(board, near, point, half, class.clearance, layers) != 0
            });
        Some(Launch {
            at,
            start,
            reserved,
            ends: scale(outward, self.ends_reach),
            outward,
            layers,
            side,
            fixed,
            stubs,
            stub_ends: ends,
            crossover: None,
            room: f64::INFINITY,
        })
    }

    /// A start with a crossover in its lead: the stubs' ends an offset
    /// apart where the link carries the stubs (KiCad then sees the tracks
    /// before the crossover a gap apart), else as far apart as the router
    /// keeps the two nets.
    fn crossed_at(&self, (outward, side): (Point, f64), start: Point) -> Option<Launch> {
        let pair = self.pair;
        let (d, e) = (pair.offset(), pair.ends_offset());
        let offsets: &[f64] = if e > d + EPSILON && self.want_fixed {
            &[d, e]
        } else {
            &[e]
        };
        offsets.iter().find_map(|&offset| {
            let launch = self.crossed_with((outward, side), start, offset)?;
            (launch.fixed || offset >= e).then_some(launch)
        })
    }

    /// A start with a crossover in its lead, the stubs' ends `e` from its
    /// centreline: the stubs' ends on the pads' sides, the crossover's
    /// copper clear, the run's lead past it (beside the straight net) and
    /// the envelope at its end clear.
    fn crossed_with(&self, (outward, side): (Point, f64), start: Point, e: f64) -> Option<Launch> {
        let (board, pair, near) = (self.board, self.pair, &self.near);
        let (class, layers, half) = (self.class, self.layers, self.half);
        let ends = [
            add(start, scale(left(outward), side * e)),
            add(start, scale(left(outward), -side * e)),
        ];
        let found = crossover(board, near, pair, (start, outward, side), (ends, e), layers)?;
        let run_start = found.run_start;
        let lead = LEAD * self.d;
        let at = add(run_start, scale(outward, lead));
        // Past the crossover the run's centreline lies beside the
        // straight net, off the pads' middle: its lead keeps the room
        // too.
        let room = neighbour_room(
            &found,
            (run_start, at, half),
            layers,
            pair,
            &self.neighbours,
        );
        if room < 0.0 {
            return None;
        }
        if clear_layers(
            board,
            near,
            at,
            half + LAUNCH_MARGIN,
            class.clearance,
            layers,
        ) == 0
        {
            return None;
        }
        let samples = (lead / LAUNCH_STEP).ceil() as usize;
        if (0..samples).any(|sample| {
            let point = add(run_start, scale(outward, sample as f64 * LAUNCH_STEP));
            clear_layers(board, near, point, half, class.clearance, layers) == 0
        }) {
            return None;
        }
        let stubs = [
            stub_start(board, self.p, ends[0], pair.width),
            stub_start(board, self.n, ends[1], pair.width),
        ];
        let fixed = self.want_fixed
            && stubs_clear(board, near, pair, stubs, ends, layers)
            && apart_from_crossover(pair, &found, (stubs, ends), layers, 0)
            && apart_from_crossover(pair, &found, (stubs, ends), layers, 1);
        Some(Launch {
            at,
            start,
            reserved: run_start,
            ends: scale(outward, self.ends_reach),
            outward,
            layers,
            side,
            fixed,
            stubs,
            stub_ends: ends,
            crossover: Some(found),
            room,
        })
    }
}

/// Whether the straight stub of `polarity` (from `stubs` to `ends`) keeps
/// its clearance from the other net's crossover copper (`found`) on
/// `layers` too.
fn apart_from_crossover(
    pair: &Pair,
    found: &Crossover,
    (stubs, ends): ([Point; 2], [Point; 2]),
    layers: core::LayerMask,
    polarity: usize,
) -> bool {
    let (pad, end) = (stubs[polarity], ends[polarity]);
    let other = 1 - polarity;
    found.segments[other]
        .iter()
        .filter(|(layer, ..)| 1 << layer == layers)
        .all(|&(_, from, to)| {
            core::geometry::segment_segment_distance(pad, end, from, to)
                >= pair.width + pair.gap.min(pair.clearance) - EPSILON
        })
        && found.vias[other].iter().all(|&via| {
            core::geometry::point_segment_distance(via, pad, end)
                >= pair.width / 2.0
                    + (pair.via_diameter / 2.0 + pair.clearance)
                        .max(pair.via_drill / 2.0 + pair.hole_clearance)
        })
}
