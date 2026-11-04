// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Parts whose two sides face two ways (`Constraints::facing`): a
//! differential pair's two series parts as a row (docs/placer.md, "Pair
//! rows"). The row's pads of one side lie on the pair's nets towards its
//! source, those of the other on the nets towards its destination; the
//! router launches the pair's coupled runs out of each side, so each side
//! should face its own nets' far pins and the row's axis lie across the
//! pair's direction of travel. Wirelength hardly tells the turns apart (the
//! pads are a millimetre apart, the far pins centimetres away), so after the
//! refinement each such part turns, about its body's centre and where that
//! stays legal, to the quarter turn that faces its two sides' far pins best
//! (`face`). Where the far pins of both sides lie in about one direction,
//! or the best two turns face them about as well (the far ends diagonal),
//! the part keeps the turn the placement gave it.

use crate::problem::{Point, Pose, Problem};

/// A part whose two sides face two ways: the nets of each side (pins of
/// the part on them are that side's; other parts' pins on them its far
/// pins).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facing {
    pub part: usize,
    pub nets: [Vec<usize>; 2],
}

/// How a part faces its far pins at a pose (`facing`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Faced {
    /// It faces them as well as any turn does (or as well within the
    /// tolerance: the far ends diagonal to it).
    Faces,
    /// The far pins tell no direction: none on either side, or both sides'
    /// in about one direction from the part.
    Ambiguous,
    /// Another turn faces them better: the best turn's angle.
    Away(f64),
}

/// Below this length the wanted direction (the sum of two unit vectors)
/// tells no direction: both sides' far pins within about 60 degrees of
/// each other as seen from the part.
const AMBIGUOUS: f64 = 1.0;

/// Turns whose facing (the cosine of the angle between the part's side
/// and the wanted direction) is within this of the best's face as well.
const TOLERANCE: f64 = 0.2;

fn unit(vector: Point) -> Option<Point> {
    let length = (vector[0] * vector[0] + vector[1] * vector[1]).sqrt();
    (length > 1.0e-9).then(|| [vector[0] / length, vector[1] / length])
}

/// The direction side 0 of `facing` should point in at `poses` (from the
/// part's centre: towards side 0's far pins and away from side 1's), and
/// the part's own direction from side 1's pins to side 0's in its frame.
/// `None`: no such direction (no pins of a side on the part, no far pin
/// at all, or the wanted direction too short to tell).
fn wanted(problem: &Problem, poses: &[Pose], facing: &Facing) -> Option<(Point, Point)> {
    let component = &problem.components[facing.part];
    let center = component.center(poses[facing.part]);
    let mut own = [[0.0, 0.0]; 2];
    let mut want = [0.0, 0.0];
    let mut any_far = false;
    for (side, nets) in facing.nets.iter().enumerate() {
        let pins: Vec<Point> = component
            .pins
            .iter()
            .filter(|pin| nets.contains(&pin.net))
            .map(|pin| pin.offset)
            .collect();
        if pins.is_empty() {
            return None;
        }
        own[side] = [
            pins.iter().map(|pin| pin[0]).sum::<f64>() / pins.len() as f64,
            pins.iter().map(|pin| pin[1]).sum::<f64>() / pins.len() as f64,
        ];
        let (mut sum, mut count) = ([0.0, 0.0], 0usize);
        for (index, other) in problem.components.iter().enumerate() {
            if index == facing.part {
                continue;
            }
            for pin in other.pins.iter().filter(|pin| nets.contains(&pin.net)) {
                let at = other.pin_position(pin, poses[index]);
                sum = [sum[0] + at[0], sum[1] + at[1]];
                count += 1;
            }
        }
        if count == 0 {
            continue;
        }
        let far = [sum[0] / count as f64, sum[1] / count as f64];
        if let Some(toward) = unit([far[0] - center[0], far[1] - center[1]]) {
            let sign = if side == 0 { 1.0 } else { -1.0 };
            want = [want[0] + sign * toward[0], want[1] + sign * toward[1]];
            any_far = true;
        }
    }
    let local = unit([own[0][0] - own[1][0], own[0][1] - own[1][1]])?;
    let length = (want[0] * want[0] + want[1] * want[1]).sqrt();
    // One side's far pins alone give a unit vector: a direction.
    let one_side = (length - 1.0).abs() < 1.0e-9;
    if !any_far || (length < AMBIGUOUS && !one_side) {
        return None;
    }
    Some((unit(want)?, local))
}

