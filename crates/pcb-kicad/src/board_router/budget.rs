// Copyright (C) 2026 Toit contributors.

//! The ladder's work budget: the work kept for the narrow-signal step,
//! and which attempts KiCad checks (`CheckGate`).

use super::*;

/// Whether the ladder's final step applies to this project: signals at
/// the project's narrowest predefined width (narrower than some class), or
/// the board's minimum clearance where the project rates clearance
/// findings as warnings (smaller than some class's clearance).
pub(crate) fn narrow_step_applies(config: &KiCadBoardRouterConfig) -> bool {
    let rules = || {
        config
            .connection_rules
            .values()
            .chain(config.default_rules.iter())
    };
    let narrow = config.use_narrow_signals.is_none()
        && config
            .narrow_signal_mm
            .is_some_and(|narrow| rules().any(|rules| rules.trace_width_mm > narrow + 1.0e-9));
    let relaxed = config
        .relaxed_clearance_mm
        .is_some_and(|relaxed| rules().any(|rules| rules.clearance_mm > relaxed + 1.0e-9));
    narrow || relaxed
}

/// The least work kept for the narrow-signal step when the reserve is a
/// floor: the step's own minimum (`narrow_step_minimum`, 180 s) and its
/// clean-up at its negotiation's floor (60 s). TelemetryOnboard (v16)
/// kept 120 s, had 153 s left, and the step that completes it never ran.
pub(crate) const NARROW_RESERVE_FLOOR: f64 = 240.0;

/// Work kept back for the final narrow-signal step, from the ladder's
/// first attempt's work: a share of it, at least `NARROW_RESERVE_FLOOR`,
/// at most `cap`, and none at all when the cap leaves less than the step
/// needs to run (`narrow_step_minimum`): a reserve the step can never use
/// would only take work from the rungs.
pub(crate) fn narrow_step_reserve(first_attempt_work: f64, share: f64, cap: f64) -> f64 {
    if share <= 0.0 {
        return 0.0;
    }
    let kept = (first_attempt_work * share)
        .max(NARROW_RESERVE_FLOOR)
        .min(cap);
    if kept < narrow_step_minimum(kept) {
        0.0
    } else {
        kept
    }
}

/// The work the narrow-signal step needs left to run, with `kept` kept
/// for it: half of that, and at least three times its negotiation's floor
/// of 60 s (repair and polish take up to twice the negotiation again).
pub(crate) fn narrow_step_minimum(kept: f64) -> f64 {
    (0.5 * kept).max(180.0)
}

/// The narrow-signal attempt's negotiation in work seconds when the ladder
/// kept work for it: a third of what is left (repair and polish take up
/// to twice the negotiation again), within 60 to 900 s, and never more
/// than the attempt it varies had.
pub(crate) fn narrow_step_negotiation(work_left: f64, own: Option<f64>) -> f64 {
    let share = (work_left / 3.0).clamp(60.0, 900.0);
    own.map_or(share, |own| own.min(share))
}

/// `narrow_step_negotiation`, at least what the ladder's first attempt
/// negotiated (`first`), bounded by the work kept for the step and by
/// half of what is left (the hard repair after the negotiation needs as
/// much: a deadline inside the negotiation leaves every net in conflict
/// open). DSP-ADAU1452's step, varying a finer rung with a smaller
/// budget, got less negotiation than the first attempt had.
pub(crate) fn narrow_step_negotiation_floored(
    work_left: f64,
    own: Option<f64>,
    first: f64,
    kept: f64,
) -> f64 {
    narrow_step_negotiation(work_left, own).max(first.min(kept).min(0.5 * work_left))
}

/// The work kept for the narrow-signal step with its own KiCad check:
/// `reserve` (the step's routing) and one check's work (`check`) on top
/// when anything is kept at all.
pub(crate) fn reserve_with_check(reserve: f64, check: f64) -> f64 {
    if reserve > 0.0 { reserve + check } else { 0.0 }
}

/// Whether an attempt projected to take `cost` of work may start with
/// `work_left` left and `reserve` kept for the narrow-signal step: only if
/// the reserve stays whole after it (an attempt counts at least 120 s).
pub(crate) fn fits_before_reserve(work_left: f64, reserve: f64, cost: f64) -> bool {
    work_left - reserve >= cost.max(120.0)
}

/// The work an attempt on a lattice of `pitch` mm is projected to take
/// from one of `work` on `previous` mm: a lattice grows with the square of
/// the pitch ratio.
pub(crate) fn projected_attempt_work(work: f64, previous: f64, pitch: f64) -> f64 {
    if pitch > 0.0 && pitch < previous {
        work * (previous / pitch).powi(2)
    } else {
        work
    }
}

