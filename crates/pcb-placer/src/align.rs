// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Rows of repeated parts (an experiment, off by default; docs/placer.md,
//! "The designer as teacher (2026-11-04)").
//!
//! On the fifteen boards where the designer's placement routes best against
//! ours, the designer lines repeated parts up: of the moved parts of one
//! footprint, 0.6 to 0.95 share an x or a y with another of their kind
//! (ours 0.2 to 0.66, lower on 13 of 15 boards), and they stand turned
//! alike (lower on 12). This term prices a part that is not lined up with
//! one of its kind nearby.
//!
//! A part's *kind* is its body size, pin count and side (as
//! `legal::align_orientations` groups them). Its cost is
//! `min(cap, min over its kind's other parts j within reach of a(i, j))`,
//! where `a(i, j)` is the smaller of the two body centres' offsets in x
//! and y, plus `cap` where the two stand turned differently (two-pin parts
//! modulo half a turn). A part alone in its kind costs nothing; one with
//! others of its kind but none lined up within `reach` costs `cap`. The
//! board's energy is `weight` times the sum over the parts of kinds of two
//! or more (fixed ones too: they are anchors). A move of a part changes
//! its own cost and that of its kind's parts within `reach` of its old or
//! new centre, so `change` is exact.

use crate::problem::{Pose, Problem};

/// The term's settings (`Config::align`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Millimetres of wirelength one millimetre of misalignment costs.
    pub weight: f64,
    /// The most a part costs (millimetres): a part whose nearest lined-up
    /// partner is farther off counts as not lined up.
    pub cap: f64,
    /// Parts of a kind whose body centres lie farther apart than this
    /// (millimetres), or than one and a half times the kind's larger body
    /// side where that is more, do not line up with each other.
    pub reach: f64,
    /// After the refinement, parts move up to `cap` along one axis to line
    /// up with one of their kind where that stays legal and costs at most
    /// half a millimetre of wire (`legal::align_rows`).
    pub finish: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            weight: 1.0,
            cap: 1.0,
            reach: 5.0,
            finish: true,
        }
    }
}

/// Offsets below this count as lined up (millimetres).
pub const ALIGNED: f64 = 1.0e-3;

/// The kinds of a problem: which parts line up with which.
#[derive(Clone, Debug)]
pub struct Align {
    pub settings: Settings,
    /// Per part, its kind (an index into `kinds`) where the kind has two
    /// parts or more and one of them moves.
    pub kind: Vec<Option<usize>>,
    pub kinds: Vec<Vec<usize>>,
    /// Per kind, the reach squared (`Settings::reach`, or one and a half
    /// times the kind's larger body side).
    pub reach2: Vec<f64>,
}

/// The work of one part of a kind looked at (its centre against the
/// part's): booked as two units, as measured against the anneal's own
/// (the 4in1's anneal took 2.2 times the CPU at 1.6 times the work when
/// booked as one).
const MEMBER_WORK: u64 = 2;

/// Two-pin parts look alike half a turn apart.
fn angle_key(problem: &Problem, index: usize, angle: f64) -> i64 {
    let modulo = if problem.components[index].pins.len() == 2 {
        180.0
    } else {
        360.0
    };
    (angle.rem_euclid(modulo).round() as i64) % modulo as i64
}

impl Align {
    pub fn new(problem: &Problem, settings: Settings) -> Self {
        let mut by: std::collections::BTreeMap<(i64, i64, usize, u8), Vec<usize>> =
            Default::default();
        for (index, component) in problem.components.iter().enumerate() {
            if component.pins.len() < 2 {
                continue;
            }
            by.entry((
                (component.body_size[0] * 100.0).round() as i64,
                (component.body_size[1] * 100.0).round() as i64,
                component.pins.len(),
                component.side as u8,
            ))
            .or_default()
            .push(index);
        }
        let mut kind = vec![None; problem.components.len()];
        let mut kinds = Vec::new();
        let mut reach2 = Vec::new();
        for (key, members) in by {
            if members.len() < 2 || members.iter().all(|m| problem.components[*m].fixed) {
                continue;
            }
            for member in &members {
                kind[*member] = Some(kinds.len());
            }
            kinds.push(members);
            let side = key.0.max(key.1) as f64 / 100.0;
            let reach = settings.reach.max(1.5 * side);
            reach2.push(reach * reach);
        }
        Self {
            settings,
            kind,
            kinds,
            reach2,
        }
    }

    /// Whether any part moves with a kind to line up with.
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    /// Part `index`'s cost at `poses` (unweighted, millimetres), counting
    /// its work (one unit per part of its kind looked at).
    pub fn part_cost(&self, problem: &Problem, poses: &[Pose], index: usize) -> f64 {
        self.cost_with(index, |part| {
            (
                problem.components[part].center(poses[part]),
                angle_key(problem, part, poses[part].angle),
            )
        })
    }

