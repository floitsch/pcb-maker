// Copyright (C) 2026 Toit contributors.

//! The first route: the placement race (each candidate placement probed by
//! a router of its own, side by side as memory allows, the best one routed
//! on) or, without a race, the first route of the kept placement.

use super::candidates::{Candidates, Kept, OtherPlacement};
use super::placement::{Copper, posed};
use super::setup::{LayoutContext, WorkLedger};
use super::*;

/// What the race decided, for the report: each probe's nets unfinished,
/// the race's report, why the routes have lines at the pin rows (where
/// they do), the pre-ranking and the routed placement's pads without a way
/// out.
pub(super) struct RaceOutcome {
    pub placement_race: Vec<usize>,
    pub report: Option<KiCadRaceReport>,
    pub lines_why: Option<String>,
    pub congestion: Option<KiCadCongestionRanking>,
    pub pad_exits: Option<crate::board_placer::KiCadPadExitsReport>,
}

/// The router settings of the race, the move trials and the first route
/// (`race_core_config`), and the kept placement lowered for the router
/// (with the pairs, their plan's work is the first route's, booked). A new
/// router of the first route stops where the layout's work ends. Hands on
/// the settings, the router's board and when the first route started.
pub(super) fn lower_first_route(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    pcb: &Expr,
    copper: &Copper,
) -> Result<(core::Config, core::Board, std::time::Instant), String> {
    // The in-place router compares placements (`race_core_config`); the
    // final ladder routes with the full patience.
    let mut core_config = race_core_config(&ctx.router_config);
    let first_started = std::time::Instant::now();
    // (With the pairs, their plan's work is the first route's.)
    let (board, plan_work) = lower_for_route(
        &without_copper_texts(pcb, ctx.config.placer.keep_board_marks),
        &ctx.router_config,
        copper.connect,
    )?;
    ledger.work += plan_work;
    // A new router of the first route stops where the layout's work ends.
    core_config.deadline_work = Some(ledger.left(ctx.work_deadline).max(0.0));
    Ok((core_config, board, first_started))
}

/// The first route of the layout: the race over `candidates` where it runs
/// (one router seed, no pin swaps, another placement to probe, the wall
/// clock allowing), else `route_once` on the kept placement. The kept
/// placement becomes the race's pick. Hands on the router and its result,
/// and what the race decided.
pub(super) fn race_and_route(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    kept: &mut Kept,
    candidates: Candidates,
    core_config: &core::Config,
    copper: &Copper,
    no_pin_swaps: bool,
) -> Result<(core::router::Router, core::RoutingResult, RaceOutcome), String> {
    let Candidates {
        others,
        labels,
        pool_exits,
        congestion,
        dropped,
        shortened,
    } = candidates;
    // The pool index of the placement routed: the first-ranked, unless the
    // race keeps another.
    let mut routed_pool_index = labels[0];
    let mut placement_race = Vec::new();
    let mut report = None;
    // Why the routes from the kept placement's probe on have lines at the
    // pin rows (`race_lines_probe`), where they do.
    let mut lines_why = None;
    let (router, result) = if ctx.seeds == 1
        && no_pin_swaps
        && !others.is_empty()
        && ctx.stop.allows(LayoutStep::Race)
    {
        let race = run_race(
            ctx,
            ledger,
            kept,
            others,
            &labels,
            dropped,
            shortened,
            core_config,
            copper,
        )?;
        routed_pool_index = race.routed_pool_index;
        placement_race = race.unfinished;
        report = Some(race.report);
        lines_why = race.lines_why;
        (race.router, race.result)
    } else {
        crate::mem_profile::phase("first route");
        let (router, result, first_work) = route_once(&kept.board, core_config, ctx.seeds);
        ledger.work += first_work;
        (router, result)
    };
    Ok((
        router,
        result,
        RaceOutcome {
            placement_race,
            report,
            lines_why,
            congestion,
            pad_exits: pool_exits.get(routed_pool_index).cloned().flatten(),
        },
    ))
}

