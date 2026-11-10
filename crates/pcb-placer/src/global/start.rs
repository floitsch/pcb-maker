// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The global placement's start, in its order: the immovable charge
//! (`fixed_charge`), the movable bodies and the fillers (`bodies`,
//! `add_fillers`), the start's noise (`noise_start`) and the netlist's
//! start with its stacks spread into banks (`netlist_start`,
//! `spread_stacks`; docs/placer.md, "The global placement's start").

use super::field::{Body, Field, Random};
use super::{Board, GlobalConfig, Stacks};
use crate::problem::{Point, Pose, Problem, Side, point_in_polygon};
use std::collections::BTreeMap;

/// The netlist start's share of the common start's noise
/// (`initial::quadratic_start`).
const NETLIST_NOISE: f64 = 0.1;

/// The immovable charge: outside the outline and the fixed bodies, one
/// density field per side of a two-sided board.
pub(super) struct FixedCharge {
    /// The density fields: 2 on a board with parts on both sides, else 1.
    pub(super) side_count: usize,
    /// Per field, the immovable area in each bin (at most the bin's).
    pub(super) fixed: Vec<Vec<f64>>,
    /// Per field, the area the immovable charge leaves free.
    pub(super) free: Vec<f64>,
}

/// The density fields a part on `side` charges, of `side_count`.
pub(super) fn fields_of(side: Side, side_count: usize) -> Vec<usize> {
    match side {
        Side::Front => vec![0],
        Side::Back => vec![side_count - 1],
        Side::Both => (0..side_count).collect(),
        Side::Neither => Vec::new(),
    }
}

/// The immovable charge at `poses`: the area outside the outline in each
/// bin (sampled), into `field.fixed`, and per side the fixed bodies (their
/// halos, a hollow part's boxes, a through-hole part's holes on the other
/// side).
pub(super) fn fixed_charge(problem: &Problem, poses: &[Pose], field: &mut Field) -> FixedCharge {
    let bin_area = field.bin[0] * field.bin[1];
    let n = field.bins;

    // Immovable charge: outside the outline and fixed bodies.
    const SAMPLES: usize = 4;
    for y in 0..n {
        for x in 0..n {
            let mut outside = 0;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let point = [
                        field.origin[0]
                            + (x as f64 + (sx as f64 + 0.5) / SAMPLES as f64) * field.bin[0],
                        field.origin[1]
                            + (y as f64 + (sy as f64 + 0.5) / SAMPLES as f64) * field.bin[1],
                    ];
                    outside += !point_in_polygon(point, &problem.outline) as usize;
                }
            }
            field.fixed[y * n + x] = bin_area * outside as f64 / (SAMPLES * SAMPLES) as f64;
        }
    }
    // A board with parts on both sides gets one density field per side:
    // a front part and a back part may share the same spot.
    let two_sided = problem
        .components
        .iter()
        .any(|component| component.side == Side::Front)
        && problem
            .components
            .iter()
            .any(|component| component.side == Side::Back);
    let side_count = if two_sided { 2 } else { 1 };
    let fields_of = |side: Side| fields_of(side, side_count);
    let mut side_fixed = vec![field.fixed.clone(); side_count];
    for (index, component) in problem.components.iter().enumerate() {
        if component.fixed {
            let pose = poses[index];
            let half = component.half_extent(pose.angle);
            for side in fields_of(component.side) {
                if component.hollow.is_empty() {
                    field.overlap(
                        component.center(pose),
                        [half[0] + component.halo, half[1] + component.halo],
                        |bin, area| side_fixed[side][bin] += area,
                    );
                } else {
                    for (center, half) in component.hollow_boxes(pose) {
                        field.overlap(center, half, |bin, area| side_fixed[side][bin] += area);
                    }
                }
            }
            // A through-hole part's holes are in the way on the other side.
            if two_sided && component.side != Side::Both {
                let other = 1 - fields_of(component.side).first().copied().unwrap_or(0);
                for (center, half) in component.far_boxes(pose) {
                    field.overlap(center, half, |bin, area| side_fixed[other][bin] += area);
                }
            }
        }
    }
    for fixed in &mut side_fixed {
        for value in fixed.iter_mut() {
            *value = value.min(bin_area);
        }
    }
    let side_free: Vec<f64> = side_fixed
        .iter()
        .map(|fixed| fixed.iter().map(|fixed| bin_area - fixed).sum())
        .collect();
    FixedCharge {
        side_count,
        fixed: side_fixed,
        free: side_free,
    }
}

