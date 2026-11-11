// Copyright (C) 2026 Toit contributors.

//! The placements the race may probe: the other seeds' placements that keep
//! the constraints as well as the kept one, the placement digest, the
//! pre-ranking by predicted routability and the probes the budget allows.

use super::placement::{Copper, posed};
use super::setup::{LayoutContext, WorkLedger};
use super::*;

/// One placement besides the routed one: its poses and its relaxation.
pub(super) type OtherPlacement = (Vec<placer::Pose>, placer::Relaxation);

/// The placement the layout routes: the placed board, its lowering for the
/// router and for the moves, and the placer configuration the moves keep
/// to (its `tight_bodies` the placement's).
pub(super) struct Kept {
    pub pcb: Expr,
    pub board: core::Board,
    pub problem: crate::board_placer::LoweredPlacement,
    pub placer_config: KiCadBoardPlacerConfig,
}

impl Kept {
    /// Takes the placement `poses` at `relaxation`: the board posed, its
    /// lowering for the moves again. The router's board is the caller's.
    pub(super) fn adopt(
        &mut self,
        pcb: Expr,
        relaxation: &placer::Relaxation,
    ) -> Result<(), String> {
        self.pcb = pcb;
        self.placer_config.tight_bodies = Some(relaxation.tight);
        self.problem = lower_placement(&self.pcb, &self.placer_config)?;
        relaxation.apply(&mut self.problem.problem);
        Ok(())
    }
}

/// The placements the race may probe beside the routed one, in the order
/// it probes them, and what the report needs of them.
pub(super) struct Candidates {
    /// The other placements, in the order the race probes them.
    pub others: Vec<OtherPlacement>,
    /// Each raced placement's index among the ranked ones (as
    /// `congestion.scores`; without a pre-ranking, the kept one and the
    /// others in seed order).
    pub labels: Vec<usize>,
    /// Per placement of the pool (the kept one, then the others in the
    /// placer's order), its pads without a way out (`pad_exits`; `None`
    /// off): the report follows the placement the race keeps.
    pub pool_exits: Vec<Option<crate::board_placer::KiCadPadExitsReport>>,
    /// The pre-ranking, where it ran.
    pub congestion: Option<KiCadCongestionRanking>,
    /// The placements the final route's minimum share left unprobed.
    pub dropped: usize,
    /// The probes' work where the budget shortened them.
    pub shortened: Option<f64>,
}

/// Whether two placements are alike (every pose within a micrometre and a
/// thousandth of a degree): seeds that placed alike count once.
pub(super) fn alike(a: &[placer::Pose], b: &[placer::Pose]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            (a.position[0] - b.position[0]).abs() < 1.0e-3
                && (a.position[1] - b.position[1]).abs() < 1.0e-3
                && ((a.angle - b.angle).rem_euclid(360.0) + 1.0e-3) % 360.0 < 2.0e-3
        })
}

/// The indices of the `alternatives` (poses, relaxation, how far they miss
/// the constraints) that race the kept placement (`kept`, missing by
/// `missed`): those that keep the constraints as well and differ from
/// every placement before them.
pub(super) fn distinct_alternatives(
    kept: &[placer::Pose],
    missed: f64,
    alternatives: &[(Vec<placer::Pose>, placer::Relaxation, f64)],
) -> Vec<usize> {
    let mut seen = vec![kept.to_vec()];
    let mut chosen = Vec::new();
    for (index, (poses, _, other_missed)) in alternatives.iter().enumerate() {
        if *other_missed <= missed + 1.0e-6 && !seen.iter().any(|known| alike(known, poses)) {
            seen.push(poses.clone());
            chosen.push(index);
        }
    }
    chosen
}

/// The candidates before any ranking: the other seeds' placements that
/// race the kept one (`distinct_alternatives`), in the placer's order, and
/// every pool placement's pads without a way out.
pub(super) fn race_candidates(
    placement: &KiCadBoardPlacerResult,
    kept: &[placer::Pose],
) -> Candidates {
    // What the user asked for comes first: only placements that keep the
    // constraints as well as the kept one race it. Seeds that placed alike
    // (every part fixed: the laptop raced one placement against itself for
    // 6 minutes) count once.
    let missed: f64 = placement
        .constraints
        .iter()
        .map(|status| status.violation_mm)
        .sum();
    let mut others = Vec::new();
    let mut pool_exits = vec![placement.pad_exits.clone()];
    for index in distinct_alternatives(kept, missed, &placement.alternatives) {
        let (poses, relaxation, _) = &placement.alternatives[index];
        others.push((poses.clone(), relaxation.clone()));
        pool_exits.push(placement.alternative_exits.get(index).cloned().flatten());
    }
    Candidates {
        labels: (0..=others.len()).collect(),
        others,
        pool_exits,
        congestion: None,
        dropped: 0,
        shortened: None,
    }
}