/// The first route without a race: with seeds, done several ways at once,
/// the best router state carrying the move phase. Hands on that router,
/// its result and the work of every seed.
fn route_once(
    board: &core::Board,
    core_config: &core::Config,
    seeds: u64,
) -> (core::router::Router, core::RoutingResult, f64) {
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..seeds)
            .map(|index| {
                let mut seeded = core_config.clone();
                // As the race's resume below: at most half of what is
                // left before the deadline negotiates.
                let left = seeded.deadline_work.unwrap_or(f64::INFINITY);
                let half = ((left / 2.0).max(1.0) * core::router::expansions_per_second()) as u64;
                seeded.negotiation_expansions = if seeded.negotiation_expansions == 0 {
                    half
                } else {
                    seeded.negotiation_expansions.min(half)
                };
                seeded.seed = (index > 0).then_some(index);
                seeded.verbose = core_config.verbose && index == 0;
                scope.spawn(move || {
                    // Placements are compared on what stays open; the
                    // polish waits for the one kept.
                    let mut router = core::router::Router::new(board, &seeded);
                    let result = router.run_in_place_polished(false);
                    (router, result)
                })
            })
            .collect();
        let routed: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().expect("routing thread"))
            .collect();
        let expansions: u64 = routed.iter().map(|(router, _)| router.expansions()).sum();
        let best = routed
            .into_iter()
            .min_by(|a, b| {
                score(&a.1, board)
                    .partial_cmp(&score(&b.1, board))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("at least one seed");
        (
            best.0,
            best.1,
            expansions as f64 / core::router::expansions_per_second(),
        )
    })
}

/// The race's outcome: the kept placement's router after its first route,
/// that route, the race's report and its probes' nets unfinished.
struct Race {
    router: core::router::Router,
    result: core::RoutingResult,
    report: KiCadRaceReport,
    unfinished: Vec<usize>,
    routed_pool_index: usize,
    lines_why: Option<String>,
}

/// The probes of the race, in the ranking's order: each one's router
/// parked, the nets it left unfinished, its standing, its whole first
/// route (with `race_probe.repair`) and, with the ripped count, its open
/// terminals and nets ripped.
struct RaceProbes {
    parked: Vec<(usize, core::router::ParkedRouter)>,
    unfinished: Vec<usize>,
    standings: Vec<core::router::ProbeStanding>,
    results: Vec<Option<core::RoutingResult>>,
    ripped_counts: Vec<(usize, usize)>,
    cost: KiCadRaceRippedCost,
}

