// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Routing room from congestion: per-part halos from the routing demand
//! around each part (RUDY) instead of from its pin count alone.
//!
//! The pin-count halo is the same wherever a part sits; measured on routed
//! boards (2026-10-14, `experiments/routing-room`), the routing density in
//! a part's neighbourhood does not follow its pin count (Spearman about 0
//! per board) but follows the RUDY demand there (0.4-0.8), and 15-75 % of
//! the halo area on boards with full halos carries under 5 % of its
//! capacity. Here each part's halo is scaled by the demand around it,
//! relative to the board's median, and the scales are normalized so that
//! the halos take no more area than the pin-count halos did.

use crate::problem::{Pose, Problem, Side};

#[derive(Clone, Debug)]
pub struct RoutingRoom {
    /// Per net: how much its wire counts as routing demand (0 for nets a
    /// plane carries).
    pub net_demand: Vec<f64>,
    /// The ring around a body over which its demand is read, in mm.
    pub band: f64,
    /// Parts whose demand is below this share of the median get no halo.
    pub floor: f64,
    /// The largest factor on a part's pin-count halo.
    pub most: f64,
    /// Share of the pin-count halos' area the new halos may take.
    pub share: f64,
    /// Every movable part's pin-count halo (before the board's fit and
    /// the level's scales) is the base the demand scales, so a part the
    /// level left without halo (small parts on a crowded board) gets one
    /// where the demand is high; the area stays the level's. Without it,
    /// only the halos the level kept are redistributed.
    pub all_parts: bool,
}

impl Default for RoutingRoom {
    fn default() -> Self {
        Self {
            net_demand: Vec::new(),
            band: 1.0,
            floor: 0.5,
            most: 2.0,
            share: 1.0,
            all_parts: true,
        }
    }
}

/// A net's spanning-tree length (Prim on up to 64 pins; larger nets their
/// half perimeter times a pin-count factor) and its pins' bounding box.
fn net_wire(points: &[[f64; 2]]) -> f64 {
    let n = points.len();
    if n < 2 {
        return 0.0;
    }
    let manhattan = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() + (a[1] - b[1]).abs();
    if n > 64 {
        let (mut low, mut high) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for point in points {
            for axis in 0..2 {
                low[axis] = low[axis].min(point[axis]);
                high[axis] = high[axis].max(point[axis]);
            }
        }
        return manhattan(low, high) * (n as f64).sqrt() / 2.0;
    }
    let mut in_tree = vec![false; n];
    let mut best = vec![f64::INFINITY; n];
    best[0] = 0.0;
    let mut total = 0.0;
    for _ in 0..n {
        let next = (0..n)
            .filter(|index| !in_tree[*index])
            .min_by(|a, b| best[*a].total_cmp(&best[*b]))
            .unwrap();
        in_tree[next] = true;
        total += best[next];
        for other in 0..n {
            if !in_tree[other] {
                best[other] = best[other].min(manhattan(points[next], points[other]));
            }
        }
    }
    total
}

fn overlap(a: [f64; 4], b: [f64; 4]) -> f64 {
    let width = a[2].min(b[2]) - a[0].max(b[0]);
    let height = a[3].min(b[3]) - a[1].max(b[1]);
    if width > 0.0 && height > 0.0 {
        width * height
    } else {
        0.0
    }
}

/// Per component, the routing demand density (RUDY: each net's wire
/// length over its box, at least `band` wide) averaged over its body grown
/// by `band`. Fixed parts and parts without a side get 0.
pub fn demand(problem: &Problem, poses: &[Pose], room: &RoutingRoom) -> Vec<f64> {
    let nets = problem.net_weights.len();
    let mut points: Vec<Vec<[f64; 2]>> = vec![Vec::new(); nets];
    for (component, pose) in problem.components.iter().zip(poses) {
        for pin in &component.pins {
            if pin.net < nets {
                points[pin.net].push(component.pin_position(pin, *pose));
            }
        }
    }
    let mut boxes = Vec::new();
    for (net, points) in points.iter().enumerate() {
        let weight = room.net_demand.get(net).copied().unwrap_or(1.0);
        if points.len() < 2 || weight <= 0.0 {
            continue;
        }
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for point in points {
            bounds = [
                bounds[0].min(point[0]),
                bounds[1].min(point[1]),
                bounds[2].max(point[0]),
                bounds[3].max(point[1]),
            ];
        }
        for axis in 0..2 {
            let missing = 2.0 * room.band - (bounds[axis + 2] - bounds[axis]);
            if missing > 0.0 {
                bounds[axis] -= missing / 2.0;
                bounds[axis + 2] += missing / 2.0;
            }
        }
        let area = (bounds[2] - bounds[0]) * (bounds[3] - bounds[1]);
        boxes.push((bounds, weight * net_wire(points) / area));
    }
    crate::add_work((boxes.len() * problem.components.len()) as u64);
    problem
        .components
        .iter()
        .zip(poses)
        .map(|(component, pose)| {
            if component.fixed || component.side == Side::Neither {
                return 0.0;
            }
            let center = component.center(*pose);
            let half = component.half_extent(pose.angle);
            let ring = [
                center[0] - half[0] - room.band,
                center[1] - half[1] - room.band,
                center[0] + half[0] + room.band,
                center[1] + half[1] + room.band,
            ];
            let area = (ring[2] - ring[0]) * (ring[3] - ring[1]);
            boxes
                .iter()
                .map(|(bounds, density)| density * overlap(*bounds, ring))
                .sum::<f64>()
                / area
        })
        .collect()
}

