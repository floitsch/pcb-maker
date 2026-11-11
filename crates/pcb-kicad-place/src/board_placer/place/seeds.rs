// Copyright (C) 2026 Toit contributors.

//! The seeds placed in a pool, ranked by the placer's key, and the pick:
//! the best placement, the other legal ones as alternatives, and what
//! the race reports.

use super::*;

/// The race over skeletons as the board runs it (`skeleton_race`): its
/// settings, the passives by the class rule, the share of plain seeds,
/// and whether the placement's work reaches the race's least
/// (`min_work_seconds`).
pub(super) struct SkeletonRace {
    pub settings: core::skeleton::Settings,
    pub passive: Vec<bool>,
    pub plain_share: f64,
    pub budget_ok: bool,
}

/// The race over skeletons where it is on and the board is placed whole
/// (not as a panel's copies or held sub-boards): the passives by the
/// class rule; its seeds by skeleton where the placement's work reaches
/// the race's least (`min_work_seconds`), else plain, as with the switch
/// off.
pub(super) fn skeleton_race(
    context: &PlaceContext,
    pcb: &Expr,
    lowered: &LoweredPlacement,
    pieces: &BoardPieces,
) -> Option<SkeletonRace> {
    let config = &context.config;
    config
        .skeleton_race
        .as_ref()
        .filter(|_| pieces.copies.is_none() && pieces.held_sub_boards.is_none())
        .map(|race| SkeletonRace {
            settings: race.settings(),
            passive: passive_mask(pcb, lowered.problem.components.len()),
            plain_share: race.plain_share,
            budget_ok: skeleton::budget_ok(config.work_seconds, race.min_work_seconds),
        })
}

/// The placement work the race booked: the kept seeds' work as the
/// clock of the reference machine sees it (`REFERENCE_THREADS` at once)
/// and in all, and the seeds run.
#[derive(Clone, Copy)]
pub(super) struct SeedWork {
    pub pool: u64,
    pub seeds: u64,
    pub run: u64,
}

/// The kept seeds, best first (`rank`), and the work they booked.
pub(super) struct SeedsPlaced {
    pub kept_seeds: Vec<u64>,
    pub placements: Vec<core::Placement>,
    pub work: SeedWork,
}

/// Places the board's seeds in a pool and ranks the kept ones.
pub(super) fn place_seeds(
    context: &PlaceContext,
    lowered: &LoweredPlacement,
    placer_config: &core::Config,
    pieces: &BoardPieces,
    skeleton_race: Option<&SkeletonRace>,
) -> SeedsPlaced {
    let config = &context.config;
    let problem = &lowered.problem;
    // Several seeds, in parallel; the placement with the fewest illegal
    // parts, then the least missed constraints, then the least wirelength
    // wins. Placement takes seconds; the constraints are what the user asked
    // for.
    let seeds = config.tuning.placement_seeds.max(1) as u64;
    // The seeds share the placement's work budget: seed k is kept only
    // while the seeds before it, in seed order, used less than the budget
    // (three seeds at least). They run in a pool (`race_seeds`), as many
    // at once as the process has search threads; which seeds run and
    // which are kept depends on their work alone, not on the threads or
    // the wall clock (SmartSpin2k: sixteen seeds took 1218 s of placement
    // on four cores and left the route no time).
    let minimum_seeds = seeds.min(3);
    let threads = rayon::current_num_threads().max(1);
    let race = race_seeds(
        seeds,
        minimum_seeds,
        placer_config.work_limit,
        threads,
        |placement: &core::Placement| placement.work,
        |offset| {
            let mut seeded = placer_config.clone();
            seeded.global.seed = config.tuning.seed + offset;
            seeded.anneal.seed = seeded.anneal.seed.wrapping_add(offset);
            // The pad-exits repair is a whole board's (its test
            // names parts by the board's indices): not inside a
            // panel's copies or a held sub-board.
            if pieces.copies.is_some() || pieces.held_sub_boards.is_some() {
                seeded.pad_exits = None;
            }
            match (&pieces.copies, &pieces.held_sub_boards) {
                (Some(copies), _) => panel::place_copies(problem, copies, &seeded),
                (None, Some(sub)) => crate::sub_boards::place_sub_boards(problem, sub, &seeded),
                (None, None) => match skeleton_race {
                    Some(race)
                        if race.budget_ok && !skeleton::plain_seed(offset, race.plain_share) =>
                    {
                        core::skeleton::place_skeleton(
                            problem,
                            &seeded,
                            &race.passive,
                            &lowered.poured,
                            &race.settings,
                        )
                    }
                    // The plain seeds of a mixed race (`plain_share`)
                    // place as with the switch off.
                    _ => core::place(problem, &seeded),
                },
            }
        },
    );
    // The pad-exits repair's work (`pad_exits`), booked apart from the
    // seeds' (the seeds kept do not depend on it): on the clock as the
    // reference machine's pool runs it beside them.
    let exits_works: Vec<u64> = race
        .kept
        .iter()
        .filter_map(|(_, placement)| placement.exits.as_ref().map(|outcome| outcome.work))
        .collect();
    let work = SeedWork {
        pool: race.pool_work + seed_race::pool_makespan(&exits_works, REFERENCE_THREADS),
        seeds: race
            .kept
            .iter()
            .map(|(_, placement)| placement.work)
            .sum::<u64>()
            + exits_works.iter().sum::<u64>(),
        run: race.run,
    };
    let mut placements: Vec<(u64, core::Placement)> = race
        .kept
        .into_iter()
        .map(|(offset, placement)| (config.tuning.seed + offset, placement))
        .collect();
    sort_by_rank(&mut placements);
    let (kept_seeds, placements) = placements.into_iter().unzip();
    SeedsPlaced {
        kept_seeds,
        placements,
        work,
    }
}