/// The race: the other placements lowered, every placement probed with the
/// same search, the probes charged and scored, one placement picked and
/// made the kept one, the lines at the pin rows decided at its probe, and
/// its first route finished from its probe. Wirelength is not routability:
/// each placement is probed as route mode probes its rungs, and only the
/// one with the fewest nets unfinished routes on (routing the others in
/// full took as long as the first route and left link no time for
/// anything after).
#[allow(
    clippy::too_many_arguments,
    reason = "the layout's state and the candidates' parts are threaded through"
)]
fn run_race(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    kept: &mut Kept,
    others: Vec<OtherPlacement>,
    labels: &[usize],
    dropped: usize,
    shortened: Option<f64>,
    core_config: &core::Config,
    copper: &Copper,
) -> Result<Race, String> {
    let config = ctx.config;
    let race_probe = config.tuning.race_probe.as_ref();
    let probe_seconds = match race_probe {
        Some(probe) => probe.share.max(0.0) * config.total_seconds.max(0.0),
        None => shortened.unwrap_or(ctx.router_config.probe_seconds.unwrap_or(75.0)),
    };
    let candidates = lower_candidates(ctx, ledger, &kept.pcb, others, copper)?;
    let boards: Vec<&core::Board> = std::iter::once(&kept.board)
        .chain(candidates.iter().map(|(_, board, _)| board))
        .collect();
    let repair = race_probe.is_some_and(|probe| probe.repair);
    // The ripped count (`race_count: "ripped"`): each probe's router's
    // nets in conflict stripped of their conflicted branches and the
    // board's open terminals counted, the router then put back as it
    // negotiated (the kept one routes on). One router at a time, in
    // the ranking's order, as each wave of probes ends: deterministic.
    // No search runs: nothing is charged to the work clock.
    let ripped_asked = config.tuning.race_count == KiCadRaceCount::Ripped;
    let ripped = ripped_asked && !repair;
    let (probes, router_mb) =
        probe_placements(&boards, core_config, probe_seconds, repair, ripped_asked);
    drop(boards);
    // Every probe that ran is charged.
    let probe_work: Vec<f64> = probes
        .parked
        .iter()
        .map(|(_, router)| router.expansions() as f64 / core::router::expansions_per_second())
        .collect();
    ledger.work += probe_work.iter().sum::<f64>();
    let ripped_cost = ripped.then(|| {
        eprintln!(
            "placement race: the ripped count: copies {:.2} s (at most {:.0} MB), rip-ups {:.2} s",
            probes.cost.copy_seconds, probes.cost.copy_mb, probes.cost.rip_seconds
        );
        probes.cost.clone()
    });
    let (scores, kept_gated) = score_probes(
        config,
        ledger.work,
        &probes,
        &probe_work,
        labels,
        probe_seconds,
    );
    let (best, chosen, confident, reason) =
        pick_placement(&probes, labels, ripped, race_probe.is_some());
    eprintln!(
        "placement race: placement {} kept: {reason}",
        labels[chosen]
    );
    let RaceProbes {
        parked,
        unfinished,
        mut results,
        ..
    } = probes;
    let mut parked: Vec<Option<core::router::ParkedRouter>> =
        parked.into_iter().map(|(_, router)| Some(router)).collect();
    let chosen_probe = parked[chosen].take().expect("the chosen placement's probe");
    drop(parked);
    let mut router = chosen_probe.unpark();
    if chosen > 0 {
        let (poses, candidate_board, relaxation) = candidates
            .into_iter()
            .nth(chosen - 1)
            .expect("a candidate per probe");
        let pcb = posed(&kept.pcb, &poses)?;
        kept.board = candidate_board;
        router.config_mut().verbose = core_config.verbose;
        kept.adopt(pcb, &relaxation)?;
    }
    // The lines at the pin rows, decided at the kept placement's probe
    // by the pads it left unfinished (`race_lines_probe`; the setting
    // unset; not with the repair probe, whose probe is a whole first
    // route).
    let mut lines_why = None;
    if !repair
        && crate::board_router::layout_lines_probes().race
        && race_lines_automatic(&ctx.router_config)
    {
        let left = ledger.left(ctx.work_deadline).max(0.0);
        let (lines_work, lead) = race_lines_probe(
            &kept.board,
            core_config,
            &router,
            unfinished[chosen],
            probe_seconds,
            left,
            router_mb,
        );
        ledger.work += lines_work;
        if let Some((mut lines_router, why)) = lead {
            lines_router.config_mut().verbose = core_config.verbose;
            router = lines_router;
            lines_why = Some(why);
        }
    }
    let report = KiCadRaceReport {
        probe: scores,
        kept: labels[chosen],
        probe_seconds,
        repair,
        kept_gated,
        dropped,
        best: labels[best],
        confident,
        reason,
        ripped: ripped_cost,
    };
    ctx.arm(&mut router, ledger.clock());
    // A whole first route as the probe: the kept one's is the first
    // route.
    let result = match results.get_mut(chosen).and_then(Option::take) {
        Some(result) => result,
        None => first_route_from_probe(ctx, ledger, &mut router, core_config, probe_seconds),
    };
    Ok(Race {
        router,
        result,
        report,
        unfinished,
        routed_pool_index: labels[chosen],
        lines_why,
    })
}

