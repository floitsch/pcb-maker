// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The global placement's start from the netlist (RePlAce's initial
//! placement): the movable parts' centres that minimise the nets'
//! quadratic wirelength in the bound-to-bound model (Spindler, Schlichtmann
//! and Johannes, "Kraftwerk2"), with the fixed parts' pins as anchors.
//! Started from one point with a little noise (`global::global_place`
//! without it), the descent's density spreads the parts in the order the
//! noise gave them and the wires never rearrange them: the active parts'
//! arrangement was its start's noise (docs/placer.md, "Where the
//! arrangement is lost (2026-11-04)").
//!
//! Each axis is a sparse positive definite system, solved by conjugate
//! gradients with a diagonal preconditioner; the bound-to-bound weights
//! are linearised again from the solution a few times. A part whose
//! constraints hold it (an edge, a region, the board's bounds) is
//! projected there after each solve and anchored at the projection in the
//! solves after it, so that its neighbours follow it to its edge.

use crate::problem::{Point, Problem};

/// Bound-to-bound linearisations at most.
const ROUNDS: usize = 12;
/// The rounds stop where no part moves more than this share of the
/// board's larger side.
const SETTLED: f64 = 1.0e-4;
/// Conjugate-gradient iterations at most per solve.
const CG_ITERATIONS: usize = 400;
/// The solve stops at this residual relative to the right-hand side's.
const CG_TOLERANCE: f64 = 1.0e-8;
/// The least pin distance in a bound-to-bound weight, as a share of the
/// board's larger side (two pins on one spot would weigh infinitely).
const LEAST_DISTANCE: f64 = 1.0 / 200.0;
/// Every part's pull to its anchor point (the common start, or its own:
/// `Constraints::starts`), as a share of the mean net stiffness: without
/// fixed pins the system is singular, and a part without nets needs a
/// place.
const ANCHOR_SHARE: f64 = 1.0e-3;
/// A projected part's pull to its projection, in mean net stiffnesses.
const PROJECTED: f64 = 1.0e3;

