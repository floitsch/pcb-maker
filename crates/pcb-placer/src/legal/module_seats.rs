// Copyright (C) 2026 Florian Loitsch.

//! Seat explicitly constrained module pairs before ordinary single-part legalization.

use super::*;
use crate::constraints::{ModuleEnvelope, ModuleEnvelopePair};

/// Leave a legal pair in the initial occupied set. The ordinary refinement
/// can still move either module with every placement constraint in effect.
pub(crate) fn seat_pairs(
    problem: &Problem,
    poses: &mut [Pose],
    placed: &mut Vec<usize>,
    stop: Option<u64>,
    maximum_seconds: f64,
) {
    let cap = (maximum_seconds * crate::work_per_second()) as u64;
    let stop = stop
        .unwrap_or(u64::MAX)
        .min(crate::work().saturating_add(cap));
    for pair in &problem.constraints.module_envelopes {
        let [a, b] = pair.modules.each_ref().map(|module| module.part);
        if pair.modules[0].back != pair.modules[1].back
            || (placed.contains(&a) && placed.contains(&b))
        {
            continue;
        }
        let out_of_work = || crate::work() >= stop || crate::out_of_work(0);
        if out_of_work() {
            break;
        }
        let mut a_seats = seats(problem, poses, a, b, placed, out_of_work);
        let mut b_seats = seats(problem, poses, b, a, placed, out_of_work);
        if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
            eprintln!(
                "placer: mounted-module candidate seats for {} and {}: {} and {} (budget reached: {})",
                problem.components[a].name,
                problem.components[b].name,
                a_seats.len(),
                b_seats.len(),
                out_of_work(),
            );
        }
        if a_seats.is_empty() || b_seats.is_empty() {
            continue;
        }
        a_seats.sort_by(|a, b| a.0.total_cmp(&b.0));
        b_seats.sort_by(|a, b| a.0.total_cmp(&b.0));
        if let Some([first, second]) =
            choose_pair(problem, poses, pair, [&a_seats, &b_seats], out_of_work)
        {
            for (part, pose) in [(a, first), (b, second)] {
                poses[part] = pose;
                if !placed.contains(&part) {
                    placed.push(part);
                }
            }
        }
    }
}

fn seats(
    problem: &Problem,
    poses: &[Pose],
    part: usize,
    partner: usize,
    placed: &[usize],
    out_of_work: impl Fn() -> bool,
) -> Vec<(f64, Pose)> {
    let wanted = poses[part];
    if placed.contains(&part) {
        return vec![(0.0, wanted)];
    }
    let legal = |pose| {
        is_legal(
            problem,
            poses,
            part,
            pose,
            placed.iter().copied().filter(|&other| other != partner),
        )
    };
    let component = &problem.components[part];
    let wanted_center = component.center(wanted);
    let [left, top, right, bottom] = problem.bounds();
    let grid = problem.grid.max(1.0e-5);
    let held_edge = problem
        .constraints
        .edges
        .iter()
        .any(|(index, ..)| *index == part);
    let mut seats = Vec::new();
    for &angle in &component.angle_options {
        let [x, y] = component.half_extent(angle);
        let margin = if problem.constraints.pad_clearance {
            0.0
        } else {
            component.edge_margin(problem.edge_margin)
        };
        let board = [
            left + x + margin,
            top + y + margin,
            right - x - margin,
            bottom - y - margin,
        ];
        let [left, top, right, bottom] = region_window(problem, part, angle).unwrap_or(board);
        let x_spots = edge_spots(left, right, wanted_center[0], grid);
        let y_spots = edge_spots(top, bottom, wanted_center[1], grid);
        let fine_x = across_span(left, right, grid);
        let fine_y = across_span(top, bottom, grid);
        let edge_x = near_ends(left, right, grid);
        let edge_y = near_ends(top, bottom, grid);
        // Fixed connectors can leave a narrow seat near an end of the
        // board. Sample the long direction more closely in these bands.
        let mut spots = Vec::new();
        spots.extend(across(&x_spots, &y_spots, false));
        spots.extend(across(&fine_x, &edge_y, false));
        spots.extend(across(&fine_y, &edge_x, true));
        let mut seen = std::collections::BTreeSet::new();
        for center in spots {
            if out_of_work() {
                return seats;
            }
            crate::add_work(super::SPOT_WORK as u64);
            let center = if held_edge {
                constraints::flush_center(problem, part, center, angle)
            } else {
                center
            };
            let position = component.position_for_center(center, angle);
            let pose = Pose {
                position: constraints::snap_position(problem, part, position, angle),
                angle,
            };
            if seen.insert(pose.position.map(f64::to_bits)) && legal(pose) {
                let center = component.center(pose);
                let turn = if angle == wanted.angle {
                    0.0
                } else {
                    grid * grid
                };
                let distance = (center[0] - wanted_center[0]).powi(2)
                    + (center[1] - wanted_center[1]).powi(2)
                    + turn;
                seats.push((distance, pose));
            }
        }
    }
    seats
}

