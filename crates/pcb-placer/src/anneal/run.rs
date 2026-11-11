// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The anneal's run: what a move costs (`move_work`), the state the moves
//! update (`Run`, made by `Run::new`), the schedule of the overlap
//! penalty, the move radius and the temperature (`Schedule`), the stages
//! (`Run::stages`: the moves of moves.rs, the bus groups' batch, the work
//! booked, the crossings term's end) and the debug lines at the end
//! (`Run::finish`).

use super::{
    AnnealConfig, GroupStep, GroupTally, Random, State, TERM_SHARE, Tally, group_batch, peers,
};
use crate::constraints;
use crate::crossings::{CrossingState, Crossings};
use crate::problem::{Pose, Problem};

/// Per part, a move's work besides the parts the overlap test visits
/// (counted there).
pub(super) fn move_work(problem: &Problem, state: &State) -> Vec<u64> {
    // Work per move (before and after), besides the parts the overlap
    // test visits (counted there): when the outline is not a rectangle,
    // four corners against it (cheap per edge next to a part's visit);
    // the wirelength visits the part's pins' nets; the constraint cost
    // scans the relations. Fitted so that boards as different as katia,
    // link and OpenESC's do about the same work per second.
    // The hard constraints' test scans every edge and region entry for
    // the part's own (a panel's copies put a region on every part).
    let held = &problem.constraints;
    // Finding a part's relations scans every relation, and every anchor of
    // a decoupling capacitor's (glasgow's 82 relations, 23 anchors each).
    let scan: usize = held
        .relations
        .iter()
        .map(|relation| match relation {
            constraints::Relation::NearAny { anchors, .. } => 1 + anchors.len() / 16,
            _ => 1,
        })
        .sum();
    let fixed =
        scan + if state.rectangular {
            0
        } else {
            problem.outline.len()
        } + if held.is_empty() {
            0
        } else {
            (held.edges.len() + held.regions.len()) / 2
        };
    // The relations a part is in are evaluated before and after its move.
    (0..problem.components.len())
        .map(|index| {
            let pins: usize = state.incident[index]
                .iter()
                .map(|net| state.members[*net].len())
                .sum();
            2 * (fixed as u64 + crate::PIN_WORK * pins as u64)
                + 2 * constraints::relation_work(problem, index)
        })
        .collect()
}

/// The debug line on what the anneal works on (`PCB_PLACER_DEBUG`).
pub(super) fn describe(problem: &Problem, state: &State, movable: &[usize], move_work: &[u64]) {
    let average = move_work.iter().sum::<u64>() as f64 / move_work.len().max(1) as f64;
    eprintln!(
        "anneal: {} parts ({} movable), outline {} ({}), relations {}, linked {}, far-side parts {}, hollow parts {}, move work {average:.0}",
        problem.components.len(),
        movable.len(),
        problem.outline.len(),
        if state.rectangular {
            "rectangular"
        } else {
            "shaped"
        },
        problem.constraints.relations.len(),
        problem.constraints.linked.len(),
        problem
            .components
            .iter()
            .filter(|component| !component.far_side.is_empty())
            .count(),
        problem
            .components
            .iter()
            .filter(|component| !component.hollow.is_empty())
            .count(),
    );
}

/// `crossings` counting only the nets of up to `most` pins (`members`:
/// each net's pins).
pub(super) fn counted_up_to(
    crossings: &Crossings,
    members: &[Vec<(usize, usize)>],
    most: usize,
) -> Crossings {
    let mut fact = crossings.clone();
    for (net, counted) in fact.counted.iter_mut().enumerate() {
        if members.get(net).is_some_and(|pins| pins.len() > most) {
            *counted = false;
        }
    }
    fact
}

/// The anneal's schedule: the stage's move radius, overlap penalty and
/// temperature, and their factors from stage to stage.
pub(super) struct Schedule {
    /// The largest step of a shift, mm.
    pub(super) radius: f64,
    /// Millimetres of wirelength one square millimetre of overlap is worth.
    pub(super) penalty: f64,
    pub(super) temperature: f64,
    /// The grid the shifts step on (0.05 mm on a problem without one).
    pub(super) grid: f64,
    penalty_growth: f64,
    radius_decay: f64,
}