/// The other placements lowered for the router, in the race's order (their
/// plans' work booked). A placement's board is written again for the one
/// the race keeps (the same footprints written at the same poses): the
/// race held a copy of the parsed board per placement until 2026-11-04
/// (link: a 10 MB board).
fn lower_candidates(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    pcb: &Expr,
    others: Vec<OtherPlacement>,
    copper: &Copper,
) -> Result<Vec<(Vec<placer::Pose>, core::Board, placer::Relaxation)>, String> {
    let mut candidates = Vec::new();
    for (poses, relaxation) in others {
        let (candidate_board, plan_work) = lower_for_route(
            &without_copper_texts(&posed(pcb, &poses)?, ctx.config.placer.keep_board_marks),
            &ctx.router_config,
            copper.connect,
        )?;
        ledger.work += plan_work;
        candidates.push((poses, candidate_board, relaxation));
    }
    Ok(candidates)
}

/// Probes every placement (`boards`, the kept one first) for
/// `probe_seconds` of work each, side by side, each a router of its own,
/// as many at once as fit half the memory available (the first router
/// tells its size), the threads and the memory ledger. The outcome is the
/// one of probing them one after the other: the first placement, then the
/// others in the ranking's order. A probe's budget is work, so what runs
/// beside it does not change it. `ripped_asked`: `race_count: "ripped"`
/// (with `repair` it does not apply). Hands on the probes and a router's
/// size.
fn probe_placements(
    boards: &[&core::Board],
    core_config: &core::Config,
    probe_seconds: f64,
    repair: bool,
    ripped_asked: bool,
) -> (RaceProbes, f64) {
    let ripped = ripped_asked && !repair;
    crate::mem_profile::phase("race probes");
    let before_mb = crate::proc_memory::resident_mb();
    let first = core::router::Router::new(boards[0], core_config);
    let router_mb = (crate::proc_memory::resident_mb() - before_mb).max(1.0);
    let side_by_side = crate::board_router::memory_mb()
        .map_or(1, |(_, available)| {
            (available * 0.5 / (router_mb * 1.5)) as usize
        })
        .clamp(1, boards.len());
    // No more than the search threads (`at_most_the_threads`).
    let side_by_side = crate::board_router::at_most_the_threads(side_by_side);
    // The memory ledger: as many as it has room for, reserved first.
    let side_by_side = crate::ledger::routers_side_by_side(
        0.0,
        router_mb * 1.5,
        side_by_side,
        "the placement race's routers",
    );
    if side_by_side > 1 {
        eprintln!(
            "placement race: {} placements, {side_by_side} side by side",
            boards.len()
        );
    }
    let mut probes = RaceProbes {
        parked: Vec::new(),
        unfinished: Vec::new(),
        standings: Vec::new(),
        results: Vec::new(),
        ripped_counts: Vec::new(),
        cost: KiCadRaceRippedCost {
            copy_seconds: 0.0,
            rip_seconds: 0.0,
            copy_mb: 0.0,
        },
    };
    if ripped_asked && repair {
        eprintln!(
            "placement race: race_count \"ripped\" does not apply with race_probe.repair (each probe's count is its first route's)"
        );
    }
    let mut first = Some(first);
    let mut next = 0;
    while next < boards.len() {
        let wave: Vec<usize> = (next..(next + side_by_side).min(boards.len())).collect();
        let routers = probe_wave(
            &wave,
            &mut first,
            boards,
            core_config,
            probe_seconds,
            repair,
        );
        // The wave's search arrays (as many sets as searches ran side
        // by side) are idle from here: the ripped count searches
        // nothing, and the next wave and the kept router's route take
        // what they need again.
        let idle = core::router::release_idle_search_arrays();
        crate::mem_profile::note("the race's idle search arrays freed:", idle);
        for (index, unfinished, standing, repaired, mut router) in routers {
            if ripped {
                probes
                    .ripped_counts
                    .push(ripped_count(&mut router, router_mb, &mut probes.cost));
            }
            // Parked until the race picks: only the kept one is built
            // whole again.
            probes.parked.push((index, router.park()));
            probes.unfinished.push(unfinished);
            probes.standings.push(standing);
            probes.results.push(repaired);
        }
        next += wave.len();
    }
    (probes, router_mb)
}