    /// Part `index`'s cost with each part's body centre and angle key from
    /// `at`.
    fn cost_with(&self, index: usize, at: impl Fn(usize) -> ([f64; 2], i64)) -> f64 {
        let Some(kind) = self.kind[index] else {
            return 0.0;
        };
        let members = &self.kinds[kind];
        crate::add_work(MEMBER_WORK * members.len() as u64);
        let cap = self.settings.cap;
        let reach = self.reach2[kind];
        let (center, key) = at(index);
        let mut best = cap;
        for &other in members {
            if other == index {
                continue;
            }
            let (there, other_key) = at(other);
            let dx = (there[0] - center[0]).abs();
            let dy = (there[1] - center[1]).abs();
            if dx * dx + dy * dy > reach {
                continue;
            }
            let mut offset = dx.min(dy);
            if offset < ALIGNED {
                offset = 0.0;
            }
            if other_key != key {
                offset += cap;
            }
            best = best.min(offset);
            if best == 0.0 {
                break;
            }
        }
        best
    }

    /// The parts whose cost a move of `index` from `from` (its old body
    /// centre) to its pose in `poses` can change: itself and its kind's
    /// parts within reach of either centre. Appends to `into`.
    pub fn affected(
        &self,
        problem: &Problem,
        poses: &[Pose],
        index: usize,
        from: [f64; 2],
        into: &mut Vec<usize>,
    ) {
        let Some(kind) = self.kind[index] else {
            return;
        };
        let members = &self.kinds[kind];
        crate::add_work(MEMBER_WORK * members.len() as u64);
        let to = problem.components[index].center(poses[index]);
        let reach = self.reach2[kind];
        let near = |a: [f64; 2], b: [f64; 2]| {
            let dx = a[0] - b[0];
            let dy = a[1] - b[1];
            dx * dx + dy * dy <= reach
        };
        for &other in members {
            if other == index {
                into.push(other);
                continue;
            }
            let there = problem.components[other].center(poses[other]);
            if near(there, from) || near(there, to) {
                into.push(other);
            }
        }
    }

    /// The weighted sum of the costs of `parts` (each once) at `poses`.
    pub fn sum(&self, problem: &Problem, poses: &[Pose], parts: &[usize]) -> f64 {
        self.settings.weight
            * parts
                .iter()
                .map(|part| self.part_cost(problem, poses, *part))
                .sum::<f64>()
    }

    /// The board's energy at `poses` (weighted).
    pub fn total(&self, problem: &Problem, poses: &[Pose]) -> f64 {
        let parts: Vec<usize> = self.kinds.iter().flatten().copied().collect();
        self.sum(problem, poses, &parts)
    }

    /// The share of the movable parts of kinds of two or more whose cost
    /// is 0 (lined up with one of their kind, turned alike).
    pub fn lined_up(&self, problem: &Problem, poses: &[Pose]) -> Option<f64> {
        let movable: Vec<usize> = self
            .kinds
            .iter()
            .flatten()
            .copied()
            .filter(|part| !problem.components[*part].fixed)
            .collect();
        if movable.is_empty() {
            return None;
        }
        let counted = crate::work();
        let zero = movable
            .iter()
            .filter(|part| self.part_cost(problem, poses, **part) == 0.0)
            .count();
        crate::add_work_back(crate::work() - counted);
        Some(zero as f64 / movable.len() as f64)
    }
}

/// The anneal's view of the term: each part's weighted cost, body centre
/// and angle key at the poses kept, and the change of a move.
pub(crate) struct AlignState<'a> {
    pub align: &'a Align,
    /// Per part, its weighted cost at the poses kept.
    costs: Vec<f64>,
    /// Per part, its body centre and angle key at the poses kept.
    at: Vec<([f64; 2], i64)>,
    /// The parts a move touched and their costs after it (`change`), and
    /// the moved parts' new centres and keys, for `keep`.
    touched: Vec<usize>,
    after: Vec<f64>,
    moved: Vec<(usize, ([f64; 2], i64))>,
}

impl<'a> AlignState<'a> {
    pub fn new(problem: &Problem, align: &'a Align, poses: &[Pose]) -> Self {
        let mut state = Self {
            align,
            costs: vec![0.0; problem.components.len()],
            at: Vec::new(),
            touched: Vec::new(),
            after: Vec::new(),
            moved: Vec::new(),
        };
        state.refresh(problem, poses);
        state
    }