/// What the steps after the placement read from it, as a digest
/// (`placement_digest`), logged: the harness reuses a layout's routing for
/// a placement it has routed before (run.py --reuse).
pub(super) fn digest(
    kept: &Kept,
    candidates: &Candidates,
    pin_swaps: Option<&KiCadPinSwapResult>,
    ledger: &WorkLedger,
) -> String {
    let others: Vec<(&Vec<placer::Pose>, &placer::Relaxation)> = candidates
        .others
        .iter()
        .map(|(poses, relaxation)| (poses, relaxation))
        .collect();
    let digest = placement_digest(
        &encode(&kept.pcb),
        &kept.problem,
        &others,
        &kept.placer_config,
        pin_swaps,
        ledger.work,
        ledger.pool_excess,
    );
    eprintln!("layout: placement digest {digest}");
    digest
}

/// How many placements the race probes: the route probe's own count, else
/// `congestion_probes`; at least one.
pub(super) fn race_probes(config: &KiCadBoardLayoutConfig) -> usize {
    config
        .race_probe
        .as_ref()
        .map_or(config.congestion_probes, |probe| probe.seeds)
        .max(1)
}

/// The seeds' placements ranked by predicted routability (RUDY,
/// `congestion_score`), where the race pre-ranks and there
/// are more than it probes: only the best few are probed (a probe is a
/// real route of up to `probe_seconds`; a score takes milliseconds), in
/// the ranking's order. Where the first-ranked is not the placer's kept
/// one, it becomes the routed placement (lowered again, its work booked).
/// Scores and their order depend only on the placements, which the
/// placer makes per seed and sorts by its own key: nothing here depends
/// on thread timing.
#[allow(
    clippy::too_many_arguments,
    reason = "the layout's state and the first route's settings are threaded through by reference"
)]
pub(super) fn pre_rank(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    kept: &mut Kept,
    candidates: &mut Candidates,
    first_relaxation: placer::Relaxation,
    core_config: &core::Config,
    copper: &Copper,
    no_pin_swaps: bool,
) -> Result<(), String> {
    let config = ctx.config;
    let probes = race_probes(config);
    if !(pre_ranks(config, &ctx.router_config)
        && ctx.seeds == 1
        && no_pin_swaps
        && candidates.others.len() + 1 > probes)
    {
        return Ok(());
    }
    // The placements the pre-ranking scores (the kept one first).
    let mut pool: Vec<OtherPlacement> = Vec::new();
    pool.push((kept.problem.problem.poses.clone(), first_relaxation));
    pool.extend(
        candidates
            .others
            .iter()
            .map(|(poses, relaxation)| (poses.clone(), relaxation.clone())),
    );
    crate::mem_profile::phase("pre-ranking");
    let ranked = crate::congestion_rank::rank_placements(
        &without_copper_texts(&kept.pcb, config.placer.keep_board_marks),
        &pool,
        &ctx.router_config,
        core_config,
        copper.connect,
    )?;
    let crossings = seed_crossings(&kept.placer_config, &kept.problem, &pool);
    let order = probe_order(&ranked.order, crossings.as_deref(), &pool);
    let keep: Vec<usize> = order.iter().copied().take(probes).collect();
    eprintln!(
        "placement pre-ranking ({}): {} placements scored in {:.2} s (lowering {:.2} s, scoring {:.2} s); probing {:?} (scores {:?})",
        config.congestion_score,
        pool.len(),
        ranked.seconds,
        ranked.lower_seconds,
        ranked.score_seconds,
        keep,
        keep.iter()
            .map(|index| (ranked.scores[*index] * 100.0).round() / 100.0)
            .collect::<Vec<_>>()
    );
    if keep[0] != 0 {
        let (poses, relaxation) = &pool[keep[0]];
        let candidate = posed(&kept.pcb, poses)?;
        let plan_work;
        (kept.board, plan_work) = lower_for_route(
            &without_copper_texts(&candidate, config.placer.keep_board_marks),
            &ctx.router_config,
            copper.connect,
        )?;
        ledger.work += plan_work;
        kept.adopt(candidate, relaxation)?;
    }
    candidates.others = keep[1..].iter().map(|index| pool[*index].clone()).collect();
    candidates.labels = keep.clone();
    candidates.congestion = Some(KiCadCongestionRanking {
        scores: ranked.scores,
        probed: keep,
        crossings,
    });
    Ok(())
}