/// The work deadline of each of `routers` routers whose work counts
/// together (seeds side by side) when the work kept for the narrow-signal
/// step may not be touched: their share of what is left beyond it.
pub(crate) fn deadline_before_reserve(work_left: f64, reserve: f64, routers: usize) -> f64 {
    ((work_left - reserve) / routers.max(1) as f64).max(0.0)
}

/// Route mode (no work deadline): how far the ladder's whole work may go
/// beyond `ladder_budget_seconds` (`ladder_work_bound`). Every router the
/// ladder starts stops at its share of what is left under the bound
/// (router `deadline_work`); beyond it go only what a router does past
/// its deadline: a fresh router's first negotiation iteration, the batch
/// or round under way (2026-10-21).
pub(crate) const LADDER_WORK_TOLERANCE: f64 = 0.05;

/// Route mode: the share of the ladder's budget the rung probes take
/// together at most (each `probe_seconds`, 75 s, or less: the 4-layer
/// intake boards probed 4 x 75 s on a 30 s budget).
pub(crate) const PROBE_SHARE: f64 = 0.5;

/// Route mode: no further attempt (rung, seeds, narrow-signal or
/// minimum-clearance step) starts with less than this share of the
/// ladder's budget left under its bound.
pub(crate) const LADDER_MIN_ATTEMPT_SHARE: f64 = 0.05;

/// Route mode: the most work the ladder does, from its budget.
pub(crate) fn ladder_work_bound(budget: f64) -> f64 {
    budget * (1.0 + LADDER_WORK_TOLERANCE)
}

/// Route mode: the work each rung probe negotiates, `probe_seconds` or
/// less, so that `rungs` probes take at most `PROBE_SHARE` of `budget`.
pub(crate) fn bounded_probe_seconds(probe_seconds: f64, budget: f64, rungs: usize) -> f64 {
    probe_seconds.min(PROBE_SHARE * budget / rungs.max(1) as f64)
}

/// Route mode: a negotiation of `share` seconds of work, within 60 to 900
/// s as before, but at most half of the work `left` under the ladder's
/// bound (the repair, clean-up and polish after it need the rest: a
/// 30 s budget's rungs negotiated 60 s each).
pub(crate) fn bounded_negotiation(share: f64, left: f64) -> f64 {
    share.clamp(60.0, 900.0).min(0.5 * left.max(0.0))
}

/// Route mode: the work deadline of each rung of a pitch after the
/// ladder's first attempt, `left` under the bound split evenly over the
/// `rungs` still to come (decided before they run, side by side or not,
/// so that their work together stays within the bound). The first
/// attempt has all that is left.
pub(crate) fn rung_share(left: f64, rungs: usize) -> f64 {
    left.max(0.0) / rungs.max(1) as f64
}

/// Route mode: whether another attempt may start with `left` of the
/// ladder's bound and a budget of `budget`.
pub(crate) fn bound_allows_attempt(left: f64, budget: f64) -> bool {
    left >= LADDER_MIN_ATTEMPT_SHARE * budget
}

/// Which attempts of a ladder KiCad checks. An attempt's quality is the
/// router's open count (unconnected terminals and internal violations)
/// plus KiCad's unconnected items, then KiCad's starved thermals, then
/// vias and copper (`open` in `route_kicad_board`); the router's count
/// alone is a lower bound of the first, with no slack (KiCad's part only
/// adds). An attempt whose router count already loses to the kept board's
/// checked quality cannot win, and its check (10 to 50 s of kicad-cli on a
/// big or hatched board, not counted as work) is skipped. In the layout's
/// final ladder (`top`), only the attempts among the best `top` by the
/// router's measures (its open count, then vias, then copper), the
/// layout's own board counted, are checked at all; KiCad decides between
/// the attempts checked. Once the wall clock passes the deadline,
/// nothing after an attempt depends on it but whether it beats the kept
/// board and the caller's (`kept`).
#[derive(Debug, Default)]
pub(crate) struct CheckGate {
    /// At most this many attempts by the router's measures are checked
    /// (None: any that could win).
    pub top: Option<usize>,
    /// The router's measures of the boards checked (the caller's first).
    pub counts: Vec<RouterMeasure>,
    /// The caller's kept board as it compares a ladder's result (KiCad's
    /// open count with the router's, starved thermals).
    pub kept: Option<(usize, usize)>,
    pub checked: usize,
    pub skipped: usize,
}

/// A board as the router alone measures it: its open count (unconnected
/// terminals and internal violations), vias, copper length.
pub(crate) type RouterMeasure = (usize, usize, f64);

/// A checked board's quality as the ladder compares boards: (open, starved
/// thermals), vias, copper length.
pub(crate) type CheckedQuality = ((usize, usize), usize, f64);

