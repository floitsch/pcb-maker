// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The move descent (docs/experiments/move-descent.md): after the first
//! route, single parts are moved a little at a time where a move helps,
//! with trials cheap enough to run on boards far from complete.
//!
//! - **Candidates**: the parts with unconnected terminals of their own,
//!   then those whose nets are open, then those whose nets cross the
//!   fullest boundaries of the global route (the overflow attribution).
//! - **Moves**: nudges of `steps_mm` in eight directions, quarter turns in
//!   place, swaps with a part of the same size nearby; legal by the
//!   placer's checks at the placement's relaxation.
//! - **Tier one**: every legal move is scored by the incremental global
//!   route (`pcb_global::Incremental`): the capacities around the part's
//!   old and new place measured again and its nets routed again on the
//!   tiles against the rest's usage; the predicted open nets, the overflow
//!   and the soft overflow against the current placement's.
//! - **Tier two**: the best `confirm` moves that tier one does not find
//!   worse are lowered and tried by the router: `Router::update_nets`
//!   rips up what the move invalidated and `Router::local_reroute` routes
//!   those nets again against the rest as walls. A move that leaves fewer
//!   terminals open is kept (the router's state with it) and the parts
//!   are ranked again.
//!
//! The descent spends at most its budget of work; the layout's later
//! steps (the moves, the final ladder) route the placement it leaves.

use super::*;
use crate::board_layout::{footprint_items, without_copper_texts};
use crate::board_placer::{LoweredPlacement, write_footprint_pose};
use crate::board_router::lower;
use pcb_placer as placer;
use pcb_router as core;
use pcb_router::geometry::{Aabb, Point, Shape};
use std::collections::HashMap;

/// The move descent's settings (`move_descent` in layout.json; absent:
/// off). `PCB_LAYOUT_MOVE_DESCENT` overrides: `0` off, anything else on
/// with these defaults where layout.json has none.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadMoveDescent {
    /// Share of the moves' budget (`move_seconds`, within 65 % of
    /// `total_seconds`) the descent may spend, in work.
    pub trial_share: f64,
    /// Nudge distances in millimetres, each in eight directions.
    pub steps_mm: Vec<f64>,
    /// Moves per part the router tries (the best by the global route).
    pub confirm: usize,
    /// Search work of one router trial at most, in seconds.
    pub local_seconds: f64,
    /// Swaps with a part of the same size whose centre is at most this
    /// far, in millimetres (0: no swaps).
    pub swap_mm: f64,
    /// The descent stops after this many parts in a row without a kept
    /// move.
    pub patience: usize,
    /// After a kept move, the parts within this many millimetres of the
    /// moved parts (and those sharing a net of up to eight pads with
    /// them) may be tried again.
    pub retry_mm: f64,
}

impl Default for KiCadMoveDescent {
    fn default() -> Self {
        Self {
            trial_share: 0.5,
            steps_mm: vec![0.5, 1.0, 2.0, 3.0],
            confirm: 4,
            local_seconds: 2.0,
            swap_mm: 10.0,
            patience: 60,
            retry_mm: 5.0,
        }
    }
}

/// The descent's settings for a layout: layout.json's, or the override's.
pub(crate) fn descent_config(configured: Option<&KiCadMoveDescent>) -> Option<KiCadMoveDescent> {
    match std::env::var("PCB_LAYOUT_MOVE_DESCENT") {
        Ok(value) if value == "0" => None,
        Ok(_) => Some(configured.cloned().unwrap_or_default()),
        Err(_) => configured.cloned(),
    }
}

/// A move the descent kept.
#[derive(Clone, Debug, Serialize)]
pub struct KiCadDescentMove {
    /// `nudge`, `turn` or `swap`.
    pub kind: String,
    /// The parts moved (the first is the candidate).
    pub references: Vec<String>,
    /// Open terminals (the router's count) before and after.
    pub open_before: usize,
    pub open_after: usize,
    /// The global route's cost change (tier one; negative is better).
    pub predicted: f64,
}

