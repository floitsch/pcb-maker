// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The anneal's moves: a part drawn (`Run::step`), then a swap with
//! another (`Run::swap`) or a turn or shift (`Run::propose`) judged as a
//! single move (`Run::try_move`), each taken or undone by the acceptance
//! rule (`Run::rejected`). Every draw of a random number stays where it
//! was: the anneal is seeded, and one draw moved changes every placement
//! after it.

use super::run::Run;
use super::{PEER_SWAPS, Random, judge_with_crossings};
use crate::constraints;
use crate::problem::Pose;

impl Run<'_> {
    /// One move: a part drawn (by the guidance's attribution where it has
    /// one, else at random), then a swap of it, a turn or a shift, judged.
    /// `done` is the anneal's work so far, this stage's moves included.
    /// Returns the move's work besides what the overlap sums book.
    pub(super) fn step(&mut self, done: u64) -> u64 {
        self.tally.moves += 1;
        if let Some(guide) = self.guide.as_mut()
            && guide.due(done + self.state.work.get())
        {
            guide.evaluate(&mut self.state, &self.movable);
        }
        let index = match self
            .guide
            .as_ref()
            .and_then(|guide| guide.cumulative.as_ref())
        {
            Some(cumulative) if self.random.next() < self.guide.as_ref().unwrap().bias => {
                self.movable[crate::guidance::pick(cumulative, self.random.next())]
            }
            _ => self.movable[self.random.below(self.movable.len())],
        };
        // A bank's members move with their bank (`Settings::rigid_banks`).
        if self.rigid.get(index) == Some(&true) {
            return 0;
        }
        let work = self.move_work[index];
        let kind = self.random.next();
        if kind < 0.12 && self.movable.len() > 1 {
            return work + self.swap(index);
        }
        if let Some(new) = self.propose(index, kind) {
            self.try_move(index, new);
        }
        work
    }

    /// A swap of part `index` with another (with the crossings term one of
    /// its `peers` half the time, `PEER_SWAPS`), each at the other's body
    /// centre, judged. Returns the other part's move work (0 where the draw
    /// gave no other part).
    fn swap(&mut self, index: usize) -> u64 {
        let problem = self.state.problem;
        let component = &problem.components[index];
        // Swap two parts, keeping each at the other's body centre.
        let other = match self.peers.get(index) {
            Some(like) if !like.is_empty() && self.random.next() < PEER_SWAPS => {
                like[self.random.below(like.len())]
            }
            _ => self.movable[self.random.below(self.movable.len())],
        };
        if other == index || self.rigid.get(other) == Some(&true) {
            return 0;
        }
        let work = self.move_work[other];
        let penalty = self.schedule.penalty;
        let state = &mut self.state;
        let second = &problem.components[other];
        let (first_pose, second_pose) = (state.poses[index], state.poses[other]);
        let first_new = Pose {
            position: component.position_for_center(second.center(second_pose), first_pose.angle),
            angle: first_pose.angle,
        };
        let second_new = Pose {
            position: second.position_for_center(component.center(first_pose), second_pose.angle),
            angle: second_pose.angle,
        };
        let snapped = |part: usize, pose: Pose| Pose {
            position: constraints::snap_position(problem, part, pose.position, pose.angle),
            angle: pose.angle,
        };
        let (first_new, second_new) = (snapped(index, first_new), snapped(other, second_new));
        let mut nets = state.incident[index].clone();
        nets.extend(&state.incident[other]);
        nets.sort_unstable();
        nets.dedup();
        let before = state.cached_wirelength(&nets)
            + state.constraint_cost(index)
            + state.constraint_cost(other)
            + penalty
                * (state.overlap(index, state.rects[index])
                    + state.overlap(other, state.rects[other]));
        let saved = (state.rects[index], state.rects[other]);
        state.poses[index] = first_new;
        state.poses[other] = second_new;
        let (first_rect, second_rect) =
            (state.rect(index, first_new), state.rect(other, second_new));
        state.set_rect(index, first_rect);
        state.set_rect(other, second_rect);
        let after = state.wirelength_terms(&nets, &mut self.terms)
            + state.constraint_cost(index)
            + state.constraint_cost(other)
            + penalty
                * (state.overlap(index, state.rects[index])
                    + state.overlap(other, state.rects[other]));
        let delta = after - before;
        if self.rejected(&[index, other], delta) {
            let state = &mut self.state;
            state.poses[index] = first_pose;
            state.poses[other] = second_pose;
            state.set_rect(index, saved.0);
            state.set_rect(other, saved.1);
        } else {
            self.state.keep_lengths(&nets, &self.terms);
            if let Some(guide) = self.guide.as_mut() {
                guide.accepted += 1;
            }
        }
        work
    }

    /// A turn of part `index` (where `kind` says so and it has angles to
    /// turn to: a random one about its body centre, snapped) or else a
    /// shift by up to the stage's radius on each axis, in grid steps.
    /// `None` where the draw leaves the part where it is.
    fn propose(&mut self, index: usize, kind: f64) -> Option<Pose> {
        let problem = self.state.problem;
        let component = &problem.components[index];
        let old = self.state.poses[index];
        if kind < 0.27 && component.angle_options.len() > 1 {
            let angle = component.angle_options[self.random.below(component.angle_options.len())];
            if (angle - old.angle).abs() < 1.0e-9 {
                return None;
            }
            let position = component.position_for_center(component.center(old), angle);
            Some(Pose {
                position: constraints::snap_position(problem, index, position, angle),
                angle,
            })
        } else {
            let (radius, grid) = (self.schedule.radius, self.schedule.grid);
            let step = |random: &mut Random| {
                let cells = (radius / grid).max(1.0);
                ((random.next() * 2.0 - 1.0) * cells).round() * grid
            };
            let position = [
                old.position[0] + step(&mut self.random),
                old.position[1] + step(&mut self.random),
            ];
            if position == old.position {
                return None;
            }
            Some(Pose {
                position,
                angle: old.angle,
            })
        }
    }

    /// Part `index` to `new`, judged by its nets' wirelength, its
    /// constraints and its overlap at the stage's penalty; undone where
    /// rejected.
    fn try_move(&mut self, index: usize, new: Pose) {
        let penalty = self.schedule.penalty;
        let state = &mut self.state;
        let old = state.poses[index];
        let before = state.cached_wirelength(&state.incident[index])
            + state.constraint_cost(index)
            + penalty * state.overlap(index, state.rects[index]);
        let saved = state.rects[index];
        state.poses[index] = new;
        let new_rect = state.rect(index, new);
        state.set_rect(index, new_rect);
        let after = state.wirelength_terms(&state.incident[index], &mut self.terms)
            + state.constraint_cost(index)
            + penalty * state.overlap(index, state.rects[index]);
        let delta = after - before;
        if self.rejected(&[index], delta) {
            self.state.poses[index] = old;
            self.state.set_rect(index, saved);
        } else {
            // The nets of a part are its own list: no copy of it.
            let state = &mut self.state;
            let incident = std::mem::take(&mut state.incident[index]);
            state.keep_lengths(&incident, &self.terms);
            state.incident[index] = incident;
            if let Some(guide) = self.guide.as_mut() {
                guide.accepted += 1;
            }
        }
    }

    /// Whether the move of `moved` (at their new poses in the state), whose
    /// other terms change the cost by `delta`, is rejected: with the
    /// crossings term by `judge_with_crossings`, else by the plain rule at
    /// the stage's temperature (taken when the cost falls, else with
    /// probability exp(-delta / temperature), a number drawn only then).
    fn rejected(&mut self, moved: &[usize], delta: f64) -> bool {
        let temperature = self.schedule.temperature;
        match self.crossings.as_mut() {
            Some(crossings) => !judge_with_crossings(
                crossings,
                &self.state.poses,
                moved,
                delta,
                temperature,
                &mut self.random,
                &mut self.tally,
            ),
            None => delta > 0.0 && self.random.next() >= (-delta / temperature).exp(),
        }
    }
}