/// The movable components' bodies at `poses`: each grown by its halo, its
/// charge scaled down where it is hollow, on its side's field.
pub(super) fn bodies(
    problem: &Problem,
    poses: &[Pose],
    movable: &[usize],
    side_count: usize,
) -> Vec<Body> {
    movable
        .iter()
        .map(|index| {
            let component = &problem.components[*index];
            let half = component.half_extent(poses[*index].angle);
            let halo = component.halo;
            Body {
                component: Some(*index),
                half: [half[0] + halo, half[1] + halo],
                bare: half,
                angle: poses[*index].angle,
                pins: component.pins.len() as f64,
                field: fields_of(component.side, side_count)
                    .first()
                    .copied()
                    .unwrap_or(0),
                density: if component.hollow.is_empty() {
                    1.0
                } else {
                    let whole = 4.0 * (half[0] + halo) * (half[1] + halo);
                    (component.blocking_area() / whole.max(1.0e-9))
                        .sqrt()
                        .min(1.0)
                },
                extra: [0.0; 4],
            }
        })
        .collect()
}

/// Fillers after the movable bodies: square charges of about a typical
/// body's size (the mean of the middle eight tenths of the areas, at least
/// a bin) that take `GlobalConfig::whitespace_fill` of each side's
/// whitespace.
pub(super) fn add_fillers(
    bodies: &mut Vec<Body>,
    config: &GlobalConfig,
    field: &Field,
    charge: &FixedCharge,
) {
    let mut areas: Vec<f64> = bodies
        .iter()
        .map(|body| 4.0 * body.charge_half()[0] * body.charge_half()[1])
        .collect();
    areas.sort_by(f64::total_cmp);
    let trimmed =
        &areas[areas.len() / 10..(areas.len() * 9).div_ceil(10).max(areas.len() / 10 + 1)];
    let filler_side = (trimmed.iter().sum::<f64>() / trimmed.len() as f64)
        .sqrt()
        .max(field.bin[0].min(field.bin[1]));
    // Fillers take a share of each side's whitespace.
    for (side, free) in charge.free.iter().enumerate().take(charge.side_count) {
        let side_movable: f64 = bodies
            .iter()
            .filter(|body| body.field == side)
            .map(|body| 4.0 * body.charge_half()[0] * body.charge_half()[1])
            .sum();
        let filler_area = config.whitespace_fill.clamp(0.0, 1.0) * (free - side_movable).max(0.0);
        let filler_count = (filler_area / (filler_side * filler_side)).floor() as usize;
        for _ in 0..filler_count {
            bodies.push(Body {
                component: None,
                half: [filler_side / 2.0; 2],
                bare: [filler_side / 2.0; 2],
                angle: 0.0,
                pins: 0.0,
                field: side,
                density: 1.0,
                extra: [0.0; 4],
            });
        }
    }
}