/// One wave of probes, side by side: the placements `wave` (indices into
/// `boards`), each a router of its own (index 0's the one built already,
/// `first`), probed for `probe_seconds`, or with `repair` a whole first
/// route within that share. One probe's lines in the log.
fn probe_wave(
    wave: &[usize],
    first: &mut Option<core::router::Router>,
    boards: &[&core::Board],
    core_config: &core::Config,
    probe_seconds: f64,
    repair: bool,
) -> Vec<RaceProbed> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = wave
            .iter()
            .map(|&index| {
                let built = if index == 0 { first.take() } else { None };
                let board = boards[index];
                let mut quiet = core_config.clone();
                // One probe's lines in the log; the others' would
                // interleave with them.
                quiet.verbose = quiet.verbose && index == wave[0];
                scope.spawn(move || {
                    let mut router =
                        built.unwrap_or_else(|| core::router::Router::new(board, &quiet));
                    if !repair {
                        let unfinished = router.probe(probe_seconds);
                        let standing = router.probe_standing();
                        return (index, unfinished, standing, None, router);
                    }
                    // A whole first route within the share: half of
                    // it negotiating at most, then the repair.
                    let end = router.work() + probe_seconds;
                    let saved = router.config_mut().deadline_work;
                    router.config_mut().deadline_work =
                        Some(saved.unwrap_or(f64::INFINITY).min(end));
                    let unfinished = router.probe(probe_seconds / 2.0);
                    let mut standing = router.probe_standing();
                    let left = (end - router.work()).max(0.0);
                    let result = router.resume_polished((left / 2.0).max(1.0), false);
                    standing.open_terminals = score(&result, board).0;
                    router.config_mut().deadline_work = saved;
                    (index, unfinished, standing, Some(result), router)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("probe thread"))
            .collect()
    })
}

/// The ripped count of one probe's router (`race_count: "ripped"`): the
/// router ripped in place and put back, its memory reserved first. Adds
/// its wall clock and memory to `cost`; hands on the board's open
/// terminals after the rip-up and the nets ripped.
fn ripped_count(
    router: &mut core::router::Router,
    router_mb: f64,
    cost: &mut KiCadRaceRippedCost,
) -> (usize, usize) {
    crate::ledger::reserve_at_least(
        crate::proc_memory::resident_mb() + router_mb,
        "the race's ripped count",
    );
    crate::mem_profile::phase("race ripped count");
    let before_mb = crate::proc_memory::resident_mb();
    let started = std::time::Instant::now();
    // The probe's router ripped in place and put back:
    // what it saves to go back to is a fraction of a copy.
    let saved = router.saved_for_rip_up();
    cost.copy_seconds += started.elapsed().as_secs_f64();
    cost.copy_mb = cost
        .copy_mb
        .max(crate::proc_memory::resident_mb() - before_mb);
    let started = std::time::Instant::now();
    let (nets, standing) = router.rip_up_standing();
    router.restore_after_rip_up(saved);
    cost.rip_seconds += started.elapsed().as_secs_f64();
    (standing.open_terminals, nets)
}