/// What the descent did (`move_descent` in board-layout.json and the
/// verdict's `moves`).
#[derive(Clone, Debug, Default, Serialize)]
pub struct KiCadMoveDescentReport {
    /// Parts tried, moves scored by the global route (tier one), moves
    /// tried by the router (tier two), moves kept.
    pub parts_tried: usize,
    pub trials: usize,
    pub confirmed: usize,
    pub kept: usize,
    /// The parts a kept move moved.
    pub parts_moved: Vec<String>,
    /// Open terminals (the layout's score of the routes) before and after.
    pub open_before: usize,
    pub open_after: usize,
    /// Work spent (seconds of an idle machine) of the budget, and per
    /// tier; the wall clock per tier.
    pub budget_seconds: f64,
    pub work_seconds: f64,
    pub global_work_seconds: f64,
    pub tier_one_work_seconds: f64,
    pub tier_two_work_seconds: f64,
    pub global_wall_seconds: f64,
    pub tier_one_wall_seconds: f64,
    pub tier_two_wall_seconds: f64,
    /// Why it stopped: `budget`, `patience`, `candidates`, `deadline`.
    pub stopped: String,
    /// The calibration: the open terminals after a router trial of no
    /// move at all (the update and the local reroute on the board as it
    /// is), against `open_before`.
    pub null_open: usize,
    pub moves: Vec<KiCadDescentMove>,
}

/// The verdict's `moves`: the descent in brief.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct KiCadMovesFact {
    /// Moves scored by the global route and moves the router tried.
    pub trials: usize,
    pub confirmed: usize,
    pub kept: usize,
    pub parts_moved: Vec<String>,
    pub open_before: usize,
    pub open_after: usize,
}

impl KiCadMoveDescentReport {
    pub fn fact(&self) -> KiCadMovesFact {
        KiCadMovesFact {
            trials: self.trials,
            confirmed: self.confirmed,
            kept: self.kept,
            parts_moved: self.parts_moved.clone(),
            open_before: self.open_before,
            open_after: self.open_after,
        }
    }
}

/// Placement work per unit of the global route's own counts, as the
/// placer's guidance counts it (docs/global-route.md, "Phase 2").
const WORK_PER_OBSTACLE: f64 = 3_000.0;
const WORK_PER_CAPACITY_TEST: f64 = 5.0;
const WORK_PER_POP: f64 = 30.0;

fn global_work(capacity_tests: u64, pops: u64) -> f64 {
    (WORK_PER_CAPACITY_TEST * capacity_tests as f64 + WORK_PER_POP * pops as f64)
        / placer::work_per_second()
}

/// The layout's state the descent works on.
pub(crate) struct DescentState<'a> {
    pub pcb: &'a mut Expr,
    pub board: &'a mut core::Board,
    pub problem: &'a mut LoweredPlacement,
    pub router: &'a mut core::router::Router,
    pub work: &'a mut f64,
}

/// What a part owns on the lowered board: its obstacles and its
/// terminals (net, terminal).
#[derive(Clone, Debug, Default)]
struct PartCopper {
    obstacles: Vec<usize>,
    terminals: Vec<(usize, usize)>,
}

/// The part a label names (`REF.PAD...`, `REF copper ...`).
fn owner(label: &str, index: &HashMap<String, usize>) -> Option<usize> {
    label
        .match_indices(['.', ' '])
        .map(|(at, _)| &label[..at])
        .find_map(|reference| index.get(reference).copied())
}

fn part_copper(
    board: &core::Board,
    index: &HashMap<String, usize>,
    parts: usize,
) -> Vec<PartCopper> {
    let mut copper = vec![PartCopper::default(); parts];
    for (obstacle, item) in board.obstacles.iter().enumerate() {
        if let Some(part) = owner(&item.label, index) {
            copper[part].obstacles.push(obstacle);
        }
    }
    for (net, item) in board.nets.iter().enumerate() {
        for (terminal, record) in item.terminals.iter().enumerate() {
            if let Some(part) = owner(&record.label, index) {
                copper[part].terminals.push((net, terminal));
                if !copper[part].obstacles.contains(&record.pad) {
                    copper[part].obstacles.push(record.pad);
                }
            }
        }
    }
    copper
}

/// The rigid motion taking a part's copper from `from` to `to`.
fn motion(from: placer::Pose, to: placer::Pose) -> impl Fn(Point) -> Point {
    move |point: Point| {
        let local = placer::problem::rotate(
            [point[0] - from.position[0], point[1] - from.position[1]],
            from.angle,
        );
        let world = placer::problem::rotate(local, -to.angle);
        [to.position[0] + world[0], to.position[1] + world[1]]
    }
}