    /// The energy's change when the parts `moved` went from the poses kept
    /// to their poses in `poses`; the new costs are kept aside for `keep`.
    pub fn change(&mut self, problem: &Problem, poses: &[Pose], moved: &[usize]) -> f64 {
        self.touched.clear();
        self.moved.clear();
        for &part in moved {
            let Some(kind) = self.align.kind[part] else {
                continue;
            };
            let reach = self.align.reach2[kind];
            let new = (
                problem.components[part].center(poses[part]),
                angle_key(problem, part, poses[part].angle),
            );
            self.moved.push((part, new));
            let from = self.at[part].0;
            let members = &self.align.kinds[kind];
            crate::add_work(MEMBER_WORK * members.len() as u64);
            let near = |a: [f64; 2], b: [f64; 2]| {
                let dx = a[0] - b[0];
                let dy = a[1] - b[1];
                dx * dx + dy * dy <= reach
            };
            for &other in members {
                let there = self.at[other].0;
                if other == part || near(there, from) || near(there, new.0) {
                    self.touched.push(other);
                }
            }
        }
        if self.touched.is_empty() {
            return 0.0;
        }
        self.touched.sort_unstable();
        self.touched.dedup();
        let weight = self.align.settings.weight;
        self.after.clear();
        let mut change = 0.0;
        for &part in &self.touched {
            let (at, moved) = (&self.at, &self.moved);
            let cost = weight
                * self.align.cost_with(part, |other| {
                    moved
                        .iter()
                        .find(|(m, _)| *m == other)
                        .map(|(_, new)| *new)
                        .unwrap_or(at[other])
                });
            change += cost - self.costs[part];
            self.after.push(cost);
        }
        change
    }

    /// Takes the costs, centres and keys of the last `change` (its move
    /// was kept).
    pub fn keep(&mut self) {
        for (part, cost) in self.touched.iter().zip(&self.after) {
            self.costs[*part] = *cost;
        }
        for (part, new) in &self.moved {
            self.at[*part] = *new;
        }
    }

    /// Every part's cost, centre and key again (the start, moves made
    /// outside the single moves).
    pub fn refresh(&mut self, problem: &Problem, poses: &[Pose]) {
        self.at = (0..problem.components.len())
            .map(|part| {
                (
                    problem.components[part].center(poses[part]),
                    angle_key(problem, part, poses[part].angle),
                )
            })
            .collect();
        let weight = self.align.settings.weight;
        for part in 0..self.costs.len() {
            let at = &self.at;
            self.costs[part] = weight * self.align.cost_with(part, |other| at[other]);
        }
    }

    /// The energy at the poses kept.
    pub fn total(&self) -> f64 {
        self.costs.iter().sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::problem::{Component, Pin, Side};

    fn part(net: usize, fixed: bool) -> Component {
        Component {
            name: String::new(),
            body_center: [0.2, 0.0],
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
            angle_options: vec![0.0, 90.0, 180.0, 270.0],
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
            hole_rings: Vec::new(),
            copper_graphics: Vec::new(),
        }
    }

    #[test]
    fn the_anneals_change_is_the_energys_change() {
        let components: Vec<Component> = (0..10).map(|i| part(2 * i, i >= 8)).collect();
        let poses: Vec<Pose> = (0..10)
            .map(|i| Pose {
                position: [4.0 + 3.0 * (i % 4) as f64, 4.0 + 3.0 * (i / 4) as f64],
                angle: 0.0,
            })
            .collect();
        let problem = Problem {
            outline: vec![[0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [0.0, 20.0]],
            components,
            net_weights: vec![1.0; 20],
            poses: poses.clone(),
            spacing: 0.5,
            grid: 0.25,
            edge_margin: 0.5,
            min_spacing: 0.2,
            far_side_pads_only: false,
            shared_halos: false,
            escape: None,
            crossings: None,
            pieces: Vec::new(),
            constraints: Default::default(),
        };
        let align = Align::new(
            &problem,
            Settings {
                weight: 1.5,
                reach: 5.0,
                ..Settings::default()
            },
        );
        let mut state = AlignState::new(&problem, &align, &poses);
        let mut poses = poses;
        let mut seed = 99u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        for _ in 0..400 {
            let first = ((next() * 8.0) as usize).min(7);
            let second = ((next() * 8.0) as usize).min(7);
            let moved: Vec<usize> = if next() < 0.3 && first != second {
                vec![first, second]
            } else {
                vec![first]
            };
            let saved = poses.clone();
            for &part in &moved {
                poses[part] = if next() < 0.25 {
                    Pose {
                        angle: [0.0, 90.0, 180.0, 270.0][((next() * 4.0) as usize).min(3)],
                        ..poses[part]
                    }
                } else {
                    Pose {
                        position: [
                            ((poses[part].position[0] + (next() - 0.5) * 6.0) * 4.0).round() / 4.0,
                            ((poses[part].position[1] + (next() - 0.5) * 6.0) * 4.0).round() / 4.0,
                        ],
                        angle: poses[part].angle,
                    }
                };
            }
            let before = align.total(&problem, &saved);
            let after = align.total(&problem, &poses);
            let change = state.change(&problem, &poses, &moved);
            assert!(
                (change - (after - before)).abs() < 1e-9,
                "change {change}, energy {before} -> {after}"
            );
            if next() < 0.6 {
                state.keep();
                assert!((state.total() - after).abs() < 1e-9);
            } else {
                poses = saved;
            }
        }
    }
}