/// The placer's key of a placement, the smaller the better: its parts
/// without a legal place, its pads closed, whether it needed courtyards
/// touching, its parts on the board's marks, its missed constraints, its
/// wirelength.
pub(super) fn rank(placement: &core::Placement) -> (usize, usize, usize, usize, f64, f64) {
    let missed: f64 = placement
        .constraints
        .iter()
        .map(|status| status.violation)
        .sum();
    // Pads another part or the edge closes (`pad_exits`; none
    // with it off) count after the illegal parts.
    let closed = placement.exits.as_ref().map_or(0, |outcome| {
        outcome.closed.iter().filter(|pad| !pad.own_only).count()
    });
    (
        placement.unplaced.len() + placement.illegal.len(),
        closed,
        placement.relaxation.courtyard_spacing as usize,
        placement.relaxation.on_marks.len(),
        missed,
        placement.wirelength_final,
    )
}

/// Sorts the seeds' placements by `rank`. A placement that needed
/// courtyards touching (the last relaxation level) is a last resort: it
/// ranks below every legal one that did not, however short its wires.
pub(super) fn sort_by_rank(placements: &mut [(u64, core::Placement)]) {
    // Stable: a tie goes to the earlier seed.
    placements.sort_by(|(_, a), (_, b)| {
        rank(a)
            .partial_cmp(&rank(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// The pick (`pick`): the best placement and what the report says of the
/// race.
pub(super) struct Picked {
    pub placement: core::Placement,
    /// The seed of the placement picked.
    pub seed_kept: u64,
    /// The longest kept seed's work, in seconds (the budget's measure).
    pub work_seconds: f64,
    pub seeds_placed: usize,
    pub work: SeedWork,
    pub skeleton_race: Option<KiCadSkeletonRaceReport>,
    pub panel_copies: Option<KiCadPanelCopiesReport>,
    /// The other legal placements and their pads without a way out.
    pub alternatives: Vec<(Vec<core::Pose>, core::Relaxation, f64)>,
    pub alternative_exits: Vec<Option<exits::KiCadPadExitsReport>>,
}

/// Takes the best placement (the first by `rank`), says how the race over
/// skeletons went, and keeps the other legal placements as alternatives.
pub(super) fn pick(
    context: &PlaceContext,
    pcb: &Expr,
    lowered: &LoweredPlacement,
    pieces: &BoardPieces,
    skeleton_race: Option<&SkeletonRace>,
    seeds: SeedsPlaced,
) -> Picked {
    let config = &context.config;
    let SeedsPlaced {
        kept_seeds,
        mut placements,
        work,
    } = seeds;
    let work_seconds = placements
        .iter()
        .map(|placement| placement.work)
        .max()
        .unwrap_or(0) as f64
        / core::work_per_second();
    let seeds_placed = placements.len();
    let skeleton_race_report = skeleton_race.map(|race| {
        KiCadSkeletonRaceReport::of(
            config.skeleton_race.as_ref().expect("the race's settings"),
            config.work_seconds,
            &race.passive,
            &placements[0],
            &placements.iter().collect::<Vec<_>>(),
        )
    });
    match &skeleton_race_report {
        Some(report) => eprintln!("placer: skeleton race: {}", report.summary),
        // On, but a panel's copies or held sub-boards place each piece on
        // its own: the race is a whole board's.
        None if config.skeleton_race.is_some() => eprintln!(
            "placer: skeleton race: not run: the board is placed as {} (the race places a \
             whole board); board-placer.json has no skeleton_race",
            if pieces.copies.is_some() {
                "a panel's copies"
            } else {
                "a panel's sub-boards"
            }
        ),
        None => {}
    }
    let placement = placements.remove(0);
    let panel_copies = pieces
        .copies
        .as_ref()
        .map(|copies| KiCadPanelCopiesReport::of(&lowered.problem, copies, &placement));
    // The other legal placements, for layout to race when the best one by
    // wirelength does not route: wirelength is not routability.
    let (alternatives, alternative_exits): (Vec<_>, Vec<_>) = placements
        .into_iter()
        .filter(|other| other.unplaced.is_empty() && other.illegal.is_empty())
        .map(|other| {
            let missed = other
                .constraints
                .iter()
                .map(|status| status.violation)
                .sum();
            let exits = other
                .exits
                .as_ref()
                .map(|outcome| exits::exits_report(outcome, &lowered.references, pcb));
            ((other.poses, other.relaxation, missed), exits)
        })
        .unzip();
    Picked {
        placement,
        seed_kept: kept_seeds[0],
        work_seconds,
        seeds_placed,
        work,
        skeleton_race: skeleton_race_report,
        panel_copies,
        alternatives,
        alternative_exits,
    }
}