/// Each pool placement's ratsnest crossings, where they rank the seeds
/// beside the score (`ratsnest_crossings.rank_seeds`).
fn seed_crossings(
    placer_config: &KiCadBoardPlacerConfig,
    problem: &crate::board_placer::LoweredPlacement,
    pool: &[OtherPlacement],
) -> Option<Vec<u64>> {
    placer_config
        .ratsnest_crossings
        .as_ref()
        .filter(|crossings| crossings.rank_seeds)
        .map(|_| {
            let fact = placer::crossings::Crossings {
                counted: crate::board_placer::crossing_nets(problem),
                weight: 0.0,
                legalise: false,
                refine: false,
                anneal: false,
            };
            pool.iter()
                .map(|(poses, _)| placer::crossings::total(&problem.problem, &fact, poses))
                .collect::<Vec<u64>>()
        })
}

/// The order the race probes the pool in: the score's ranking (`ranked`),
/// with the crossings by the sum of the two ranks (a tie to the better
/// score), and a placement that needed courtyards touching or parts on
/// marks after every one that did not.
pub(super) fn probe_order(
    ranked: &[usize],
    crossings: Option<&[u64]>,
    pool: &[OtherPlacement],
) -> Vec<usize> {
    // A placement that needed courtyards touching is a last resort
    // whatever its score: it is probed only when too few others are
    // (laptop power: RUDY picked a touching seed, 18 connections open
    // against 9 for the best of the five that needed no touching).
    let mut order = ranked.to_vec();
    // Ratsnest crossings beside the score: the placements by the sum
    // of their ranks by the score and by the crossings (a tie to the
    // better score).
    if let Some(counts) = crossings {
        let mut score_rank = vec![0usize; pool.len()];
        for (rank, index) in ranked.iter().enumerate() {
            score_rank[*index] = rank;
        }
        let mut by_crossings: Vec<usize> = (0..pool.len()).collect();
        by_crossings.sort_by_key(|index| (counts[*index], score_rank[*index]));
        let mut crossing_rank = vec![0usize; pool.len()];
        for (rank, index) in by_crossings.iter().enumerate() {
            crossing_rank[*index] = rank;
        }
        order.sort_by_key(|index| {
            (
                score_rank[*index] + crossing_rank[*index],
                score_rank[*index],
            )
        });
        eprintln!("placement pre-ranking: ratsnest crossings {counts:?}, order {order:?}");
    }
    order.sort_by_key(|index| {
        (
            pool[*index].1.courtyard_spacing,
            pool[*index].1.on_marks.len(),
        )
    });
    order
}