/// Whether an attempt with router open count `count`, `vias` and `length_mm`
/// can still beat `best` (an attempt's full open count is at least
/// `count`). A tie on the open count leaves starved thermals and then
/// vias and copper to decide.
pub(crate) fn could_win(count: usize, vias: usize, length_mm: f64, best: CheckedQuality) -> bool {
    let ((open, starved), best_vias, best_length) = best;
    count < open
        || (count == open
            && (starved > 0
                || (vias, length_mm)
                    .partial_cmp(&(best_vias, best_length))
                    .is_some_and(|order| order.is_lt())))
}

/// Whether an attempt with router open count `count` can beat the caller's
/// kept board `(open, starved)` as the layout compares them.
pub(crate) fn could_beat_kept(count: usize, kept: (usize, usize)) -> bool {
    count < kept.0 || (count == kept.0 && kept.1 > 0)
}

impl CheckGate {
    /// Whether KiCad should check an attempt (router count `count`) given
    /// the ladder's `best` so far; `finishing` once the wall clock passed
    /// the deadline (no further attempt follows).
    pub(crate) fn wants(
        &self,
        count: usize,
        vias: usize,
        length_mm: f64,
        best: Option<CheckedQuality>,
        finishing: bool,
    ) -> bool {
        if finishing {
            return best.is_none_or(|best| could_win(count, vias, length_mm, best))
                && self.kept.is_none_or(|kept| could_beat_kept(count, kept));
        }
        let Some(best) = best else {
            // The ladder's first board is its measure.
            return true;
        };
        // A board the router finishes is always checked: whether KiCad
        // agrees decides whether the ladder stops.
        let measure = (count, vias, length_mm);
        count == 0
            || (could_win(count, vias, length_mm, best)
                && self.top.is_none_or(|top| {
                    self.counts
                        .iter()
                        .filter(|other| {
                            other
                                .partial_cmp(&&measure)
                                .is_some_and(|order| order.is_le())
                        })
                        .count()
                        < top
                }))
    }

    /// Notes a board checked with the router's measures `measure`.
    pub(crate) fn record(&mut self, measure: RouterMeasure) {
        self.counts.push(measure);
        self.checked += 1;
    }
}

/// Attempts whose KiCad check this process skipped (`CheckGate`).
pub(super) static CHECKS_SKIPPED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// How many attempts' KiCad checks this process skipped (`CheckGate`).
pub(crate) fn checks_skipped() -> usize {
    CHECKS_SKIPPED.load(std::sync::atomic::Ordering::Relaxed)
}

/// The router's own open count of an attempt: unconnected terminals and
/// internal violations, a lower bound of its full open count.
pub(crate) fn router_open(result: &KiCadBoardRouterResult) -> usize {
    result.unconnected_terminals + result.internal_violations.len()
}

/// KiCad's check of an attempt routed without it
/// (`skip_native_verification`), as `finish_routed_board` runs it: the
/// native report, the starved thermals, the report file again.
pub(crate) fn check_attempt(
    result: &mut KiCadBoardRouterResult,
    directory: &Path,
    board_id: &str,
) -> Result<(), String> {
    let started = std::time::Instant::now();
    result.native = Some(verify_materialized_rung(directory, board_id)?);
    result.native_verification_seconds = started.elapsed().as_secs_f64();
    result.diagnostics.starved_thermals = starved_thermal_diagnostics(directory);
    let drc_cache = crate::drc_cache_counts();
    eprintln!(
        "board check: KiCad {:.1}s (router count {}); drc_cache: {{calls {}, hits {}}}",
        result.native_verification_seconds,
        router_open(result),
        drc_cache.calls,
        drc_cache.hits
    );
    let report_path = directory.join("board-router.json");
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&*result).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", report_path.display()))
}

/// `config` as the narrow-signal step routes it: signal nets at the
/// project's narrowest predefined width when that is narrower than a
/// class, and the board's minimum clearance when the project rates
/// clearance findings as warnings. Neither step applies to the result
/// again.
pub(crate) fn narrow_step_config(config: &KiCadBoardRouterConfig) -> KiCadBoardRouterConfig {
    let mut step = config.clone();
    let wider = |narrow: f64| {
        config
            .connection_rules
            .values()
            .chain(config.default_rules.iter())
            .any(|rules| rules.trace_width_mm > narrow + 1.0e-9)
    };
    if config.use_narrow_signals.is_none() {
        step.use_narrow_signals = Some(config.narrow_signal_mm.is_some_and(wider));
    }
    if let Some(relaxed) = config.relaxed_clearance_mm {
        step.relaxed_clearance_mm = None;
        for rules in step
            .connection_rules
            .values_mut()
            .chain(step.default_rules.iter_mut())
        {
            rules.clearance_mm = rules.clearance_mm.min(relaxed);
        }
    }
    step
}