/// The start's noise: every body's centre (unclamped) and the start
/// point. A part starts near the start point (halfway between the fixed
/// parts' pin centroid and the board's centre, or the centre) or its own
/// start (`Constraints::starts`), a warm part at its pose's centre, a
/// filler anywhere. The netlist's start keeps a tenth of this noise.
pub(super) fn noise_start(
    board: &Board,
    poses: &[Pose],
    bodies: &[Body],
    starts: &BTreeMap<usize, Point>,
) -> (Point, Vec<Point>) {
    let (problem, config, bounds) = (board.problem, board.config, board.bounds);
    // Start from the pin centroid of fixed parts (or the board centre) with
    // a little noise; fillers start spread out.
    let mut random = Random(config.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut anchor = [0.0; 2];
    let mut anchors = 0.0;
    for (index, component) in problem.components.iter().enumerate() {
        if component.fixed {
            for pin in &component.pins {
                let at = component.pin_position(pin, poses[index]);
                anchor[0] += at[0];
                anchor[1] += at[1];
                anchors += 1.0;
            }
        }
    }
    let board_center = [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0];
    let start = if anchors > 0.0 {
        [
            0.5 * (anchor[0] / anchors) + 0.5 * board_center[0],
            0.5 * (anchor[1] / anchors) + 0.5 * board_center[1],
        ]
    } else {
        board_center
    };
    let size = [bounds[2] - bounds[0], bounds[3] - bounds[1]];
    let major: Vec<Point> = bodies
        .iter()
        .map(|body| {
            if let Some(index) = body.component {
                if config.warm.as_ref().is_some_and(|warm| warm[index]) {
                    // The noise is drawn all the same: the other parts
                    // start where they would.
                    random.next();
                    random.next();
                    return problem.components[index].center(poses[index]);
                }
                let start = starts.get(&index).copied().unwrap_or(start);
                [
                    start[0] + (random.next() - 0.5) * 0.05 * size[0],
                    start[1] + (random.next() - 0.5) * 0.05 * size[1],
                ]
            } else {
                [
                    bounds[0] + random.next() * size[0],
                    bounds[1] + random.next() * size[1],
                ]
            }
        })
        .collect();
    (start, major)
}

/// The netlist's start: the movable parts' centres in `major` (the noise
/// start, clamped) become the quadratic wirelength's solution plus a tenth
/// of their noise, the netlist's stacks spread into banks there
/// (`GlobalConfig::stacks`). Returns the banks that move as one body in
/// the descent (body indices), and the same by component index.
pub(super) fn netlist_start(
    board: &Board,
    bodies: &[Body],
    offsets: &[Vec<(Point, usize)>],
    starts: &BTreeMap<usize, Point>,
    start: Point,
    major: &mut [Point],
) -> (Vec<Vec<usize>>, Vec<Vec<usize>>) {
    let (problem, config, movable) = (board.problem, board.config, &board.movable);
    let fixed_pins = &board.fixed_pins;
    let mut glued: Vec<Vec<usize>> = Vec::new();
    let mut stacked: Vec<Vec<usize>> = Vec::new();
    // The parts start where the netlist's quadratic wirelength puts
    // them (`initial::quadratic_start`), with a tenth of the noise
    // drawn above: it breaks ties (parts on the same nets), not the
    // arrangement. From one point with the whole noise the arrangement
    // was the noise's (docs/placer.md, "The global placement's start").
    let count = movable.len();
    let warm = |index: usize| config.warm.as_ref().is_some_and(|warm| warm[index]);
    // A part with a start of its own (a panel copy's, at its region's
    // centre) begins there as before: solved, the copies' parts were
    // projected against their regions' borders and piled up there
    // (SmartSpin2k, stage 1: 33 to 51 first-route open terminals
    // became 65 to 90).
    let held: Vec<bool> = movable
        .iter()
        .map(|index| warm(*index) || starts.contains_key(index))
        .collect();
    let anchor: Vec<Point> = movable
        .iter()
        .map(|index| starts.get(index).copied().unwrap_or(start))
        .collect();
    let noise: Vec<Point> = (0..count)
        .map(|body| {
            if held[body] {
                [0.0; 2]
            } else {
                [
                    major[body][0] - anchor[body][0],
                    major[body][1] - anchor[body][1],
                ]
            }
        })
        .collect();
    let mut centers: Vec<Point> = (0..count)
        .map(|body| {
            if held[body] {
                major[body]
            } else {
                anchor[body]
            }
        })
        .collect();
    let solve_work = crate::initial::quadratic_start(
        problem,
        movable,
        offsets,
        fixed_pins,
        &anchor,
        &held,
        &mut centers,
        |centers: &mut [Point]| {
            let mut moved = centers.to_vec();
            board.clamp(&mut moved, bodies);
            centers.copy_from_slice(&moved);
        },
    );
    crate::add_work(solve_work);
    if config.stacks != Stacks::Off {
        let stacks = spread_stacks(
            problem,
            config.stacks,
            movable,
            &bodies[..count],
            offsets,
            fixed_pins,
            &held,
            &mut centers,
        );
        stacked = stacks
            .iter()
            .map(|members| members.iter().map(|body| movable[*body]).collect())
            .collect();
        glued = stacks;
    }
    for body in 0..count {
        major[body] = [
            centers[body][0] + NETLIST_NOISE * noise[body][0],
            centers[body][1] + NETLIST_NOISE * noise[body][1],
        ];
    }
    board.clamp(major, bodies);
    (glued, stacked)
}

/// Spreads the netlist's stacks (`crate::stacks`) at the solved `centers`
/// of the `movable` bodies into banks at their pitch around their
/// centroids: the parts that `rule` (`Stacks::Glue`) takes as alike and
/// whose bodies (halos included) overlap there. A bank's cell is the
/// largest body's extent with its halo on each axis; its shape is the one
/// that adds the least wire to the stack's nets (`stacks::bank_columns`),
/// the bank moved onto the board where it fits. `held` bodies stay.
/// Returns the stacks (body indices).
#[allow(clippy::too_many_arguments)]
fn spread_stacks(
    problem: &Problem,
    rule: Stacks,
    movable: &[usize],
    bodies: &[Body],
    offsets: &[Vec<(Point, usize)>],
    fixed_pins: &[(Point, usize)],
    held: &[bool],
    centers: &mut [Point],
) -> Vec<Vec<usize>> {
    let Stacks::Glue {
        share: least_share,
        least_nets,
        least_common,
        least_members,
    } = rule
    else {
        return Vec::new();
    };
    let count = movable.len();
    let nets: Vec<Vec<usize>> = offsets
        .iter()
        .map(|pins| pins.iter().map(|(_, net)| *net).collect())
        .collect();
    let mut keys = crate::stacks::net_keys(&nets, least_nets);
    for (body, key) in keys.iter_mut().enumerate() {
        if held[body] {
            *key = None;
        }
    }
    let half: Vec<Point> = bodies.iter().map(|body| body.half).collect();
    let field: Vec<usize> = bodies.iter().map(|body| body.field).collect();
    let stacks = crate::stacks::find_stacks(
        centers,
        &half,
        &field,
        &keys,
        least_share,
        least_common,
        least_members,
    );
    if stacks.is_empty() {
        return Vec::new();
    }
    let bounds = problem.bounds();
    let margin = problem.edge_margin;
    let low = [bounds[0] + margin, bounds[1] + margin];
    let high = [bounds[2] - margin, bounds[3] - margin];
    let room = [(high[0] - low[0]).max(0.0), (high[1] - low[1]).max(0.0)];
    // Each net's pins: the movable body owning it (or none) and where it
    // is at the solve.
    let mut pins_of: Vec<Vec<(Option<usize>, Point)>> = vec![Vec::new(); problem.net_weights.len()];
    for (at, net) in fixed_pins {
        pins_of[*net].push((None, *at));
    }
    for (body, pins) in offsets.iter().enumerate() {
        for (offset, net) in pins {
            pins_of[*net].push((
                Some(body),
                [centers[body][0] + offset[0], centers[body][1] + offset[1]],
            ));
        }
    }
    for members in &stacks {
        // The nets every member has, and the extent of each without the
        // stack.
        let common: Vec<usize> = keys[members[0]]
            .iter()
            .flatten()
            .copied()
            .filter(|net| {
                members.iter().all(|body| {
                    keys[*body]
                        .as_ref()
                        .is_some_and(|key| key.binary_search(net).is_ok())
                })
            })
            .collect();
        let spans: Vec<(f64, Point)> = common
            .iter()
            .map(|net| {
                let mut extent = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                for (owner, at) in &pins_of[*net] {
                    if owner.is_some_and(|owner| members.binary_search(&owner).is_ok()) {
                        continue;
                    }
                    extent = [
                        extent[0].min(at[0]),
                        extent[1].min(at[1]),
                        extent[2].max(at[0]),
                        extent[3].max(at[1]),
                    ];
                }
                let span = if extent[0] <= extent[2] {
                    [extent[2] - extent[0], extent[3] - extent[1]]
                } else {
                    [0.0, 0.0]
                };
                (problem.net_weights[*net].max(0.0), span)
            })
            .collect();
        let cell = members.iter().fold([0.0f64; 2], |cell, body| {
            [
                cell[0].max(2.0 * half[*body][0]),
                cell[1].max(2.0 * half[*body][1]),
            ]
        });
        let columns = crate::stacks::bank_columns(members.len(), cell, &spans, room);
        let rows = members.len().div_ceil(columns);
        let mut centroid = [0.0; 2];
        for body in members {
            centroid[0] += centers[*body][0] / members.len() as f64;
            centroid[1] += centers[*body][1] / members.len() as f64;
        }
        // On the board where the bank fits.
        let extent = [columns as f64 * cell[0], rows as f64 * cell[1]];
        for axis in 0..2 {
            if extent[axis] <= room[axis] {
                centroid[axis] = centroid[axis].clamp(
                    low[axis] + extent[axis] / 2.0,
                    high[axis] - extent[axis] / 2.0,
                );
            }
        }
        // The members fill the cells row by row in the order of their
        // solved centres (by y, then x), the index breaking ties.
        let mut order = members.clone();
        order.sort_by(|a, b| {
            centers[*a][1]
                .total_cmp(&centers[*b][1])
                .then(centers[*a][0].total_cmp(&centers[*b][0]))
                .then(a.cmp(b))
        });
        let cells = crate::stacks::bank_centers(members.len(), cell, columns, centroid);
        for (body, at) in order.iter().zip(cells) {
            centers[*body] = at;
        }
    }
    if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
        let said: Vec<String> = stacks
            .iter()
            .map(|members| {
                let names: Vec<&str> = members
                    .iter()
                    .take(4)
                    .map(|body| problem.components[movable[*body]].name.as_str())
                    .collect();
                format!(
                    "{} [{}{}]",
                    members.len(),
                    names.join(" "),
                    if members.len() > 4 { " ..." } else { "" }
                )
            })
            .collect();
        eprintln!(
            "placer: netlist stacks banked ({count} movable): {}",
            said.join("; ")
        );
    }
    stacks
}