fn map_shape(shape: &Shape, map: &dyn Fn(Point) -> Point) -> Shape {
    match shape {
        Shape::Circle { center, radius } => Shape::Circle {
            center: map(*center),
            radius: *radius,
        },
        Shape::Capsule { start, end, radius } => Shape::Capsule {
            start: map(*start),
            end: map(*end),
            radius: *radius,
        },
        Shape::Polygon { points } => Shape::Polygon {
            points: points.iter().map(|point| map(*point)).collect(),
        },
        Shape::Union { parts } => Shape::Union {
            parts: parts.iter().map(|part| map_shape(part, map)).collect(),
        },
    }
}

fn union(a: Option<Aabb>, b: Aabb) -> Aabb {
    match a {
        None => b,
        Some(a) => Aabb {
            minimum: [
                a.minimum[0].min(b.minimum[0]),
                a.minimum[1].min(b.minimum[1]),
            ],
            maximum: [
                a.maximum[0].max(b.maximum[0]),
                a.maximum[1].max(b.maximum[1]),
            ],
        },
    }
}

/// A candidate move: the parts' new poses (followers included).
#[derive(Clone, Debug)]
struct Move {
    kind: &'static str,
    poses: Vec<placer::Pose>,
    moved: Vec<usize>,
    distance: f64,
}

/// Applies `moved` parts' motions to `board` in place (obstacles and
/// terminal anchors); returns what to restore, the area touched before
/// and after, and the nets with a terminal on them.
#[allow(clippy::type_complexity)]
fn apply(
    board: &mut core::Board,
    copper: &[PartCopper],
    old: &[placer::Pose],
    new: &[placer::Pose],
    moved: &[usize],
) -> (
    Vec<(usize, Shape)>,
    Vec<(usize, usize, Point)>,
    Aabb,
    Vec<usize>,
) {
    let mut shapes = Vec::new();
    let mut anchors = Vec::new();
    let mut area = None;
    let mut nets = Vec::new();
    for &part in moved {
        let map = motion(old[part], new[part]);
        for &obstacle in &copper[part].obstacles {
            let before = board.obstacles[obstacle].shape.clone();
            let after = map_shape(&before, &map);
            area = Some(union(area, before.aabb()));
            area = Some(union(area, after.aabb()));
            board.obstacles[obstacle].shape = after;
            shapes.push((obstacle, before));
        }
        for &(net, terminal) in &copper[part].terminals {
            let anchor = board.nets[net].terminals[terminal].anchor;
            board.nets[net].terminals[terminal].anchor = map(anchor);
            anchors.push((net, terminal, anchor));
            nets.push(net);
        }
    }
    nets.sort_unstable();
    nets.dedup();
    let area = area.unwrap_or(Aabb {
        minimum: [0.0, 0.0],
        maximum: [0.0, 0.0],
    });
    (shapes, anchors, area.inflated(1.0), nets)
}

fn restore(
    board: &mut core::Board,
    shapes: Vec<(usize, Shape)>,
    anchors: Vec<(usize, usize, Point)>,
) {
    for (obstacle, shape) in shapes {
        board.obstacles[obstacle].shape = shape;
    }
    for (net, terminal, anchor) in anchors {
        board.nets[net].terminals[terminal].anchor = anchor;
    }
}

/// Whether the poses (the parts in `moved` changed) are legal and keep
/// the near and relative constraints at least as well.
fn legal(problem: &placer::Problem, poses: &[placer::Pose], moved: &[usize]) -> bool {
    for &part in moved {
        if !placer::legal::is_legal(
            problem,
            poses,
            part,
            poses[part],
            (0..problem.components.len()).filter(|other| *other != part),
        ) {
            return false;
        }
    }
    if !problem.constraints.relations.is_empty() {
        let before = placer::constraints::relation_penalty(problem, &problem.poses, None);
        if placer::constraints::relation_penalty(problem, poses, None) > before + 1.0e-9 {
            return false;
        }
    }
    true
}