/// Each probe's score for the report, logged, with what the race did until
/// 2026-10-25 (the 40 % gate, its probes uncharged): whether each probe
/// would have been dropped, and the placement it would have kept (the
/// second value). `work` is the layout's with every probe charged.
fn score_probes(
    config: &KiCadBoardLayoutConfig,
    work: f64,
    probes: &RaceProbes,
    probe_work: &[f64],
    labels: &[usize],
    probe_seconds: f64,
) -> (Vec<KiCadRaceProbeScore>, usize) {
    let race = &probes.unfinished;
    let ripped_counts = &probes.ripped_counts;
    let gate = PLACEMENT_SHARE * config.total_seconds;
    let mut gated_work = work - probe_work[1..].iter().sum::<f64>();
    let mut gated_best = race[0];
    let mut kept_gated = labels[0];
    let mut gated = false;
    let mut scores = Vec::new();
    for index in 0..race.len() {
        if index > 0 {
            gated = gated || gated_work > gate;
            if !gated {
                gated_work += probe_work[index];
                if race[index] < gated_best {
                    gated_best = race[index];
                    kept_gated = labels[index];
                }
            }
        }
        let standing = probes.standings[index];
        scores.push(KiCadRaceProbeScore {
            placement: labels[index],
            open_terminals: standing.open_terminals,
            unfinished: standing.unfinished,
            conflicted: standing.conflicted,
            work_seconds: probe_work[index],
            gated,
            ripped_open_terminals: ripped_counts.get(index).map(|count| count.0),
            ripped_nets: ripped_counts.get(index).map(|count| count.1),
        });
        eprintln!(
            "placement race: probed placement {} ({probe_seconds:.0} s): {} open terminals, {} nets unfinished, {} in conflict{}{}",
            labels[index],
            standing.open_terminals,
            standing.unfinished,
            standing.conflicted,
            ripped_counts
                .get(index)
                .map_or(String::new(), |(open, nets)| format!(
                    "; {open} open terminals after ripping {nets} nets"
                )),
            if gated {
                " (the 40 % gate would have dropped it)"
            } else {
                ""
            }
        );
    }
    (scores, kept_gated)
}

/// The race's pick: the probes' best (by the ripped count, ties by the
/// standing; else with the route probe by the standing; else by the nets
/// left unfinished; a tie to the better-ranked placement) and the one
/// kept (with the route probe the best, else `race_pick`'s margin over
/// the first-ranked). Hands on the best, the chosen, whether the probes
/// were confident, and why.
fn pick_placement(
    probes: &RaceProbes,
    labels: &[usize],
    ripped: bool,
    route_probe: bool,
) -> (usize, usize, bool, String) {
    let race = &probes.unfinished;
    let standings = &probes.standings;
    let ripped_counts = &probes.ripped_counts;
    let mut best = 0;
    for index in 1..race.len() {
        let better = if ripped {
            (ripped_counts[index].0, standings[index]) < (ripped_counts[best].0, standings[best])
        } else if route_probe {
            standings[index] < standings[best]
        } else {
            race[index] < race[best]
        };
        if better {
            best = index;
        }
    }
    // Without the route probe the pick needs a margin over the
    // first-ranked placement (`race_pick`).
    let (chosen, confident, reason) = if route_probe {
        (
            best,
            true,
            if ripped {
                "the route probe ranks by the ripped count".to_string()
            } else {
                "the route probe ranks by its standing".to_string()
            },
        )
    } else if ripped {
        let counts: Vec<usize> = ripped_counts.iter().map(|count| count.0).collect();
        race_pick(&counts, best, labels, true)
    } else {
        race_pick(race, best, labels, false)
    };
    (best, chosen, confident, reason)
}