impl Schedule {
    /// The first stage's: the radius `initial_radius` of the board's larger
    /// side (two grid steps at least), the penalty 0.5 growing to 400 and
    /// the radius falling to a grid step and a half over the stages.
    pub(super) fn new(problem: &Problem, bounds: [f64; 4], config: &AnnealConfig) -> Self {
        let board = (bounds[2] - bounds[0]).max(bounds[3] - bounds[1]);
        let grid = if problem.grid > 0.0 {
            problem.grid
        } else {
            0.05
        };
        let radius = (config.initial_radius * board).max(2.0 * grid);
        // Millimetres of wirelength one square millimetre of overlap is worth.
        let penalty = 0.5;
        let temperature = 0.3;
        let penalty_growth = (400.0f64 / penalty).powf(1.0 / config.stages as f64);
        let radius_decay = ((1.5 * grid) / radius).powf(1.0 / config.stages as f64);
        Schedule {
            radius,
            penalty,
            temperature,
            grid,
            penalty_growth,
            radius_decay,
        }
    }

    /// The next stage's: the penalty grows, the radius and the temperature
    /// fall.
    pub(super) fn cool(&mut self) {
        self.penalty *= self.penalty_growth;
        self.radius *= self.radius_decay;
        self.temperature *= 0.975;
    }
}

/// What the anneal's moves read and update: the state, the parts and their
/// move work, the random numbers, the crossings term, the bus groups and
/// the schedule.
pub(super) struct Run<'a> {
    pub(super) state: State<'a>,
    pub(super) movable: Vec<usize>,
    pub(super) move_work: Vec<u64>,
    /// Per part, the parts a swap trades it with half the time (`peers`;
    /// none without the crossings term).
    pub(super) peers: Vec<Vec<usize>>,
    /// The parts that move only with their bank (`Settings::rigid_banks`).
    pub(super) rigid: Vec<bool>,
    pub(super) random: Random,
    /// The crossings term (`Crossings::anneal`), until its work reached
    /// its share (`TERM_SHARE`).
    pub(super) crossings: Option<CrossingState<'a>>,
    pub(super) tally: Tally,
    /// A move's nets' wirelength terms after it (`wirelength_terms`).
    pub(super) terms: Vec<f64>,
    pub(super) schedule: Schedule,
    /// Bus-group moves: the groups, their own crossings state (the nets of
    /// up to the groups' bus size, brought to the poses before each batch)
    /// and their counts.
    groups: Option<&'a crate::groups::BusGroups>,
    group_crossings: Option<CrossingState<'a>>,
    group_tally: GroupTally,
    /// The anneal's work so far (its stages').
    anneal_work: u64,
    /// The thread's work when the moves began (the debug line's).
    work_before: u64,
}