/// The legal moves of part `index`.
fn moves_of(problem: &placer::Problem, index: usize, descent: &KiCadMoveDescent) -> Vec<Move> {
    let component = &problem.components[index];
    let current = problem.poses[index];
    let center = component.center(current);
    // (kind, the parts' new poses, the distance moved)
    type Wanted = (&'static str, Vec<(usize, placer::Pose)>, f64);
    let mut wanted: Vec<Wanted> = Vec::new();
    let snap = |part: usize, center: Point, angle: f64| {
        let component = &problem.components[part];
        let position = component.position_for_center(center, angle);
        placer::Pose {
            position: placer::constraints::snap_position(problem, part, position, angle),
            angle,
        }
    };
    let diagonal = std::f64::consts::FRAC_1_SQRT_2;
    for step in descent.steps_mm.iter().copied() {
        for (dx, dy) in [
            (1.0, 0.0),
            (-1.0, 0.0),
            (0.0, 1.0),
            (0.0, -1.0),
            (diagonal, diagonal),
            (diagonal, -diagonal),
            (-diagonal, diagonal),
            (-diagonal, -diagonal),
        ] {
            let pose = snap(
                index,
                [center[0] + step * dx, center[1] + step * dy],
                current.angle,
            );
            wanted.push(("nudge", vec![(index, pose)], step));
        }
    }
    for angle in component.angle_options.iter().copied() {
        if (angle - current.angle).abs() < 1.0e-6 {
            continue;
        }
        wanted.push(("turn", vec![(index, snap(index, center, angle))], 0.0));
    }
    if descent.swap_mm > 0.0 {
        let extent = component.half_extent(current.angle);
        for (other, record) in problem.components.iter().enumerate() {
            if other == index
                || record.fixed
                || record.pins.is_empty()
                || record.side != component.side
            {
                continue;
            }
            let other_pose = problem.poses[other];
            let other_center = record.center(other_pose);
            let distance = ((other_center[0] - center[0]).powi(2)
                + (other_center[1] - center[1]).powi(2))
            .sqrt();
            if distance > descent.swap_mm || distance < 1.0e-6 {
                continue;
            }
            let other_extent = record.half_extent(other_pose.angle);
            if (extent[0] - other_extent[0]).abs() > 0.05
                || (extent[1] - other_extent[1]).abs() > 0.05
            {
                continue;
            }
            wanted.push((
                "swap",
                vec![
                    (index, snap(index, other_center, current.angle)),
                    (other, snap(other, center, other_pose.angle)),
                ],
                distance,
            ));
        }
    }
    let mut moves = Vec::new();
    for (kind, changes, distance) in wanted {
        if changes
            .iter()
            .all(|(part, pose)| problem.poses[*part] == *pose)
        {
            continue;
        }
        let mut poses = problem.poses.clone();
        for (part, pose) in &changes {
            poses[*part] = *pose;
        }
        problem.sync_followers(&mut poses);
        let moved: Vec<usize> = (0..poses.len())
            .filter(|part| poses[*part] != problem.poses[*part])
            .collect();
        if moved.is_empty() || !legal(problem, &poses, &moved) {
            continue;
        }
        moves.push(Move {
            kind,
            poses,
            moved,
            distance,
        });
    }
    moves
}

/// Debugging aid: `PCB_LAYOUT_DESCENT_CLASSIC=<n>`: the first `n` router
/// trials are also tried as the coupled loop tries a move (a clone, the
/// update, a reroute without the polish) and both are timed.
fn classic_trials() -> usize {
    std::env::var("PCB_LAYOUT_DESCENT_CLASSIC")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

/// Runs the descent on `state` with `budget` seconds of work. `arm` sets
/// a router's work deadline from the layout's work; `allowed` says
/// whether the wall clock lets another trial start.
#[allow(clippy::too_many_arguments)]
pub(crate) fn descend(
    state: DescentState<'_>,
    descent: &KiCadMoveDescent,
    budget: f64,
    router_config: &KiCadBoardRouterConfig,
    connect: bool,
    keep_board_marks: bool,
    arm: &dyn Fn(&mut core::router::Router, f64),
    allowed: &dyn Fn() -> bool,
) -> Result<KiCadMoveDescentReport, String> {
    let DescentState {
        pcb,
        board,
        problem,
        router,
        work,
    } = state;
    let started_work = *work;
    let mut report = KiCadMoveDescentReport {
        budget_seconds: budget,
        ..KiCadMoveDescentReport::default()
    };
    let parts = problem.problem.components.len();
    let index = crate::board_placer::reference_index(&problem.references);
    let global_started = std::time::Instant::now();
    let global_config = pcb_global::GlobalConfig::default();
    let mut global = pcb_global::Incremental::new(board, &global_config);
    let global_work_first = WORK_PER_OBSTACLE * board.obstacles.len() as f64
        / placer::work_per_second()
        + global_work(global.capacity_tests, global.pops);
    *work += global_work_first;
    report.global_work_seconds = global_work_first;
    report.global_wall_seconds = global_started.elapsed().as_secs_f64();
    let mut copper = part_copper(board, &index, parts);
    // Nets of many pads (supplies) do not make their parts neighbours.
    let big: Vec<bool> = board
        .nets
        .iter()
        .map(|net| net.terminals.len() > 8)
        .collect();
    let mut open = router.board_open_terminals();
    report.open_before = open;
    // The calibration: a trial of no move.
    {
        let mut trial = router.clone();
        arm(&mut trial, *work);
        let before = trial.expansions();
        let ripped = trial.update_nets(board)?;
        report.null_open = trial.local_reroute(&ripped, descent.local_seconds);
        let cost = trial.update_work()
            + trial.expansions().saturating_sub(before) as f64
                / core::router::expansions_per_second();
        *work += cost;
        report.tier_two_work_seconds += cost;
        eprintln!(
            "move descent: a trial of no move leaves {} open ({} nets routed again, {cost:.2} s of work)",
            report.null_open,
            ripped.len()
        );
    }
    let classic = classic_trials();
    let mut classic_done = 0;
    let mut tried: Vec<bool> = vec![false; parts];
    let mut since_kept = 0;
    let mut working = board.clone();
    eprintln!(
        "move descent: {open} open, budget {budget:.0} s of work; global route {:.2} s ({:.2} s of work), overflowed nets {}, soft overflow {:.0}",
        report.global_wall_seconds,
        global_work_first,
        global.measure().overflowed_nets,
        global.measure().soft
    );
    'parts: loop {
        if *work - started_work > budget {
            report.stopped = "budget".into();
            break;
        }
        if !allowed() || router.past_deadline() {
            report.stopped = "deadline".into();
            break;
        }
        if since_kept >= descent.patience {
            report.stopped = "patience".into();
            break;
        }
        // The candidates: own open terminals, their nets' open terminals,
        // the overflow attribution.
        let mut own = vec![0usize; parts];
        let mut nets_open = vec![0usize; parts];
        let mut open_per_net: HashMap<usize, usize> = HashMap::new();
        for (net, terminal) in router.unconnected() {
            *open_per_net.entry(net as usize).or_default() += 1;
            if let Some(part) = owner(&board.nets[net as usize].terminals[terminal].label, &index) {
                own[part] += 1;
            }
        }
        for (part, record) in copper.iter().enumerate() {
            let mut nets: Vec<usize> = record.terminals.iter().map(|(net, _)| *net).collect();
            nets.sort_unstable();
            nets.dedup();
            nets_open[part] = nets
                .iter()
                .map(|net| open_per_net.get(net).copied().unwrap_or(0))
                .sum();
        }
        let (attribution, _) =
            crate::board_placer::attribution(board, &global.route, &index, parts, 0.5);
        let mut ranked: Vec<(usize, usize, f64, usize)> = (0..parts)
            .filter(|part| {
                let component = &problem.problem.components[*part];
                !component.fixed && !component.pins.is_empty() && !tried[*part]
            })
            .map(|part| (own[part], nets_open[part], attribution[part], part))
            .filter(|(own, nets, attributed, _)| *own > 0 || *nets > 0 || *attributed > 0.0)
            .collect();
        ranked.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then(b.1.cmp(&a.1))
                .then(b.2.total_cmp(&a.2))
                .then(a.3.cmp(&b.3))
        });
        let Some(&(_, _, _, part)) = ranked.first() else {
            report.stopped = "candidates".into();
            break;
        };
        tried[part] = true;
        report.parts_tried += 1;
        since_kept += 1;

        // Tier one: every legal move on the global route.
        let tier_one_started = std::time::Instant::now();
        let base = global.measure().cost();
        let mut scored: Vec<(f64, f64, Move)> = Vec::new();
        for candidate in moves_of(&problem.problem, part, descent) {
            let (shapes, anchors, area, nets) = apply(
                &mut working,
                &copper,
                &problem.problem.poses,
                &candidate.poses,
                &candidate.moved,
            );
            let after = global.moved(&working, area, &nets);
            restore(&mut working, shapes, anchors);
            let tests = after.capacity_tests - global.capacity_tests;
            let pops = after.pops - global.pops;
            let cost = global_work(tests, pops);
            *work += cost;
            report.tier_one_work_seconds += cost;
            report.trials += 1;
            scored.push((after.measure().cost() - base, candidate.distance, candidate));
        }
        report.tier_one_wall_seconds += tier_one_started.elapsed().as_secs_f64();
        scored.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        scored.retain(|(delta, _, _)| *delta <= 1.0e-9);
        scored.truncate(descent.confirm);

        // Tier two: the router tries the best.
        for (predicted, _, candidate) in scored {
            if *work - started_work > budget {
                report.stopped = "budget".into();
                break 'parts;
            }
            if !allowed() || router.past_deadline() {
                report.stopped = "deadline".into();
                break 'parts;
            }
            let tier_two_started = std::time::Instant::now();
            let mut trial_pcb = pcb.clone();
            {
                let mut footprints = footprint_items(&mut trial_pcb)?;
                for &moved in &candidate.moved {
                    write_footprint_pose(footprints[moved], candidate.poses[moved])?;
                }
            }
            let lowered_started = std::time::Instant::now();
            let trial_board = lower(
                &without_copper_texts(&trial_pcb, keep_board_marks),
                router_config,
                connect,
            )?
            .board;
            let lowered = lowered_started.elapsed().as_secs_f64();
            let lower_work =
                WORK_PER_OBSTACLE * trial_board.obstacles.len() as f64 / placer::work_per_second();
            if classic_done < classic {
                classic_done += 1;
                let clone_started = std::time::Instant::now();
                let mut copy = router.clone();
                let cloned = clone_started.elapsed().as_secs_f64();
                arm(&mut copy, *work);
                let update_started = std::time::Instant::now();
                let before = copy.expansions();
                let ripped = copy.update(&trial_board)?;
                let updated = update_started.elapsed().as_secs_f64();
                let reroute_started = std::time::Instant::now();
                let rerouted = copy.reroute(false);
                let search = copy.expansions().saturating_sub(before) as f64
                    / core::router::expansions_per_second();
                eprintln!(
                    "move descent: classic trial of {}: lowering {lowered:.2} s ({lower_work:.2} s of work), clone {cloned:.2} s, update {updated:.2} s ({:.2} s of build work, {ripped} nets ripped up), reroute {:.2} s ({search:.2} s of search work), open {} (before {open})",
                    problem.references[part],
                    copy.update_work(),
                    reroute_started.elapsed().as_secs_f64(),
                    score_open(&rerouted, &trial_board),
                );
            }
            let clone_started = std::time::Instant::now();
            let mut trial = router.clone();
            let cloned = clone_started.elapsed().as_secs_f64();
            arm(&mut trial, *work);
            let update_started = std::time::Instant::now();
            let before = trial.expansions();
            let ripped = trial.update_nets(&trial_board)?;
            let updated = update_started.elapsed().as_secs_f64();
            let update_work = trial.update_work();
            let local_started = std::time::Instant::now();
            let after = trial.local_reroute(&ripped, descent.local_seconds);
            let local = local_started.elapsed().as_secs_f64();
            let search = trial.expansions().saturating_sub(before) as f64
                / core::router::expansions_per_second();
            let cost = lower_work + update_work + search;
            *work += cost;
            report.tier_two_work_seconds += cost;
            report.confirmed += 1;
            report.tier_two_wall_seconds += tier_two_started.elapsed().as_secs_f64();
            let kept = after < open;
            eprintln!(
                "move descent: {} {} ({}), predicted {predicted:+.1}: {after} open (before {open}), {} nets ripped up; lowering {lowered:.2} s, clone {cloned:.2} s, update {updated:.2} s, reroute {local:.2} s; work {cost:.2} s (update {update_work:.2}, search {search:.2}){}",
                candidate.kind,
                candidate
                    .moved
                    .iter()
                    .map(|part| problem.references[*part].as_str())
                    .collect::<Vec<_>>()
                    .join("+"),
                problem.references[part],
                ripped.len(),
                if kept { ", kept" } else { "" }
            );
            if !kept {
                continue;
            }
            // Kept: the router's state, the board, the poses, the global
            // route on the board as lowered.
            let (_, _, area, nets) = apply(
                &mut working,
                &copper,
                &problem.problem.poses,
                &candidate.poses,
                &candidate.moved,
            );
            let next = global.moved(&trial_board, area, &nets);
            let cost = global_work(
                next.capacity_tests - global.capacity_tests,
                next.pops - global.pops,
            );
            *work += cost;
            report.tier_one_work_seconds += cost;
            global = next;
            *router = trial;
            *pcb = trial_pcb;
            *board = trial_board;
            working = board.clone();
            let previous = std::mem::replace(&mut problem.problem.poses, candidate.poses.clone());
            copper = part_copper(board, &index, parts);
            report.moves.push(KiCadDescentMove {
                kind: candidate.kind.into(),
                references: candidate
                    .moved
                    .iter()
                    .map(|part| problem.references[*part].clone())
                    .collect(),
                open_before: open,
                open_after: after,
                predicted,
            });
            for &moved in &candidate.moved {
                let reference = &problem.references[moved];
                if !report.parts_moved.contains(reference) {
                    report.parts_moved.push(reference.clone());
                }
            }
            open = after;
            report.kept += 1;
            since_kept = 0;
            // Ranked again: the parts the move may have helped (those
            // sharing a net with the moved ones, or near their old or new
            // place) may be tried again.
            let moved_nets: Vec<usize> = candidate
                .moved
                .iter()
                .flat_map(|part| copper[*part].terminals.iter().map(|(net, _)| *net))
                .collect();
            let places: Vec<Point> = candidate
                .moved
                .iter()
                .flat_map(|part| {
                    let component = &problem.problem.components[*part];
                    [
                        component.center(previous[*part]),
                        component.center(problem.problem.poses[*part]),
                    ]
                })
                .collect();
            for (other, flag) in tried.iter_mut().enumerate() {
                let center = problem.problem.components[other].center(problem.problem.poses[other]);
                let near = places.iter().any(|place| {
                    (place[0] - center[0]).abs() <= descent.retry_mm
                        && (place[1] - center[1]).abs() <= descent.retry_mm
                });
                if near
                    || copper[other]
                        .terminals
                        .iter()
                        .any(|(net, _)| moved_nets.contains(net) && !big[*net])
                {
                    *flag = false;
                }
            }
            continue 'parts;
        }
    }
    report.open_after = open;
    report.work_seconds = *work - started_work;
    eprintln!(
        "move descent: {} parts, {} trials on the global route ({:.1} s, {:.1} s of work), {} by the router ({:.1} s, {:.1} s of work), {} kept; open {} -> {}; stopped: {}",
        report.parts_tried,
        report.trials,
        report.tier_one_wall_seconds,
        report.tier_one_work_seconds,
        report.confirmed,
        report.tier_two_wall_seconds,
        report.tier_two_work_seconds,
        report.kept,
        report.open_before,
        report.open_after,
        report.stopped
    );
    Ok(report)
}