/// The kept placement's first route, resumed from its probe (the router
/// armed already). The negotiation gets at most half of what is left
/// before the deadline: the hard repair after it may take as long as the
/// negotiation did, and a deadline inside the negotiation leaves every
/// net still in conflict open (video: 195 nets in conflict when the wall
/// clock ended its repair after 26 to 98 of them, at a point that
/// depended on the load). Books its work.
fn first_route_from_probe(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    router: &mut core::router::Router,
    core_config: &core::Config,
    probe_seconds: f64,
) -> core::RoutingResult {
    let left = ledger.left(ctx.work_deadline).max(0.0);
    // (The router's negotiation budget in work: its seconds are the
    // wall-clock guard, `GUARD` times that.)
    let budget = core_config.negotiation_expansions as f64 / core::router::expansions_per_second();
    let mut seconds = (budget - probe_seconds).max(60.0);
    if seconds > left / 2.0 {
        seconds = (left / 2.0).max(1.0);
        eprintln!(
            "layout: the first route negotiates on for at most {seconds:.0} s of work ({left:.0} s left before the deadline)"
        );
    }
    let before = router.expansions();
    crate::mem_profile::phase("first route");
    let result = router.resume_polished(seconds, false);
    ledger.work +=
        router.expansions().saturating_sub(before) as f64 / core::router::expansions_per_second();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standing(open_terminals: usize, unfinished: usize) -> core::router::ProbeStanding {
        core::router::ProbeStanding {
            open_terminals,
            unfinished,
            conflicted: 0,
        }
    }

    /// Probes without routers (the scores and the pick read none).
    fn probes(
        unfinished: &[usize],
        standings: &[core::router::ProbeStanding],
        ripped_counts: &[(usize, usize)],
    ) -> RaceProbes {
        RaceProbes {
            parked: Vec::new(),
            unfinished: unfinished.to_vec(),
            standings: standings.to_vec(),
            results: vec![None; unfinished.len()],
            ripped_counts: ripped_counts.to_vec(),
            cost: KiCadRaceRippedCost {
                copy_seconds: 0.0,
                rip_seconds: 0.0,
                copy_mb: 0.0,
            },
        }
    }

    #[test]
    fn the_race_picks_by_unfinished_nets_with_a_margin_over_the_first_ranked() {
        let standings = [standing(9, 9); 4];
        let clear = probes(&[20, 15, 18, 5], &standings, &[]);
        let (best, chosen, confident, _) = pick_placement(&clear, &[0, 3, 1, 2], false, false);
        assert_eq!((best, chosen, confident), (3, 3, true));
        // 3 % fewer: the probes do not separate them, the first-ranked
        // routes on.
        let close = probes(&[100, 97, 99], &standings[..3], &[]);
        let (best, chosen, confident, reason) = pick_placement(&close, &[0, 1, 2], false, false);
        assert_eq!((best, chosen, confident), (1, 0, false));
        assert!(reason.contains("first-ranked kept"), "{reason}");
    }

    #[test]
    fn the_ripped_count_breaks_ties_by_the_standing_and_the_route_probe_takes_its_best() {
        let standings = [standing(5, 5), standing(4, 9), standing(4, 3)];
        let ripped = probes(&[1, 1, 1], &standings, &[(10, 1), (8, 2), (8, 2)]);
        let (best, chosen, confident, _) = pick_placement(&ripped, &[0, 1, 2], true, false);
        assert_eq!((best, chosen, confident), (2, 2, true));
        // The route probe ranks by the standing alone, no margin.
        let route = probes(&[1, 1, 1], &standings, &[]);
        let (best, chosen, confident, reason) = pick_placement(&route, &[0, 1, 2], false, true);
        assert_eq!((best, chosen, confident), (2, 2, true));
        assert_eq!(reason, "the route probe ranks by its standing");
    }

    #[test]
    fn the_report_marks_the_probes_the_old_gate_would_have_dropped() {
        let config = KiCadBoardLayoutConfig {
            total_seconds: 100.0,
            ..KiCadBoardLayoutConfig::default()
        };
        let race = probes(
            &[20, 15, 18, 5],
            &[
                standing(1, 20),
                standing(2, 15),
                standing(3, 18),
                standing(4, 5),
            ],
            &[],
        );
        // 60 s of work with every probe charged, 30 s before the others:
        // the gate (40 s) passes after the second other probe.
        let (scores, kept_gated) = score_probes(
            &config,
            60.0,
            &race,
            &[10.0, 10.0, 15.0, 5.0],
            &[0, 3, 1, 2],
            75.0,
        );
        let gated: Vec<bool> = scores.iter().map(|score| score.gated).collect();
        assert_eq!(gated, vec![false, false, false, true]);
        // The gate would have kept placement 3 (15 unfinished), not the
        // dropped probe's 5.
        assert_eq!(kept_gated, 3);
        let placements: Vec<usize> = scores.iter().map(|score| score.placement).collect();
        assert_eq!(placements, vec![0, 3, 1, 2]);
        assert_eq!(scores[2].open_terminals, 3);
        assert_eq!(scores[1].work_seconds, 10.0);
        assert_eq!(scores[0].ripped_open_terminals, None);
    }
}