fn across<'a>(x: &'a [f64], y: &'a [f64], swap: bool) -> impl Iterator<Item = Point> + 'a {
    x.iter().copied().flat_map(move |a| {
        y.iter()
            .copied()
            .map(move |b| if swap { [b, a] } else { [a, b] })
    })
}

fn across_span(low: f64, high: f64, grid: f64) -> Vec<f64> {
    if low > high {
        return Vec::new();
    }
    let count = ((high - low) / grid).ceil().clamp(1.0, 80.0) as u32;
    (0..=count)
        .map(|i| low + (high - low) * f64::from(i) / f64::from(count))
        .collect()
}

fn near_ends(low: f64, high: f64, grid: f64) -> Vec<f64> {
    if low > high {
        return Vec::new();
    }
    let mut spots = Vec::new();
    for delta in [0.0, grid * 0.6, grid * 1.5, 0.35, 0.7, 1.2, 2.0] {
        spots.push((low + delta).min(high));
        spots.push((high - delta).max(low));
    }
    spots
}

fn edge_spots(low: f64, high: f64, wanted: f64, grid: f64) -> Vec<f64> {
    if low > high {
        return Vec::new();
    }
    let mut spots = vec![wanted.clamp(low, high)];
    for index in 0..=16 {
        spots.push(low + (high - low) * f64::from(index) / 16.0);
    }
    // Densify the places where two large modules can just fit across the
    // board. Try both seam positions even when the centre grid is coarse.
    for grids in [0.6, 1.5, 3.0] {
        spots.push((low + grids * grid).min(high));
        spots.push((high - grids * grid).max(low));
    }
    spots.sort_by(f64::total_cmp);
    spots.dedup_by(|a, b| (*a - *b).abs() < 1.0e-6);
    spots
}

fn choose_pair(
    problem: &Problem,
    poses: &mut [Pose],
    pair: &ModuleEnvelopePair,
    [first, second]: [&[(f64, Pose)]; 2],
    out_of_work: impl Fn() -> bool,
) -> Option<[Pose; 2]> {
    let modules = &pair.modules;
    let [a, b] = modules.each_ref().map(|module| module.part);
    let saved = poses[a];
    // This inexpensive pass quickly finds seats at both ends of a narrow
    // host board. Try contours that can clear diagonally if it finds none.
    let first_boxes: Vec<_> = first
        .iter()
        .map(|(_, pose)| envelope_box(&modules[0], *pose))
        .collect();
    let second_boxes: Vec<_> = second
        .iter()
        .map(|(_, pose)| envelope_box(&modules[1], *pose))
        .collect();
    crate::add_work(
        (first.len() * modules[0].vertices.len() + second.len() * modules[1].vertices.len()) as u64,
    );
    let mut best = None;
    for axes_only in [true, false] {
        let mut best_cost = f64::INFINITY;
        'first: for (i, &(a_cost, a_pose)) in first.iter().enumerate() {
            if a_cost + second[0].0 >= best_cost {
                break;
            }
            poses[a] = a_pose;
            for (j, &(b_cost, b_pose)) in second.iter().enumerate() {
                let cost = a_cost + b_cost;
                if cost >= best_cost {
                    break;
                }
                if out_of_work() {
                    break 'first;
                }
                let mut work = 1;
                let clear = (!axes_only
                    || boxes_clear(first_boxes[i], second_boxes[j], pair.min_gap_mm))
                    && clear_of(problem, poses, b, b_pose, std::iter::once(a), &mut work);
                crate::add_work(work);
                if clear {
                    best = Some([a_pose, b_pose]);
                    best_cost = cost;
                    break;
                }
            }
        }
        if best.is_some() || out_of_work() {
            break;
        }
    }
    poses[a] = saved;
    best
}

fn envelope_box(module: &ModuleEnvelope, pose: Pose) -> [f64; 4] {
    module.vertices.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |[left, top, right, bottom], &vertex| {
            let [x, y] = crate::problem::rotate(vertex, -pose.angle);
            let [x, y] = [x + pose.position[0], y + pose.position[1]];
            [left.min(x), top.min(y), right.max(x), bottom.max(y)]
        },
    )
}

fn boxes_clear(a: [f64; 4], b: [f64; 4], gap: f64) -> bool {
    a[2] + gap <= b[0] + 1.0e-6
        || b[2] + gap <= a[0] + 1.0e-6
        || a[3] + gap <= b[1] + 1.0e-6
        || b[3] + gap <= a[1] + 1.0e-6
}