/// The race's probes cut to what it may probe: with the route probe its
/// own count (the kept one, then seed order); without it, as many as leave
/// the final route its share (`race_probes_that_fit`), each shortened
/// alike where needed.
pub(super) fn limit_race(
    ctx: &LayoutContext,
    ledger: &WorkLedger,
    candidates: &mut Candidates,
    no_pin_swaps: bool,
) {
    let config = ctx.config;
    if config.race_probe.is_some() {
        // Without a pre-ranking all seeds' placements would race: the
        // probe takes the first `probes` (the kept one, then seed order).
        let probes = race_probes(config);
        candidates.others.truncate(probes - 1);
        candidates.labels.truncate(probes);
    }
    // Every probe that runs counts and is charged to the work clock; the
    // final route keeps at least `RACE_MIN_FINAL_SHARE` of the budget.
    // Where the placement overran its share (or the budget is small for
    // 75 s probes), the race shortens every probe alike (they stay
    // comparable), to a third at most, then probes fewer placements (the
    // lowest-ranked dropped first; `race_probes_that_fit`). The kept
    // placement's probe is the first part of its first route: only the
    // others count against the minimum. Decided before any probe runs,
    // from the work so far: deterministic. (Until 2026-10-25 the probes
    // ran and a gate dropped every one judged past 40 % of the budget,
    // uncharged: the race kept the placer's pick on the boards whose
    // placement takes its whole share.)
    if config.race_probe.is_none()
        && ctx.seeds == 1
        && no_pin_swaps
        && !candidates.others.is_empty()
    {
        let probe_seconds = ctx.router_config.probe_seconds.unwrap_or(75.0);
        let others = candidates.others.len();
        let (fit, seconds) = race_probes_that_fit(
            config.total_seconds.max(0.0),
            ledger.clock(),
            probe_seconds,
            others,
        );
        if fit < others || seconds < probe_seconds {
            candidates.dropped = others - fit;
            eprintln!(
                "placement race: {fit} of {others} other placements probed, {seconds:.0} s each (work {:.0} s of {:.0}: the final route keeps {:.0} %)",
                ledger.clock(),
                config.total_seconds,
                RACE_MIN_FINAL_SHARE * 100.0
            );
            candidates.others.truncate(fit);
            candidates.labels.truncate(fit + 1);
            if let Some(congestion) = &mut candidates.congestion {
                congestion.probed.truncate(fit + 1);
            }
            if seconds < probe_seconds {
                candidates.shortened = Some(seconds);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(x: f64, y: f64, angle: f64) -> placer::Pose {
        placer::Pose {
            position: [x, y],
            angle,
        }
    }

    fn relaxation(courtyard_spacing: bool, on_marks: usize) -> placer::Relaxation {
        placer::Relaxation {
            spacing: 0.0,
            grid: 0.0,
            halo_scale: 1.0,
            small_halo_scale: 1.0,
            edge_inset: false,
            tight: false,
            edge_copper: false,
            edge_rule: 0.0,
            courtyard_spacing,
            pad_clearance: false,
            halos: None,
            on_marks: vec![0; on_marks],
        }
    }

    #[test]
    fn placements_alike_within_a_micrometre_and_a_full_turn_count_once() {
        let a = [pose(1.0, 2.0, 90.0)];
        assert!(alike(&a, &[pose(1.0 + 5.0e-4, 2.0, 90.0)]));
        assert!(alike(&a, &[pose(1.0, 2.0, 450.0)]));
        assert!(alike(&[pose(0.0, 0.0, 0.0)], &[pose(0.0, 0.0, 359.9995)]));
        assert!(!alike(&a, &[pose(1.01, 2.0, 90.0)]));
        assert!(!alike(&a, &[pose(1.0, 2.0, 180.0)]));
        assert!(!alike(&a, &[pose(1.0, 2.0, 90.0), pose(0.0, 0.0, 0.0)]));
    }

    #[test]
    fn only_distinct_placements_that_keep_the_constraints_as_well_race() {
        let kept = vec![pose(0.0, 0.0, 0.0)];
        let alternatives = vec![
            // As the kept one: not again.
            (vec![pose(0.0, 0.0, 0.0)], relaxation(false, 0), 0.0),
            (vec![pose(5.0, 0.0, 0.0)], relaxation(false, 0), 0.0),
            // Misses the constraints by more than the kept one.
            (vec![pose(9.0, 0.0, 0.0)], relaxation(false, 0), 1.0),
            // As the second: not again.
            (vec![pose(5.0, 0.0, 0.0)], relaxation(false, 0), 0.5),
            (vec![pose(0.0, 7.0, 0.0)], relaxation(false, 0), 0.5),
        ];
        assert_eq!(distinct_alternatives(&kept, 0.5, &alternatives), vec![1, 4]);
        assert_eq!(distinct_alternatives(&kept, 0.0, &alternatives), vec![1]);
    }

    #[test]
    fn the_probe_order_puts_touching_courtyards_and_marks_last() {
        let pool = vec![
            (Vec::new(), relaxation(false, 0)),
            (Vec::new(), relaxation(true, 0)),
            (Vec::new(), relaxation(false, 2)),
            (Vec::new(), relaxation(false, 0)),
        ];
        // The score ranks 1, 2, 3, 0.
        assert_eq!(probe_order(&[1, 2, 3, 0], None, &pool), vec![3, 0, 2, 1]);
        // With the crossings: by the sum of the ranks, ties to the score.
        // Score ranks: 1:0, 2:1, 3:2, 0:3; crossings rank 0:0, 3:1, 2:2,
        // 1:3. Sums: 0:3, 1:3, 2:3, 3:3: the score's order.
        let order = probe_order(&[1, 2, 3, 0], Some(&[1, 9, 5, 2]), &pool);
        assert_eq!(order, vec![3, 0, 2, 1]);
        // Score ranks 3:0, 0:1, 1:2, 2:3; crossings rank 0:0, 1:1, 3:2,
        // 2:3. Sums 0:1, 3:2, 1:3, 2:6: the crossings put 0 first.
        let order = probe_order(&[3, 0, 1, 2], Some(&[1, 2, 9, 7]), &pool);
        assert_eq!(order, vec![0, 3, 2, 1]);
    }
}