/// The netlist's start for the movable bodies `movable` (component
/// indices) with pin offsets `offsets` (per body: offset from its centre at
/// its angle, and net), the fixed pins `fixed_pins`, each body's `anchor`
/// and whether it is `held` at its centre in `centers` (a diagnostic warm
/// start: it acts as a fixed part). `centers` is the start of the first
/// linearisation and receives the solution; `project` moves centres to
/// where their constraints allow. Returns the work done (in the global
/// placement's units: a matrix entry visited).
#[allow(clippy::too_many_arguments)]
pub fn quadratic_start(
    problem: &Problem,
    movable: &[usize],
    offsets: &[Vec<(Point, usize)>],
    fixed_pins: &[(Point, usize)],
    anchor: &[Point],
    held: &[bool],
    centers: &mut [Point],
    mut project: impl FnMut(&mut [Point]),
) -> u64 {
    let count = movable.len();
    debug_assert_eq!(offsets.len(), count);
    let bounds = problem.bounds();
    let side = (bounds[2] - bounds[0])
        .max(bounds[3] - bounds[1])
        .max(1.0e-6);
    let least = side * LEAST_DISTANCE;
    let nets = problem.net_weights.len();
    // The pins of each net: (owner body or None for a fixed pin, offset
    // from the owner's centre or the fixed position).
    let mut members: Vec<Vec<(Option<usize>, Point)>> = vec![Vec::new(); nets];
    for (at, net) in fixed_pins {
        members[*net].push((None, *at));
    }
    for (body, pins) in offsets.iter().enumerate() {
        for (offset, net) in pins {
            members[*net].push((Some(body), *offset));
        }
    }
    let mut projected: Vec<[Option<f64>; 2]> = vec![[None; 2]; count];
    let mut work = 0u64;
    let mut solution = vec![0.0; count];
    for _ in 0..ROUNDS {
        let before: Vec<Point> = centers.to_vec();
        for axis in 0..2 {
            let mut diagonal = vec![0.0; count];
            let mut rhs = vec![0.0; count];
            let mut edges: Vec<(usize, usize, f64)> = Vec::new();
            for (net, pins) in members.iter().enumerate() {
                let weight = problem.net_weights[net];
                if pins.len() < 2 || weight <= 0.0 {
                    continue;
                }
                // Where each pin is now, and its owner as the system sees
                // it (a held body's pins are fixed).
                let at: Vec<(Option<usize>, f64, f64)> = pins
                    .iter()
                    .map(|(owner, offset)| match owner {
                        Some(body) if !held[*body] => (
                            Some(*body),
                            centers[*body][axis] + offset[axis],
                            offset[axis],
                        ),
                        Some(body) => (None, centers[*body][axis] + offset[axis], 0.0),
                        None => (None, offset[axis], 0.0),
                    })
                    .collect();
                if at.iter().all(|(owner, _, _)| owner.is_none()) {
                    continue;
                }
                let (mut low, mut high) = (0, 0);
                for (index, (_, position, _)) in at.iter().enumerate() {
                    if *position < at[low].1 {
                        low = index;
                    }
                    if *position > at[high].1 {
                        high = index;
                    }
                }
                if low == high {
                    high = usize::from(low == 0);
                }
                let scale = weight * 2.0 / (at.len() as f64 - 1.0);
                let mut connect = |a: usize, b: usize| {
                    let (owner_a, position_a, offset_a) = at[a];
                    let (owner_b, position_b, offset_b) = at[b];
                    let w = scale / (position_a - position_b).abs().max(least);
                    match (owner_a, owner_b) {
                        (Some(a), Some(b)) if a == b => {}
                        (Some(a), Some(b)) => {
                            diagonal[a] += w;
                            diagonal[b] += w;
                            rhs[a] += w * (offset_b - offset_a);
                            rhs[b] += w * (offset_a - offset_b);
                            edges.push((a, b, w));
                        }
                        (Some(a), None) => {
                            diagonal[a] += w;
                            rhs[a] += w * (position_b - offset_a);
                        }
                        (None, Some(b)) => {
                            diagonal[b] += w;
                            rhs[b] += w * (position_a - offset_b);
                        }
                        (None, None) => {}
                    }
                };
                connect(low, high);
                for index in 0..at.len() {
                    if index != low && index != high {
                        connect(index, low);
                        connect(index, high);
                    }
                }
            }
            let stiff: Vec<f64> = diagonal.iter().copied().filter(|d| *d > 0.0).collect();
            let mean = if stiff.is_empty() {
                1.0
            } else {
                stiff.iter().sum::<f64>() / stiff.len() as f64
            };
            for body in 0..count {
                let (pull, target) = if held[body] {
                    (PROJECTED * mean, centers[body][axis])
                } else if let Some(target) = projected[body][axis] {
                    (PROJECTED * mean, target)
                } else {
                    (ANCHOR_SHARE * mean, anchor[body][axis])
                };
                diagonal[body] += pull;
                rhs[body] += pull * target;
            }
            for (body, center) in centers.iter().enumerate() {
                solution[body] = center[axis];
            }
            work += conjugate_gradients(&diagonal, &edges, &rhs, &mut solution);
            for (body, center) in centers.iter_mut().enumerate() {
                if !held[body] {
                    center[axis] = solution[body];
                }
            }
        }
        // Where the constraints hold a part, it stays at their projection.
        let solved: Vec<Point> = centers.to_vec();
        project(centers);
        for body in 0..count {
            for axis in 0..2 {
                if !held[body]
                    && (projected[body][axis].is_some()
                        || (centers[body][axis] - solved[body][axis]).abs() > 1.0e-9)
                {
                    projected[body][axis] = Some(centers[body][axis]);
                }
            }
        }
        let moved = centers
            .iter()
            .zip(&before)
            .map(|(a, b)| (a[0] - b[0]).abs().max((a[1] - b[1]).abs()))
            .fold(0.0, f64::max);
        if moved < SETTLED * side {
            break;
        }
    }
    work
}

