// Copyright (C) 2026 Toit contributors.

//! The kept routes finished in place: the first route resumed at the full
//! patience, then polished (clean-up, via reduction, thermal repair), each
//! kept only where it does not leave more open.

use super::moves::Routing;
use super::setup::{LayoutContext, WorkLedger};
use super::*;

/// Connections the first route and the moves left open: before the final
/// ladder starts over, the router goes on from where it is, at the full
/// patience, with a small share of what is left (PolyKybd right: the
/// first route left 33 open after 361 s; the ladder's rungs, started from
/// scratch with the rest, ended at 538 open and the first route stood;
/// the resume took 35 to 32, jetson 26 to 22). A small share: on the
/// laptop motherboard 493 s of it gained nothing, and the ladder, which
/// had completed that board to 2 unconnected, ran out of time. With many
/// open the pour mode is in question: the ladder's. Kept where it leaves
/// no more open (`board` the router's); its work booked either way.
pub(super) fn resume_first_route(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    routing: &mut Routing,
    board: &core::Board,
    many_open: bool,
) {
    let config = ctx.config;
    let router = &mut routing.router;
    ctx.arm(router, ledger.clock());
    if !(routing.best.0 > 0
        && !many_open
        && !router.past_deadline()
        && ctx.stop.allows(LayoutStep::Resume))
    {
        return;
    }
    let left = (config.total_seconds - ledger.work).max(0.0);
    let share = (0.2 * left).min(240.0);
    if share < 60.0 {
        return;
    }
    // A resume that ends worse (MokyaLora: 21 open to 89) must
    // not leave its state behind for the polish: the router is
    // restored with the result.
    crate::mem_profile::phase("resume");
    let saved_router = router.saved_copy();
    let expansions_before = router.expansions();
    ctx.arm(router, ledger.clock());
    {
        let defaults = core::router::Config::default();
        let full = router.config_mut();
        full.stall_patience = defaults.stall_patience;
        full.stall_at_cap = ctx
            .router_config
            .stall_at_cap
            .unwrap_or(defaults.stall_at_cap);
        full.stall_drop = ctx.router_config.stall_drop.unwrap_or(defaults.stall_drop);
    }
    let resumed = router.resume_polished(share, false);
    ledger.work += router.expansions().saturating_sub(expansions_before) as f64
        / core::router::expansions_per_second();
    let (before, after) = (score(&routing.result, board), score(&resumed, board));
    eprintln!(
        "layout: the first route resumed with {share:.0} s: {} open (before {}), {:.0} s",
        after.0,
        before.0,
        ctx.elapsed()
    );
    // (`best` is not read past this point.)
    if after <= before {
        routing.result = resumed;
    } else {
        router.restore_saved(saved_router);
    }
}

/// The polish of the kept routes (clean-up, via reduction, thermal
/// repair), unless a move trial polished them (`polished`) and less than
/// half the budget is left, or many connections are open. It reroutes
/// everything: past the deadline it would stop at once and throw the
/// moves' best board away (OpenAirScope: 18 open after the moves, 108 of
/// 184 routed after the polish). It is bounded by the work left before
/// the layout's nominal end (the wall clock would make the polish depend
/// on the machine's load; its deadline still guards the polish inside the
/// router), and skipped under four minutes of it: PolyKybd right's polish
/// spent 769 s on one via reduction round and was thrown away. Kept where
/// it leaves no more open; its work booked either way.
pub(super) fn polish_routes(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    routing: &mut Routing,
    board: &core::Board,
    polished: bool,
    many_open: bool,
) {
    let config = ctx.config;
    let router = &mut routing.router;
    // What is left before the layout's nominal end, in work.
    let polish_left = ledger.left(ctx.work_deadline);
    // A board a move trial already polished polishes again when more than
    // half the layout's budget is left: the trial's via reduction had only
    // its own strict budget (CyberKeeb2040: a round reverted at 60 s, the
    // next not started), the final one gets the run-sized one.
    let polish_again = polished && ledger.work < 0.5 * config.total_seconds;
    if polish_again {
        eprintln!(
            "layout: polished by a move trial, {:.0} s of {:.0} s of work left: polishing again",
            config.total_seconds - ledger.work,
            config.total_seconds
        );
    }
    ctx.arm(router, ledger.clock());
    if !((!polished || polish_again)
        && !many_open
        && !router.past_deadline()
        && polish_left >= 240.0
        && ctx.stop.allows(LayoutStep::Polish))
    {
        return;
    }
    crate::mem_profile::phase("polish");
    let saved_router = router.saved_copy();
    let expansions_before = router.expansions();
    {
        let own = expansions_before as f64 / core::router::expansions_per_second();
        let polish = router.config_mut();
        polish.via_reduction_seconds = polish
            .via_reduction_seconds
            .min((polish_left - 120.0) / 2.0);
        // What is left of the layout's budget, its router's own work
        // counted against it.
        polish.run_work = Some((
            config.total_seconds,
            (config.total_seconds - ledger.work).max(0.0) + own,
        ));
        polish.cleanup_seconds = polish.cleanup_seconds.min((polish_left - 120.0) / 4.0);
    }
    let polished_result = router.polish();
    ledger.work += router.expansions().saturating_sub(expansions_before) as f64
        / core::router::expansions_per_second();
    // A polish the deadline cut short can leave more open than it
    // found (Sisu: 146 open before it, 255 after): then the board stays
    // as the moves left it.
    let (before, after) = (
        score(&routing.result, board),
        score(&polished_result, board),
    );
    if after.0 <= before.0 {
        routing.result = polished_result;
    } else {
        eprintln!(
            "layout: the polish left {} open (before it {}): kept the unpolished routes",
            after.0, before.0
        );
        router.restore_saved(saved_router);
    }
}