impl<'a> Run<'a> {
    /// The run at its start: `fact` the crossings term's count (the nets of
    /// up to `ANNEAL_NET_PINS` pins), `groups` the bus groups that move in
    /// the anneal with `group_fact` their count.
    pub(super) fn new(
        config: &'a AnnealConfig,
        state: State<'a>,
        movable: Vec<usize>,
        move_work: Vec<u64>,
        fact: Option<&'a Crossings>,
        groups: Option<&'a crate::groups::BusGroups>,
        group_fact: Option<&'a Crossings>,
    ) -> Self {
        let problem = state.problem;
        // Ratsnest crossings in the anneal (`Crossings::anneal`): a move pays
        // for the crossings of its parts' nets, and half the swaps trade a
        // part with one like it.
        let crossings = fact.map(|fact| CrossingState::new(problem, fact, &state.poses));
        let peers = if crossings.is_some() {
            peers(problem, &movable)
        } else {
            Vec::new()
        };
        let tally = Tally {
            crossings_before: crossings.as_ref().map(|state| state.total()),
            ..Default::default()
        };
        let work_before = crate::work();
        let random = Random(config.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let schedule = Schedule::new(problem, state.bounds, config);
        let group_crossings =
            group_fact.map(|fact| CrossingState::new(problem, fact, &state.poses));
        let rigid: Vec<bool> = match groups.filter(|groups| groups.settings.rigid_banks) {
            Some(groups) => {
                let mut rigid = vec![false; problem.components.len()];
                for group in &groups.groups {
                    if group.kind == crate::groups::Kind::Bank {
                        for &part in &group.chain {
                            rigid[part] = true;
                        }
                    }
                }
                rigid
            }
            None => Vec::new(),
        };
        let group_tally = GroupTally {
            crossings_before: group_crossings.as_ref().map(|state| state.total()),
            ..Default::default()
        };
        Run {
            state,
            movable,
            move_work,
            peers,
            rigid,
            random,
            crossings,
            tally,
            terms: Vec::new(),
            schedule,
            groups,
            group_crossings,
            group_tally,
            anneal_work: 0,
            work_before,
        }
    }

    /// The stages, each until the deadline or the work limit comes.
    pub(super) fn stages(&mut self, config: &AnnealConfig) {
        for stage in 0..config.stages {
            if config
                .deadline
                .is_some_and(|deadline| std::time::Instant::now() >= deadline)
                || config
                    .stop_at_work
                    .is_some_and(|stop| crate::work() >= stop)
            {
                break;
            }
            self.stage(config, stage);
        }
    }

    /// One stage: `moves_per_part` moves per movable part, the bus groups'
    /// batch, the stage's work booked, the schedule cooled, and the
    /// crossings term ended once its work reached its share.
    fn stage(&mut self, config: &AnnealConfig, stage: usize) {
        let mut work = 0;
        for _ in 0..config.moves_per_part * self.movable.len() {
            work += self.step();
        }
        if let Some(groups) = self.groups {
            work += group_batch(
                &mut self.state,
                groups,
                &self.move_work,
                GroupStep {
                    radius: self.schedule.radius,
                    temperature: self.schedule.temperature,
                    penalty: self.schedule.penalty,
                },
                self.group_crossings.as_mut(),
                self.crossings.as_mut(),
                &mut self.random,
                &mut self.group_tally,
            );
            // Its moves keep their own sums: the cache again.
            self.state.refresh_lengths();
        }
        let stage_work = work + self.state.work.replace(0);
        self.anneal_work += stage_work;
        crate::add_work(stage_work);
        self.schedule.cool();
        self.end_term(stage, config.stages);
    }

    /// The crossings term ends once its work reaches its share of the
    /// anneal's own (`TERM_SHARE`), projected over the `stages` after
    /// `stage`.
    fn end_term(&mut self, stage: usize, stages: usize) {
        // The term ends once its work reaches its share of the anneal's
        // own (`TERM_SHARE`).
        if let Some(state) = self.crossings.as_ref() {
            let done = stage + 1;
            let projected = self.anneal_work as f64 / done as f64 * stages as f64;
            if done < stages && self.tally.work as f64 >= TERM_SHARE * projected {
                self.tally.term_stages = Some(done);
                self.tally.crossings_after = Some(state.total());
                self.crossings = None;
            }
        }
    }

    /// The end: the debug lines; the
    /// poses into `poses`. Returns the remaining overlap area (the movable
    /// parts' overlaps summed and halved).
    pub(super) fn finish(self, poses: &mut Vec<Pose>) -> f64 {
        if self.groups.is_some() && std::env::var_os("PCB_PLACER_DEBUG").is_some() {
            let group_tally = &self.group_tally;
            eprintln!(
                "anneal: bus groups: {} moves tried, taken: {} shifts, {} turns, {} swaps, {} reorders; crossings of the groups' nets {:?} -> {:?} (after the last batch)",
                group_tally.tried,
                group_tally.taken[0],
                group_tally.taken[1],
                group_tally.taken[2],
                group_tally.taken[3],
                group_tally.crossings_before,
                self.group_crossings.as_ref().map(|state| state.total()),
            );
        }
        if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
            let tally = &self.tally;
            let work = crate::work() - self.work_before;
            let per_move = |amount: u64| amount as f64 / tally.moves.max(1) as f64;
            eprintln!(
                "anneal: {} moves, work {:.2} s ({:.0} a move), crossings' share {:.0} a move; judged with crossings {}, cut by the bound {}, taken {}; the term ended after {:?} stages; crossings {:?} -> {:?}",
                tally.moves,
                work as f64 / crate::work_per_second(),
                per_move(work),
                per_move(tally.work),
                tally.judged,
                tally.cut,
                tally.taken,
                tally.term_stages,
                tally.crossings_before,
                self.crossings
                    .as_ref()
                    .map(|state| state.total())
                    .or(tally.crossings_after),
            );
        }
        let state = &self.state;
        *poses = state.poses.clone();
        self.movable
            .iter()
            .map(|index| state.overlap(*index, state.rects[*index]))
            .sum::<f64>()
            / 2.0
    }
}
