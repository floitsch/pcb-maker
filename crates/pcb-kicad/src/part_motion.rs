// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! A part's copper on a lowered board, carried rigidly with the part: the
//! placer's measured escape demand and its pad exits lower a placement
//! once and move the parts that move since into that board
//! (`part_copper`, `apply`) instead of lowering it again.

use pcb_placer as placer;
use pcb_router as core;
use pcb_router::geometry::{Aabb, Point, Shape};
use std::collections::HashMap;

/// What a part owns on the lowered board: its obstacles and its
/// terminals (net, terminal).
#[derive(Clone, Debug, Default)]
pub(crate) struct PartCopper {
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

/// Each part's copper on `board` (the parts by `index`, `parts` of them).
pub(crate) fn part_copper(
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

/// Applies `moved` parts' motions to `board` in place (obstacles and
/// terminal anchors); returns the area touched before and after and the
/// nets with a terminal on them.
pub(crate) fn apply(
    board: &mut core::Board,
    copper: &[PartCopper],
    old: &[placer::Pose],
    new: &[placer::Pose],
    moved: &[usize],
) -> (Aabb, Vec<usize>) {
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
        }
        for &(net, terminal) in &copper[part].terminals {
            let anchor = board.nets[net].terminals[terminal].anchor;
            board.nets[net].terminals[terminal].anchor = map(anchor);
            nets.push(net);
        }
    }
    nets.sort_unstable();
    nets.dedup();
    let area = area.unwrap_or(Aabb {
        minimum: [0.0, 0.0],
        maximum: [0.0, 0.0],
    });
    (area.inflated(1.0), nets)
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