/// The turns of `facing`'s part by how well side 0 points along `want`
/// (best first, ties in the options' order).
fn ranked(problem: &Problem, facing: &Facing, want: Point, local: Point) -> Vec<(f64, f64)> {
    let component = &problem.components[facing.part];
    let mut scored: Vec<(f64, f64)> = component
        .angle_options
        .iter()
        .map(|&angle| {
            let board = component.offset(local, angle);
            (board[0] * want[0] + board[1] * want[1], angle)
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored
}

/// How `facing`'s part faces its far pins at `poses`.
pub fn faced(problem: &Problem, poses: &[Pose], facing: &Facing) -> Faced {
    let Some((want, local)) = wanted(problem, poses, facing) else {
        return Faced::Ambiguous;
    };
    let scored = ranked(problem, facing, want, local);
    let Some(&(best, best_angle)) = scored.first() else {
        return Faced::Ambiguous;
    };
    let angle = poses[facing.part].angle;
    let current = problem.components[facing.part].offset(local, angle);
    let score = current[0] * want[0] + current[1] * want[1];
    if score >= best - TOLERANCE {
        Faced::Faces
    } else {
        Faced::Away(best_angle)
    }
}

/// What `face` did: parts turned, and parts facing away that no better
/// turn fits (each turn about the body's centre illegal).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Faces {
    pub turned: usize,
    pub held: usize,
}

/// How far a turned part may move off its body's centre to find a legal
/// place (millimetres), and in what steps: a row of two 0402s is about as
/// long as wide, so its turn about the centre mostly fits; where its
/// neighbours sit tight, a nudge does.
const NUDGE: f64 = 0.5;
const NUDGE_STEP: f64 = 0.05;

/// The nudges (dx, dy) within `NUDGE`, nearest first (none first).
fn nudges() -> Vec<Point> {
    let steps = (NUDGE / NUDGE_STEP).round() as i64;
    let mut out: Vec<Point> = (-steps..=steps)
        .flat_map(|x| (-steps..=steps).map(move |y| [x as f64 * NUDGE_STEP, y as f64 * NUDGE_STEP]))
        .filter(|offset| offset[0].hypot(offset[1]) <= NUDGE + 1.0e-9)
        .collect();
    out.sort_by(|a, b| {
        a[0].hypot(a[1])
            .total_cmp(&b[0].hypot(b[1]))
            .then(a[0].total_cmp(&b[0]))
            .then(a[1].total_cmp(&b[1]))
    });
    out
}

/// Turns each movable part of `problem.constraints.facing` that faces away
/// from its far pins at `poses` to the best turn that is legal there,
/// about its body's centre or nudged off it by up to `NUDGE` (the turns
/// within the tolerance of the best, best first; per turn the nearest
/// nudge). Followers are not synced (the caller does it).
pub fn face(problem: &Problem, poses: &mut [Pose]) -> Faces {
    let nudges = nudges();
    let mut faces = Faces::default();
    let count = problem.components.len();
    for facing in &problem.constraints.facing {
        let component = &problem.components[facing.part];
        if component.fixed || component.angle_options.len() < 2 {
            continue;
        }
        if !matches!(faced(problem, poses, facing), Faced::Away(_)) {
            continue;
        }
        let Some((want, local)) = wanted(problem, poses, facing) else {
            continue;
        };
        let scored = ranked(problem, facing, want, local);
        let best = scored[0].0;
        let center = component.center(poses[facing.part]);
        let mut turned = false;
        'turns: for &(_, angle) in scored
            .iter()
            .filter(|(score, _)| *score >= best - TOLERANCE)
        {
            for nudge in &nudges {
                let position = crate::constraints::snap_position(
                    problem,
                    facing.part,
                    component
                        .position_for_center([center[0] + nudge[0], center[1] + nudge[1]], angle),
                    angle,
                );
                let pose = Pose { position, angle };
                if crate::legal::is_legal(problem, poses, facing.part, pose, 0..count) {
                    poses[facing.part] = pose;
                    turned = true;
                    break 'turns;
                }
            }
        }
        if turned {
            faces.turned += 1;
        } else {
            faces.held += 1;
        }
    }
    faces
}

/// Before the legaliser, which seats the parts of an annealed placement
/// (whose turns the wirelength hardly told apart) and may turn a part to
/// seat it: each movable part of `problem.constraints.facing` that faces
/// away from its far pins at `poses` turns to the best turn about its
/// body's centre, legal or not, and every such part where the far pins
/// tell a direction keeps to the turns that face them (its angle options
/// cut to those, in `problem`, the level's own). Followers are not synced.
/// Returns the number of parts turned.
pub fn turn_to_face(problem: &mut Problem, poses: &mut [Pose]) -> usize {
    let turned = turn(problem, poses);
    for index in 0..problem.constraints.facing.len() {
        let facing = &problem.constraints.facing[index];
        let component = &problem.components[facing.part];
        if component.fixed || component.angle_options.len() < 2 {
            continue;
        }
        let Some((want, local)) = wanted(problem, poses, facing) else {
            continue;
        };
        let scored = ranked(problem, facing, want, local);
        let best = scored[0].0;
        let mut facing_turns: Vec<f64> = scored
            .iter()
            .filter(|(score, _)| *score >= best - TOLERANCE)
            .map(|(_, angle)| *angle)
            .collect();
        crate::orient::sort_angles(&mut facing_turns);
        let part = facing.part;
        problem.components[part].angle_options = facing_turns;
    }
    turned
}

fn turn(problem: &Problem, poses: &mut [Pose]) -> usize {
    let mut turned = 0;
    for facing in &problem.constraints.facing {
        let component = &problem.components[facing.part];
        if component.fixed || component.angle_options.len() < 2 {
            continue;
        }
        let Faced::Away(angle) = faced(problem, poses, facing) else {
            continue;
        };
        let center = component.center(poses[facing.part]);
        let position = crate::constraints::snap_position(
            problem,
            facing.part,
            component.position_for_center(center, angle),
            angle,
        );
        poses[facing.part] = Pose { position, angle };
        turned += 1;
    }
    turned
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::problem::{Component, Pin, Side};

    /// A part of `size` with `pins` (offset, net).
    fn part(size: Point, pins: &[(Point, usize)], fixed: bool) -> Component {
        Component {
            name: "P".into(),
            body_center: [0.0, 0.0],
            body_size: size,
            round: false,
            halo: 0.0,
            pins: pins
                .iter()
                .map(|&(offset, net)| Pin { offset, net })
                .collect(),
            side: Side::Front,
            fixed,
            angle_options: if fixed {
                vec![0.0]
            } else {
                vec![0.0, 90.0, 180.0, 270.0]
            },
            far_side: Vec::new(),
            far_copper: Vec::new(),
            far_courtyard: Vec::new(),
            hollow: Vec::new(),
            tight: None,
            edge_inset: 0.0,
            courtyards: Vec::new(),
            holes_inside: false,
            pads: pins
                .iter()
                .map(|&(offset, _)| {
                    [
                        offset[0] - 0.2,
                        offset[1] - 0.2,
                        offset[0] + 0.2,
                        offset[1] + 0.2,
                    ]
                })
                .collect(),
            copper_only: false,
            cutout_outline: Vec::new(),
            tight_hollow: Vec::new(),
            hole_rings: Vec::new(),
            copper_graphics: Vec::new(),
            courtless: Vec::new(),
            slots: Vec::new(),
        }
    }

    /// Two 0402 capacitors as a row (the pair's source side, nets 0 and 1,
    /// at x = -0.48; its destination side, nets 2 and 3, at +0.48) at
    /// (20, 20) turned by `angle`, a connector with the source nets at
    /// `source` and one with the destination nets at `destination`.
    fn board(angle: f64, source: Point, destination: Point) -> (Problem, Vec<Pose>) {
        let row = part(
            [1.82, 1.85],
            &[
                ([-0.48, -0.465], 0),
                ([0.48, -0.465], 2),
                ([-0.48, 0.465], 1),
                ([0.48, 0.465], 3),
            ],
            false,
        );
        let source_part = part([2.0, 2.0], &[([0.0, -0.25], 0), ([0.0, 0.25], 1)], true);
        let destination_part = part([2.0, 2.0], &[([0.0, -0.25], 2), ([0.0, 0.25], 3)], true);
        let poses = vec![
            Pose {
                position: [20.0, 20.0],
                angle,
            },
            Pose {
                position: source,
                angle: 0.0,
            },
            Pose {
                position: destination,
                angle: 0.0,
            },
        ];
        let mut problem = Problem {
            outline: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 40.0], [0.0, 40.0]],
            components: vec![row, source_part, destination_part],
            net_weights: vec![1.0; 4],
            poses: poses.clone(),
            spacing: 0.2,
            grid: 0.01,
            edge_margin: 0.5,
            min_spacing: 0.2,
            constraints: Default::default(),
            far_side_pads_only: false,
            shared_halos: false,
            pieces: Vec::new(),
            escape: None,
            crossings: None,
        };
        problem.constraints.facing = vec![Facing {
            part: 0,
            nets: [vec![0, 1], vec![2, 3]],
        }];
        (problem, poses)
    }

    /// The board direction of the source side (x of the part's frame).
    fn source_side(problem: &Problem, poses: &[Pose]) -> Point {
        let board = problem.components[0].offset([-1.0, 0.0], poses[0].angle);
        [board[0].round(), board[1].round()]
    }

    #[test]
    fn a_pair_row_turns_its_sides_to_the_pairs_two_ends() {
        // The source to the left, the destination to the right: the row
        // at a quarter turn faces up and down, and turns to face them.
        let (problem, mut poses) = board(90.0, [5.0, 20.0], [35.0, 20.0]);
        assert!(matches!(
            faced(&problem, &poses, &problem.constraints.facing[0]),
            Faced::Away(_)
        ));
        let faces = face(&problem, &mut poses);
        assert_eq!(faces, Faces { turned: 1, held: 0 });
        assert_eq!(source_side(&problem, &poses), [-1.0, 0.0]);
        // About its centre.
        let center = problem.components[0].center(poses[0]);
        assert!((center[0] - 20.0).abs() < 0.02 && (center[1] - 20.0).abs() < 0.02);
        assert_eq!(
            faced(&problem, &poses, &problem.constraints.facing[0]),
            Faced::Faces
        );
        // Before the legaliser: turned whether legal or not, and held to
        // the turn that faces.
        let (mut problem, mut poses) = board(90.0, [5.0, 20.0], [35.0, 20.0]);
        assert_eq!(turn_to_face(&mut problem, &mut poses), 1);
        assert_eq!(source_side(&problem, &poses), [-1.0, 0.0]);
        assert_eq!(problem.components[0].angle_options, [0.0]);
        assert_eq!(turn_to_face(&mut problem, &mut poses), 0);
        // The ends diagonal: both turns that face them stay.
        let (mut problem, mut poses) = board(90.0, [5.0, 35.0], [35.0, 5.0]);
        assert_eq!(turn_to_face(&mut problem, &mut poses), 0);
        assert_eq!(problem.components[0].angle_options, [0.0, 90.0]);
        // Facing the wrong way round (a half turn): turned too.
        let (problem, mut poses) = board(180.0, [5.0, 20.0], [35.0, 20.0]);
        assert_eq!(face(&problem, &mut poses).turned, 1);
        assert_eq!(source_side(&problem, &poses), [-1.0, 0.0]);
        // The source left, the destination up and to the right: the side
        // towards the source faces left (the run from the source comes in
        // straight, the other leaves to the right and turns up).
        let (problem, mut poses) = board(90.0, [5.0, 20.0], [30.0, 2.0]);
        face(&problem, &mut poses);
        assert_eq!(source_side(&problem, &poses), [-1.0, 0.0]);
    }

    #[test]
    fn a_pair_row_keeps_its_turn_where_the_ends_tell_no_direction() {
        // Both ends to the left: no side can face both.
        let (problem, mut poses) = board(90.0, [5.0, 19.0], [5.0, 21.0]);
        assert_eq!(
            faced(&problem, &poses, &problem.constraints.facing[0]),
            Faced::Ambiguous
        );
        assert_eq!(face(&problem, &mut poses), Faces::default());
        assert_eq!(poses[0].angle, 90.0);
        // The ends diagonal (source down-left, destination up-right): a
        // quarter turn either way faces them as well.
        for angle in [0.0, 90.0] {
            let (problem, mut poses) = board(angle, [5.0, 35.0], [35.0, 5.0]);
            assert_eq!(
                faced(&problem, &poses, &problem.constraints.facing[0]),
                Faced::Faces
            );
            assert_eq!(face(&problem, &mut poses), Faces::default());
            assert_eq!(poses[0].angle, angle);
        }
    }

    #[test]
    fn a_pair_row_turns_off_its_centre_where_a_neighbour_is_close() {
        // A row 1 x 2 mm in its frame lying along a block 0.3 mm above it:
        // turned about its centre it would reach into the block, nudged
        // down 0.4 mm it keeps the spacing.
        let (mut problem, mut poses) = board(90.0, [5.0, 20.0], [35.0, 20.0]);
        problem.components[0].body_size = [1.0, 2.0];
        problem.components.push(part([10.0, 4.0], &[], true));
        poses.push(Pose {
            position: [20.0, 17.2],
            angle: 0.0,
        });
        problem.poses = poses.clone();
        assert_eq!(face(&problem, &mut poses), Faces { turned: 1, held: 0 });
        assert_eq!(source_side(&problem, &poses), [-1.0, 0.0]);
        let center = problem.components[0].center(poses[0]);
        assert!((center[0] - 20.0).abs() < 0.02, "{center:?}");
        assert!(center[1] > 20.35 && center[1] < 20.45, "{center:?}");
    }

    #[test]
    fn a_pair_row_without_room_to_turn_is_held() {
        // A long row (1 x 3 mm in its frame) at a quarter turn, lying
        // along a channel 1.5 mm high between two fixed blocks: the turn
        // that faces the ends stands it across the channel.
        let (mut problem, mut poses) = board(90.0, [5.0, 20.0], [35.0, 20.0]);
        problem.components[0].body_size = [1.0, 3.0];
        let block = part([10.0, 4.0], &[], true);
        problem.components.push(block.clone());
        problem.components.push(block);
        poses.push(Pose {
            position: [20.0, 17.25],
            angle: 0.0,
        });
        poses.push(Pose {
            position: [20.0, 22.75],
            angle: 0.0,
        });
        problem.poses = poses.clone();
        let faces = face(&problem, &mut poses);
        assert_eq!(faces, Faces { turned: 0, held: 1 });
        assert_eq!(poses[0].angle, 90.0);
    }
}