/// The area halos `halos` add to the parts that move (as `fit_halos`
/// counts it: body plus both halos plus the spacing, less body plus
/// spacing).
fn halo_area(problem: &Problem, halos: &[f64]) -> f64 {
    problem
        .components
        .iter()
        .zip(halos)
        .filter(|(component, _)| {
            !component.fixed
                && !component.copper_only
                && component.side != Side::Neither
                && component.hollow.is_empty()
        })
        .map(|(component, halo)| {
            let sides = if problem.shared_halos { 1.0 } else { 2.0 };
            let (width, height) = (
                component.body_size[0] + problem.spacing,
                component.body_size[1] + problem.spacing,
            );
            (width + sides * halo) * (height + sides * halo) - width * height
        })
        .sum()
}

/// New halos for `problem` (whose halos are the pin-count halos at the
/// placement's level) from the demand at `poses`: each movable part's base
/// halo (`requested`, the pin-count halo before fitting and levels, with
/// `all_parts`; else its halo at the level) times
/// `factor * demand / median demand`, at most `most` times, none below
/// `floor` times the median; `factor` chosen so the halos take `share` of
/// the area the level's halos took. Fixed parts keep theirs.
pub fn halos(problem: &Problem, poses: &[Pose], room: &RoutingRoom, requested: &[f64]) -> Vec<f64> {
    let old: Vec<f64> = problem
        .components
        .iter()
        .map(|component| component.halo)
        .collect();
    let base: Vec<f64> = if room.all_parts && requested.len() == old.len() {
        requested.to_vec()
    } else {
        old.clone()
    };
    let demand = demand(problem, poses, room);
    let movable: Vec<usize> = (0..problem.components.len())
        .filter(|index| {
            let component = &problem.components[*index];
            !component.fixed && component.side != Side::Neither && base[*index] > 0.0
        })
        .collect();
    if movable.is_empty() {
        return old;
    }
    let mut sorted: Vec<f64> = movable.iter().map(|index| demand[*index]).collect();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    if median <= 0.0 {
        return old;
    }
    let ratio: Vec<f64> = demand.iter().map(|value| value / median).collect();
    let scaled = |factor: f64| -> Vec<f64> {
        let mut halos = old.clone();
        for index in &movable {
            halos[*index] = if ratio[*index] < room.floor {
                0.0
            } else {
                base[*index] * (factor * ratio[*index]).min(room.most)
            };
        }
        halos
    };
    let target = room.share * halo_area(problem, &old);
    // The area grows with the factor: bisect for the target (or the
    // largest factor, where every part is at `most`).
    let (mut low, mut high) = (0.0f64, 1.0f64);
    while halo_area(problem, &scaled(high)) < target && high < 1.0e3 {
        low = high;
        high *= 2.0;
    }
    for _ in 0..40 {
        let middle = (low + high) / 2.0;
        if halo_area(problem, &scaled(middle)) < target {
            low = middle;
        } else {
            high = middle;
        }
    }
    scaled(low)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::problem::{Component, Pin};

    fn part(name: &str, pins: Vec<Pin>, halo: f64) -> Component {
        Component {
            name: name.into(),
            body_center: [0.0, 0.0],
            body_size: [1.0, 1.0],
            round: false,
            halo,
            pins,
            side: Side::Front,
            fixed: false,
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
        }
    }

    fn pin(net: usize) -> Pin {
        Pin {
            offset: [0.0, 0.0],
            net,
        }
    }

    /// Four parts in a row, 3 mm apart: A-B and B-C joined by several
    /// nets (the busy middle), D far off with one net to C. B and C sit
    /// where the wires run and get more halo; D, alone at the end, none.
    fn board() -> (Problem, Vec<Pose>) {
        let mut components = vec![
            part("A", (0..4).map(pin).collect(), 0.65),
            part("B", (0..8).map(pin).collect(), 0.65),
            part("C", (4..8).map(pin).chain([pin(8)]).collect(), 0.65),
            part("D", vec![pin(8)], 0.65),
        ];
        components[3].body_size = [1.0, 1.0];
        let poses = [0.0, 3.0, 6.0, 30.0]
            .map(|x| Pose {
                position: [x, 0.0],
                angle: 0.0,
            })
            .to_vec();
        let problem = Problem {
            outline: vec![[-5.0, -5.0], [40.0, -5.0], [40.0, 5.0], [-5.0, 5.0]],
            components,
            net_weights: vec![1.0; 9],
            poses: poses.clone(),
            spacing: 0.5,
            grid: 0.1,
            edge_margin: 0.5,
            min_spacing: 0.2,
            constraints: Default::default(),
            far_side_pads_only: false,
            shared_halos: false,
            escape: None,
            pieces: Vec::new(),
        };
        (problem, poses)
    }

    #[test]
    fn demand_follows_the_wires() {
        let (problem, poses) = board();
        let room = RoutingRoom {
            net_demand: vec![1.0; 9],
            ..Default::default()
        };
        let demand = demand(&problem, &poses, &room);
        // B lies under all eight short nets; D under only the long one.
        assert!(demand[1] > demand[0] && demand[1] > demand[3], "{demand:?}");
        assert!(demand[3] < demand[2], "{demand:?}");
        // A plane's net is no demand.
        let mut planes = room.clone();
        planes.net_demand[8] = 0.0;
        assert_eq!(super::demand(&problem, &poses, &planes)[3], 0.0);
    }

    #[test]
    fn halos_move_to_the_busy_parts_within_the_old_area() {
        let (problem, poses) = board();
        let room = RoutingRoom {
            net_demand: vec![1.0; 9],
            ..Default::default()
        };
        let old: Vec<f64> = problem
            .components
            .iter()
            .map(|component| component.halo)
            .collect();
        let new = halos(&problem, &poses, &room, &old);
        // D, far from the demand, gets none; B the most, within `most`.
        assert_eq!(new[3], 0.0, "{new:?}");
        assert!(
            new[1] >= new[0] && new[1] > old[1] && new[1] <= room.most * old[1] + 1.0e-9,
            "{new:?}"
        );
        // The total area stays the old one.
        let (before, after) = (halo_area(&problem, &old), halo_area(&problem, &new));
        assert!(
            (after - before).abs() < 1.0e-6 * before,
            "{before} against {after}"
        );
        // A smaller share takes less.
        let half = RoutingRoom { share: 0.5, ..room };
        let less = halos(&problem, &poses, &half, &old);
        assert!((halo_area(&problem, &less) - 0.5 * before).abs() < 1.0e-6 * before);
    }

    #[test]
    fn parts_the_level_left_without_halo_get_one_where_it_is_busy() {
        let (mut problem, poses) = board();
        let requested: Vec<f64> = problem
            .components
            .iter()
            .map(|component| component.halo)
            .collect();
        // The level kept only A's halo (doubled for the area).
        for component in &mut problem.components[1..] {
            component.halo = 0.0;
        }
        problem.components[0].halo = 1.3;
        let room = RoutingRoom {
            net_demand: vec![1.0; 9],
            ..Default::default()
        };
        let new = halos(&problem, &poses, &room, &requested);
        // B, in the middle of the demand, gets a halo now; D still none.
        assert!(new[1] > 0.0 && new[3] == 0.0, "{new:?}");
        let level: Vec<f64> = problem
            .components
            .iter()
            .map(|component| component.halo)
            .collect();
        assert!(
            (halo_area(&problem, &new) - halo_area(&problem, &level)).abs()
                < 1.0e-6 * halo_area(&problem, &level)
        );
        // Within the level only A's halo moves: nobody else gets one.
        let within = halos(
            &problem,
            &poses,
            &RoutingRoom {
                all_parts: false,
                ..room
            },
            &requested,
        );
        assert!(within[1..].iter().all(|halo| *halo == 0.0), "{within:?}");
    }

    #[test]
    fn spanning_tree_of_a_square() {
        let points = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        assert!((net_wire(&points) - 3.0).abs() < 1.0e-12);
    }
}