/// The layout's open count of a result (as `score`).
fn score_open(result: &core::RoutingResult, board: &core::Board) -> usize {
    result
        .status
        .iter()
        .map(|status| match status {
            core::NetStatus::Partial {
                unconnected_terminals,
            } => *unconnected_terminals,
            core::NetStatus::Unreachable => 1,
            _ => 0,
        })
        .sum::<usize>()
        + core::verify(board, &result.routes).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_motion_moves_and_turns_the_copper_with_the_part() {
        let from = placer::Pose {
            position: [10.0, 10.0],
            angle: 0.0,
        };
        let to = placer::Pose {
            position: [12.0, 10.0],
            angle: 90.0,
        };
        let map = motion(from, to);
        // A pad 1 mm right of the origin: KiCad's 90 degrees turn it to 1
        // mm above (y down), at the new origin.
        let moved = map([11.0, 10.0]);
        let expected = placer::problem::rotate([1.0, 0.0], -90.0);
        assert!((moved[0] - (12.0 + expected[0])).abs() < 1.0e-9);
        assert!((moved[1] - (10.0 + expected[1])).abs() < 1.0e-9);
        // The identity leaves points alone.
        let same = motion(from, from)([3.0, 4.0]);
        assert!((same[0] - 3.0).abs() < 1.0e-12 && (same[1] - 4.0).abs() < 1.0e-12);
    }

    #[test]
    fn labels_name_their_part() {
        let index: HashMap<String, usize> = [("R1".to_string(), 0), ("R10".to_string(), 1)]
            .into_iter()
            .collect();
        assert_eq!(owner("R1.2", &index), Some(0));
        assert_eq!(owner("R10.1 padstack", &index), Some(1));
        assert_eq!(owner("R1 copper line", &index), Some(0));
        assert_eq!(owner("copper line", &index), None);
    }
}