/// Solves `(diag(diagonal) - offdiagonal) x = rhs` for the symmetric
/// system whose off-diagonal entries are `edges` (each `(a, b, w)` is `-w`
/// at (a, b) and (b, a)) by preconditioned conjugate gradients, from `x`.
/// Returns the work: matrix entries visited.
fn conjugate_gradients(
    diagonal: &[f64],
    edges: &[(usize, usize, f64)],
    rhs: &[f64],
    x: &mut [f64],
) -> u64 {
    let n = x.len();
    let multiply = |vector: &[f64], out: &mut [f64]| {
        for index in 0..n {
            out[index] = diagonal[index] * vector[index];
        }
        for (a, b, w) in edges {
            out[*a] -= w * vector[*b];
            out[*b] -= w * vector[*a];
        }
    };
    let entry_work = (n + 2 * edges.len()) as u64;
    let mut work = entry_work;
    let mut product = vec![0.0; n];
    multiply(x, &mut product);
    let mut residual: Vec<f64> = rhs.iter().zip(&product).map(|(b, ax)| b - ax).collect();
    let target = CG_TOLERANCE * rhs.iter().map(|b| b * b).sum::<f64>().sqrt().max(1.0e-12);
    let mut z: Vec<f64> = residual
        .iter()
        .zip(diagonal)
        .map(|(r, d)| r / d.max(1.0e-300))
        .collect();
    let mut direction = z.clone();
    let mut rz: f64 = residual.iter().zip(&z).map(|(r, z)| r * z).sum();
    for _ in 0..CG_ITERATIONS {
        if residual.iter().map(|r| r * r).sum::<f64>().sqrt() <= target {
            break;
        }
        multiply(&direction, &mut product);
        work += entry_work;
        let curvature: f64 = direction.iter().zip(&product).map(|(p, q)| p * q).sum();
        if curvature <= 0.0 {
            break;
        }
        let step = rz / curvature;
        for index in 0..n {
            x[index] += step * direction[index];
            residual[index] -= step * product[index];
        }
        for index in 0..n {
            z[index] = residual[index] / diagonal[index].max(1.0e-300);
        }
        let next: f64 = residual.iter().zip(&z).map(|(r, z)| r * z).sum();
        let blend = next / rz;
        rz = next;
        for index in 0..n {
            direction[index] = z[index] + blend * direction[index];
        }
    }
    work
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraints::Edge;
    use crate::global::{GlobalConfig, global_place};
    use crate::problem::{Component, Pin, Pose, Side};

    #[test]
    fn conjugate_gradients_solve_a_chain() {
        // x0 - x1 chain anchored at both ends: 2 x0 - x1 = 0, -x0 + 2 x1 = 3.
        let diagonal = [2.0, 2.0];
        let edges = [(0, 1, 1.0)];
        let mut x = [0.0, 0.0];
        conjugate_gradients(&diagonal, &edges, &[0.0, 3.0], &mut x);
        assert!(
            (x[0] - 1.0).abs() < 1.0e-9 && (x[1] - 2.0).abs() < 1.0e-9,
            "{x:?}"
        );
    }

    /// A part on nets `net` (left pin) and `net + 1` (right pin).
    fn part(net: usize, fixed: bool) -> Component {
        Component {
            name: format!("P{net}"),
            body_center: [0.0, 0.0],
            body_size: [2.0, 1.0],
            round: false,
            halo: 0.0,
            pins: vec![
                Pin {
                    offset: [-0.6, 0.0],
                    net,
                },
                Pin {
                    offset: [0.6, 0.0],
                    net: net + 1,
                },
            ],
            side: Side::Front,
            fixed,
            angle_options: vec![0.0],
            far_side: Vec::new(),
            far_copper: Vec::new(),
            far_open: Default::default(),
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
            hole_rings: Vec::new(),
            copper_graphics: Vec::new(),
            courtless: Vec::new(),
            slots: Vec::new(),
        }
    }

    /// A chain of six movable parts between a fixed part at x 5 and one at
    /// x 35 on a 40 x 30 mm board, the movable ones listed in a shuffled
    /// order (component `order[k]` is the k-th link of the chain) and
    /// stacked at the centre.
    fn chain() -> (Problem, Vec<usize>) {
        let order = vec![4, 1, 6, 2, 5, 3];
        let mut components = vec![part(0, true)];
        let mut link = vec![0; 7];
        for (index, k) in order.iter().enumerate() {
            link[*k] = index + 1;
        }
        for index in 1..=6 {
            let k = order[index - 1];
            components.push(part(k, false));
            debug_assert_eq!(link[k], index);
        }
        components.push(part(7, true));
        let mut poses = vec![
            Pose {
                position: [20.0, 15.0],
                angle: 0.0,
            };
            8
        ];
        poses[0].position = [5.0, 15.0];
        poses[7].position = [35.0, 15.0];
        let problem = Problem {
            outline: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]],
            components,
            net_weights: vec![1.0; 9],
            poses,
            spacing: 0.5,
            grid: 0.25,
            edge_margin: 0.5,
            min_spacing: 0.2,
            far_side_pads_only: false,
            escape: None,
            crossings: None,
            pieces: Vec::new(),
            constraints: Default::default(),
        };
        // The movable components in the chain's order.
        let links = (1..=6).map(|k| link[k]).collect();
        (problem, links)
    }

    /// `quadratic_start` on `problem` from the common start (20, 15), the
    /// held bodies at `held_at`, with `project`; the solved centres per
    /// component.
    fn solve(
        problem: &Problem,
        held_at: &[(usize, Point)],
        project: impl FnMut(&mut [Point]),
    ) -> Vec<Point> {
        let movable: Vec<usize> = (0..problem.components.len())
            .filter(|index| !problem.components[*index].fixed)
            .collect();
        let offsets: Vec<Vec<(Point, usize)>> = movable
            .iter()
            .map(|index| {
                let component = &problem.components[*index];
                component
                    .pins
                    .iter()
                    .map(|pin| {
                        (
                            component.offset(
                                [
                                    pin.offset[0] - component.body_center[0],
                                    pin.offset[1] - component.body_center[1],
                                ],
                                problem.poses[*index].angle,
                            ),
                            pin.net,
                        )
                    })
                    .collect()
            })
            .collect();
        let fixed_pins: Vec<(Point, usize)> = problem
            .components
            .iter()
            .enumerate()
            .filter(|(_, component)| component.fixed)
            .flat_map(|(index, component)| {
                component
                    .pins
                    .iter()
                    .map(move |pin| (component.pin_position(pin, problem.poses[index]), pin.net))
            })
            .collect();
        let anchor = vec![[20.0, 15.0]; movable.len()];
        let mut held = vec![false; movable.len()];
        let mut centers = anchor.clone();
        for (index, at) in held_at {
            let body = movable.iter().position(|m| m == index).unwrap();
            held[body] = true;
            centers[body] = *at;
        }
        quadratic_start(
            problem,
            &movable,
            &offsets,
            &fixed_pins,
            &anchor,
            &held,
            &mut centers,
            project,
        );
        let mut result = vec![[f64::NAN; 2]; problem.components.len()];
        for (body, index) in movable.iter().enumerate() {
            result[*index] = centers[body];
        }
        result
    }

    #[test]
    fn the_netlist_orders_a_chain_between_its_anchors() {
        let (problem, links) = chain();
        let centers = solve(&problem, &[], |_| {});
        let xs: Vec<f64> = links.iter().map(|index| centers[*index][0]).collect();
        assert!(xs.windows(2).all(|pair| pair[1] > pair[0] + 1.0), "{xs:?}");
        assert!(xs[0] > 5.0 && xs[5] < 35.0, "{xs:?}");
        // Evenly spaced: every gap within 10 % of the mean gap.
        let gap = (xs[5] - xs[0]) / 5.0;
        assert!(
            xs.windows(2)
                .all(|pair| ((pair[1] - pair[0]) - gap).abs() < 0.1 * gap),
            "{xs:?}"
        );
        for index in &links {
            assert!((centers[*index][1] - 15.0).abs() < 1.0e-6, "{centers:?}");
        }
    }

    #[test]
    fn a_projected_part_holds_its_neighbours_near_its_edge() {
        let (problem, links) = chain();
        // The third link held at y 2 (as an edge constraint's clamp
        // would): the solves after the first anchor it there.
        let third = links[2];
        let movable: Vec<usize> = (1..=6).collect();
        let body = movable.iter().position(|m| *m == third).unwrap();
        let centers = solve(&problem, &[], |centers: &mut [Point]| {
            centers[body][1] = 2.0;
        });
        assert!((centers[third][1] - 2.0).abs() < 1.0e-9);
        // The half perimeters are indifferent to where between the edge
        // and the fixed parts' line a neighbour lies; the bound-to-bound
        // weights draw it towards the edge from where it was.
        for neighbour in [links[1], links[3]] {
            let y = centers[neighbour][1];
            assert!(y > 2.0 && y < 14.9, "{centers:?}");
        }
        // Farther links follow less.
        assert!(centers[links[1]][1] < centers[links[0]][1]);
        assert!(centers[links[3]][1] < centers[links[4]][1]);
    }

    #[test]
    fn a_held_part_stays_and_pulls() {
        let (problem, links) = chain();
        let first = links[0];
        let centers = solve(&problem, &[(first, [10.0, 25.0])], |_| {});
        assert_eq!(centers[first], [10.0, 25.0]);
        assert!(centers[links[1]][1] > 15.5, "{centers:?}");
    }

    #[test]
    fn the_global_placement_starts_at_the_chains_order_on_every_seed() {
        let (mut problem, links) = chain();
        // The fourth link kept at the top edge: the start puts it there.
        problem.constraints.edges.push((links[3], Edge::Top, 0.5));
        let start = |seed: u64| {
            let config = GlobalConfig {
                netlist_start: true,
                seed,
                ..GlobalConfig::default()
            };
            global_place(&problem, &config)
        };
        let one = start(1);
        let again = start(1);
        assert_eq!(one.start, again.start, "deterministic");
        assert_eq!(one.poses, again.poses, "deterministic");
        for seed in [1, 2, 99] {
            let result = start(seed);
            let xs: Vec<f64> = links
                .iter()
                .map(|index| problem.components[*index].center(result.start[*index])[0])
                .collect();
            assert!(
                xs.windows(2).all(|pair| pair[1] > pair[0]),
                "seed {seed}: {xs:?}"
            );
            let top = problem.components[links[3]].center(result.start[links[3]])[1];
            assert!(top < 3.0, "seed {seed}: the edge part at y {top}");
        }
        // Without it the start is the common point and its noise.
        let plain = global_place(&problem, &GlobalConfig::default());
        let spread = links
            .iter()
            .map(|index| problem.components[*index].center(plain.start[*index])[0])
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), x| {
                (low.min(x), high.max(x))
            });
        assert!(spread.1 - spread.0 < 0.05 * 40.0 + 1.0e-9, "{spread:?}");
    }
}
