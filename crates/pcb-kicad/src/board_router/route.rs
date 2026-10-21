// Copyright (C) 2026 Toit contributors.

//! `route_kicad_board`: the attempt ladder, its seeds and the router's
//! configuration, and the finish of the board kept.

use super::*;
use pcb_router as core;

/// Routes every connection of `<source>/<board_id>.kicad_pcb` and writes the
/// complete project to `output`.
pub fn route_kicad_board(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardRouterConfig,
) -> Result<KiCadBoardRouterResult, String> {
    let with_deadline;
    let config = match (config.deadline, config.deadline_seconds) {
        (None, Some(seconds)) => {
            with_deadline = KiCadBoardRouterConfig {
                deadline: Some(
                    std::time::Instant::now()
                        + std::time::Duration::from_secs_f64(seconds.max(0.0)),
                ),
                ..config.clone()
            };
            &with_deadline
        }
        _ => config,
    };
    // Requested net classes set their nets' rules.
    let with_classes;
    let config = if config.net_classes.is_empty() {
        config
    } else {
        let board = source_directory.join(format!("{board_id}.kicad_pcb"));
        let text = fs::read_to_string(&board)
            .map_err(|error| format!("failed to read {}: {error}", board.display()))?;
        with_classes = crate::net_classes::with_net_classes(
            config,
            &parse(&text).map_err(crate::verdict::input_error)?,
        )
        .map_err(crate::verdict::input_error)?;
        &with_classes
    };
    // The minimum connection width floors every width, also those of a
    // configuration file or of requested net classes.
    let floored;
    let config = if config.width_floor() > 0.0 {
        let mut copy = config.clone();
        copy.apply_width_floor();
        floored = copy;
        &floored
    } else {
        config
    };
    // With interchangeable pins, the ladder routes a copy with the pins
    // assigned; every attempt copies its project from there.
    // Requested pours and net classes go into a copy of the project that the
    // attempts use.
    let poured_directory = output_directory.with_extension("poured");
    let planned = {
        let board = source_directory.join(format!("{board_id}.kicad_pcb"));
        let text = fs::read_to_string(&board)
            .map_err(|error| format!("failed to read {}: {error}", board.display()))?;
        planned_pours(
            &crate::canonical_order::parse_board(&text).map_err(crate::verdict::input_error)?,
            config,
        )
        .map_err(crate::verdict::input_error)?
    };
    if config.add_pours.is_empty() && !planned.is_empty() {
        eprintln!(
            "automatic planes: {}",
            planned
                .iter()
                .map(|pour| format!("{} on {}", pour.net, pour.layers.join(", ")))
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
    let prepared = !planned.is_empty() || !config.net_classes.is_empty();
    let source_directory = if !prepared {
        source_directory
    } else {
        if poured_directory.exists() {
            fs::remove_dir_all(&poured_directory).map_err(|error| error.to_string())?;
        }
        copy_directory_tree(source_directory, &poured_directory)?;
        let board = poured_directory.join(format!("{board_id}.kicad_pcb"));
        let text = fs::read_to_string(&board)
            .map_err(|error| format!("failed to read {}: {error}", board.display()))?;
        let mut pcb = crate::canonical_order::parse_board(&text)?;
        add_pour_zones(&mut pcb, &planned).map_err(crate::verdict::input_error)?;
        fs::write(&board, format!("{}\n", encode(&pcb)))
            .map_err(|error| format!("failed to write {}: {error}", board.display()))?;
        if !config.net_classes.is_empty() {
            crate::net_classes::write_net_classes(
                &poured_directory,
                board_id,
                &pcb,
                &config.net_classes,
            )?;
        }
        poured_directory.as_path()
    };
    let swapped_directory = output_directory.with_extension("swapped");
    let (source_directory, pin_swaps) = match &config.pin_swaps {
        Some(path) => {
            let spec = read_pin_swap_spec(&source_directory.join(path))
                .map_err(crate::verdict::input_error)?;
            let result = swap_kicad_pins(
                source_directory,
                board_id,
                &swapped_directory,
                &spec,
                &config.pin_swap,
            )?;
            eprintln!(
                "pin swaps: ratsnest {:.0} mm with {} crossings -> {:.0} mm with {} crossings, {} pins changed",
                result.before.length_mm,
                result.before.crossings,
                result.after.length_mm,
                result.after.crossings,
                result.changes.len()
            );
            (swapped_directory.as_path(), Some(result))
        }
        None => (source_directory, None),
    };
    // The topological engine makes one attempt, pour nets as tracks.
    if config.engine.as_deref() == Some("topological") {
        let mut report =
            route_kicad_board_once(source_directory, board_id, output_directory, config, false)?;
        report.pin_swaps = pin_swaps;
        report.pours = "topological (pour nets as tracks)".into();
        return Ok(report);
    }
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    let source = fs::read_to_string(&source_board)
        .map_err(|error| format!("failed to read {}: {error}", source_board.display()))?;
    let parsed = crate::canonical_order::parse_board(&source)?;
    let layer_table = LayerTable::from_pcb(&parsed)?;
    let pour_nets: Vec<String> = pours(&parsed, &layer_table)?
        .into_iter()
        .map(|pour| pour.net)
        .collect();
    let has_pours = !pour_nets.is_empty();
    // Unconnected items as the router and, when asked, KiCad see them,
    // and separately the starved thermals (pads KiCad does not count as
    // connected to their pour): those rank attempts but, being a matter of
    // zone settings as often as of routing, do not call for further rungs.
    let open = |result: &KiCadBoardRouterResult, directory: &Path| {
        (
            result.unconnected_terminals
                + result.internal_violations.len()
                + result
                    .native
                    .as_ref()
                    .map_or(0, |native| native.selected_net_unconnected_items),
            starved_thermals(directory),
        )
    };
    // The attempt ladder: pours connected, then connected with a fixed plane
    // skeleton, then pour nets as tracks; each first on the regular lattice
    // and then on finer ones. It stops at the first clean board and
    // otherwise keeps the one with the fewest opens.
    // A skeleton helps a two-layer board whose pours get shredded by the
    // signals; with inner planes the pours stay whole and the fixed tree
    // only blocks vias.
    let skeleton = config.plane_skeleton.unwrap_or(false);
    let two_layers = layer_table.names.len() == 2;
    // (connect pours, plane skeleton, exclusive planes). On four or more
    // layers, inner planes are first kept free of signals (they stay whole:
    // ColdFire 13 open against 43), then opened to them (video needs the
    // room: 9 open against 110), then pour nets go as tracks.
    let exclusive = config.exclusive_planes;
    // Rungs: (connect pours, plane skeleton, exclusive planes, plane
    // stubs). The stub rung (a via next to every surface pad of a pour
    // net before the signals route; boards with inner planes) runs when
    // the plain pour connection left pour-net pads open, like the
    // skeleton rung: it saves those pads on some boards (Framework
    // mainboard half, GND 7 -> 0) and takes room on others (ColdFire).
    let stubs = config.fixed_plane_stubs;
    let modes: Vec<(bool, bool, bool, bool)> = match (has_pours, config.pours) {
        (false, _) | (true, KiCadPourMode::Tracks) => vec![(false, false, false, false)],
        (true, KiCadPourMode::Connect) => vec![(
            true,
            skeleton,
            exclusive.unwrap_or(false),
            stubs.unwrap_or(false),
        )],
        (true, KiCadPourMode::Auto) if skeleton => {
            vec![(true, true, false, false), (false, false, false, false)]
        }
        (true, KiCadPourMode::Auto) if two_layers => {
            vec![
                (true, false, false, false),
                (true, true, false, false),
                (false, false, false, false),
            ]
        }
        (true, KiCadPourMode::Auto) => {
            let mut modes = match exclusive {
                Some(true) => vec![(true, false, true, false)],
                Some(false) => vec![(true, false, false, false)],
                None => vec![(true, false, true, false), (true, false, false, false)],
            };
            if stubs != Some(false) {
                modes.push((true, false, false, true));
            }
            modes.push((false, false, false, false));
            modes
        }
    };
    let refine = config
        .refine_pitches_mm
        .clone()
        .unwrap_or_else(|| vec![0.075, 0.05]);
    let mut pitches: Vec<Option<Vec<f64>>> = vec![None];
    let coarsest = core_config(config)
        .pitches
        .iter()
        .copied()
        .fold(0.0, f64::max);
    for pitch in refine {
        if pitch < coarsest {
            pitches.push(Some(vec![pitch]));
        }
    }
    let budget = config.refine_budget_seconds.unwrap_or(240.0);
    let scratch = output_directory.with_extension("attempt");
    let mut best: Option<((usize, usize), KiCadBoardRouterResult)> = None;
    // Routing seconds and pitch of the slowest attempt so far, to project
    // the cost of a finer lattice.
    let mut slowest: Option<(f64, f64)> = None;
    // The process's peak memory after the first attempt and that attempt's
    // pitch: about one router of this board (an upper bound: the layout's
    // first route is in the peak too). A finer lattice grows with the
    // square of the pitch ratio, and seeds run side by side (jiran-ble-lite:
    // 3299 x 1048 nodes at 0.1 mm, 7 GB a router at 0.05 mm, four seeds of
    // it took 31 GB and the machine's last free byte).
    let mut router_mb: Option<(f64, f64)> = None;
    // The first attempt's lattice maps and pitch: routers side by side
    // (rungs, seeds) are sized from them (`MemoryLimits`: a router's peak
    // is its maps times a measured ratio and a margin, as many as fit the
    // tightest of the process's limits and half the memory available).
    let mut router_maps: Option<(f64, f64)> = None;
    let memory_at_start = MemoryLimits::read(config.memory_mb);
    eprintln!(
        "memory limits: {} (the process holds {:.0} MB resident)",
        memory_at_start.describe(),
        resident_mb()
    );
    // How many routers of `maps_mb` of maps each run side by side, at
    // most `wanted`, and why, for the log.
    let side_by_side_for = |maps_mb: f64, wanted: usize| -> (usize, String) {
        let limits = MemoryLimits::read(config.memory_mb);
        let (count, limit) = limits.routers_that_fit(maps_mb, wanted);
        (count, fit_note(maps_mb, limit))
    };
    let mut extra_rung_tried = false;
    // Steps that did not run, with why: the report's `skipped` when the
    // board stays incomplete.
    let mut skipped: Vec<String> = Vec::new();
    // KiCad's checks of the attempts: each attempt routes without one, and
    // only those that could still win are checked (`CheckGate`).
    let checking = !config.skip_native_verification;
    // What one check counts as work (none in route mode), and the checks
    // counted so far: every check run, whether the DRC cache answered it
    // or not, so that a warm run ends where a cold one does.
    let check_charge = if checking {
        config.kicad_check_work.unwrap_or(0.0)
    } else {
        0.0
    };
    let checks_charged = std::cell::Cell::new(crate::check_work::CheckTally::new(check_charge));
    let gate = std::cell::RefCell::new(CheckGate {
        top: config.kicad_check_top,
        counts: config
            .kicad_kept
            .map(|(measure, _)| vec![measure])
            .unwrap_or_default(),
        kept: config.kicad_kept.map(|(_, quality)| quality),
        ..CheckGate::default()
    });
    // An attempt's configuration as it routes: without KiCad's check, which
    // `judge` runs when it matters.
    let unchecked = |attempt: &KiCadBoardRouterConfig| -> KiCadBoardRouterConfig {
        let mut attempt = attempt.clone();
        attempt.skip_native_verification = attempt.skip_native_verification || checking;
        attempt
    };
    let ladder_budget = config.ladder_budget_seconds.unwrap_or(1200.0);
    let ladder_started = std::time::Instant::now();
    // The ladder's budget is spent in work (search expansions in seconds of
    // an idle machine), so that a busy machine routes the same way; the
    // wall clock counts only beyond `GUARD` times that.
    // KiCad's checks of the attempts count in it too, at a modelled cost
    // per check that depends only on the board (`kicad_check_work`), so
    // that a check the DRC cache answers counts as one it does not.
    let ladder_work_now = std::cell::Cell::new(0.0f64);
    let charge_checks = |checks: usize| {
        let mut tally = checks_charged.get();
        ladder_work_now.set(ladder_work_now.get() + tally.add(checks));
        checks_charged.set(tally);
    };
    let work_of = |expansions: u64| expansions as f64 / core::router::expansions_per_second();
    // Without a deadline (route mode) the wall clock guards the work at
    // `GUARD` times it; with one, the deadline is the only wall guard. The
    // guard takes over only when the wall clock is ahead by more than a
    // twentieth of the budget: the work counts only search expansions, so
    // the wall clock is always ahead of it at the start of the ladder and
    // after KiCad's checks, and every route-mode run reported a "ladder"
    // cut (0 s of wall clock for 0 s of work) that changed nothing.
    let wall_margin = 0.05 * ladder_budget;
    let spent = |work: f64| {
        if config.deadline.is_some() || !core::router::wall_guard() {
            work
        } else {
            let wall = ladder_started.elapsed().as_secs_f64();
            if wall / core::router::GUARD > work + wall_margin {
                core::router::note_wall_cut(
                    "ladder",
                    format_args!("{wall:.0} s of wall clock for {work:.0} s of the ladder's work"),
                );
                wall / core::router::GUARD
            } else {
                work
            }
        }
    };
    // Route mode (no work deadline): the ladder's whole work (its probes,
    // rungs, seeds, the probed rung's continuation and the steps after,
    // each router's clean-up and polish included) stays within its budget
    // and a tolerance (`ladder_work_bound`): every router stops at its
    // share of what is left under it (router `deadline_work`), and none
    // starts with too little left (`bound_allows_attempt`). Until
    // 2026-10-21 a 30 s budget meant 60 s of negotiation a rung and 75 s
    // a probe: the intake's clean boards spent a median 53 s and the
    // probed ones 120 s, and 279 runs were cut by the wall guard. A
    // layout's ladder (`deadline_work`) is bounded by the layout's end as
    // before.
    let work_bound = config
        .deadline_work
        .is_none()
        .then(|| ladder_work_bound(ladder_budget));
    let bound_left = || work_bound.map(|bound| (bound - spent(ladder_work_now.get())).max(0.0));
    // Route mode: where the narrow-signal (or minimum-clearance) step
    // applies, once the ladder's first attempt leaves connections open,
    // up to that attempt's work is kept for the step (`route_reserve`);
    // the rungs and seeds after it see what is left without it. A complete
    // board releases it. On a 30 s budget facade60's first attempt took
    // 20 s, its two other rungs 4 s each and nothing, and its step (23 to
    // 5 open on x1231's overrun) did not run.
    let route_reserve = std::cell::Cell::new(0.0f64);
    let route_reserve_wanted = work_bound.is_some() && narrow_step_applies(config);
    let mut route_reserve_set = false;
    let rung_left = || bound_left().map(|left| (left - route_reserve.get()).max(0.0));
    let keep_for_route_step = |first_work: f64| {
        let kept = route_step_reserve(first_work, bound_left().unwrap_or(0.0));
        route_reserve.set(kept);
        eprintln!(
            "ladder: {kept:.1} s of work kept for the narrow-signal step ({:.1} s left under the bound, the first attempt took {first_work:.1} s)",
            bound_left().unwrap_or(0.0)
        );
    };
    // Whether the bound leaves room for another rung or seed round
    // (always, in a layout's ladder).
    let bound_allows = || rung_left().is_none_or(|left| bound_allows_attempt(left, ladder_budget));
    if let Some(bound) = work_bound {
        eprintln!(
            "ladder: a budget of {ladder_budget:.0} s of work, at most {bound:.0} s with its tolerance ({:.0} %); wall guard {}",
            LADDER_WORK_TOLERANCE * 100.0,
            match config.deadline_seconds {
                Some(seconds) if config.deadline.is_some() => {
                    format!("at {seconds:.0} s of wall clock (deadline_seconds)")
                }
                _ if config.deadline.is_some() => "at the caller's deadline".to_string(),
                _ if core::router::wall_guard() => format!(
                    "at {} times the work, beyond a margin of {wall_margin:.0} s",
                    core::router::GUARD
                ),
                _ => "off".to_string(),
            }
        );
    }
    let past_deadline = || {
        let past = config
            .deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline);
        if past {
            core::router::note_wall_cut(
                "deadline",
                format_args!(
                    "the ladder at {:.0} s of its work{}",
                    ladder_work_now.get(),
                    config
                        .deadline_seconds
                        .map(|seconds| format!(
                            ", the wall guard at {seconds:.0} s (deadline_seconds)"
                        ))
                        .unwrap_or_default()
                ),
            );
        }
        past
    };
    // An attempt's (open, starved) as KiCad checked it, or None when its
    // check was skipped because it cannot win (`CheckGate`; its router
    // count, a lower bound of its open count, is then what it is ranked by).
    let judge = |result: &mut KiCadBoardRouterResult,
                 directory: &Path,
                 best: Option<CheckedQuality>|
     -> Result<Option<(usize, usize)>, String> {
        if !checking {
            return Ok(Some(open(result, directory)));
        }
        let count = router_open(result);
        // Past the deadline no further attempt starts: only whether this
        // one beats the kept boards matters.
        let finishing = past_deadline();
        if !gate
            .borrow()
            .wants(count, result.vias, result.length_mm, best, finishing)
        {
            gate.borrow_mut().skipped += 1;
            CHECKS_SKIPPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let winnable =
                best.is_none_or(|best| could_win(count, result.vias, result.length_mm, best));
            eprintln!(
                "board check: KiCad skipped, the attempt {} (router count {count}, {} vias; best {}{})",
                if winnable && !finishing {
                    "is not among the best by the router's measures"
                } else {
                    "cannot win"
                },
                result.vias,
                best.map_or("none".to_string(), |((open, starved), vias, _)| format!(
                    "{open} open, {starved} starved, {vias} vias"
                )),
                if finishing { ", past the deadline" } else { "" }
            );
            return Ok(None);
        }
        check_attempt(result, directory, board_id)?;
        charge_checks(1);
        gate.borrow_mut()
            .record((count, result.vias, result.length_mm));
        Ok(Some(open(result, directory)))
    };
    let quality_of =
        |best: &Option<((usize, usize), KiCadBoardRouterResult)>| -> Option<CheckedQuality> {
            best.as_ref()
                .map(|(opens, result)| (*opens, result.vias, result.length_mm))
        };
    // How an attempt's line names an open count KiCad did not check.
    let by_router = |judged: &Option<(usize, usize)>| {
        if judged.is_none() {
            " (router's count, not checked)"
        } else {
            ""
        }
    };
    // What the last attempt took, in work: no attempt starts with less
    // than that left before the run's nominal end (an attempt checks the
    // deadline only in its negotiation; MokyaLora's last rung started at
    // 1560 s and ran to 1890 s, past the harness's limit).
    let last_attempt_work = std::cell::Cell::new(120.0f64);
    // Its lattice pitch (0: none yet), for projecting a finer rung.
    let last_attempt_pitch = std::cell::Cell::new(0.0f64);
    let work_left = || {
        config
            .deadline_work
            .map_or(f64::INFINITY, |left| left - ladder_work_now.get())
    };
    // The final narrow-signal (or minimum-clearance) step comes after
    // every rung and seed round; with a work deadline (the layout's
    // ladder, whose deadline is barely beyond its budget) the rungs used
    // to take all of it (the A13 module: 126 open to 17 by the step in
    // one run, never run in the next). Once the first attempt tells what
    // an attempt costs, a share of that is kept back for the step: the
    // rungs and seed rounds see the work left without it. Zero when the
    // step does not apply or there is no work deadline.
    let reserve = std::cell::Cell::new(0.0f64);
    let reserve_wanted = config.deadline_work.is_some() && narrow_step_applies(config);
    let reserve_share = std::env::var("PCB_NARROW_RESERVE")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .or(config.narrow_reserve_share)
        .unwrap_or(0.75);
    let reserve_cap = config
        .deadline_work
        .map_or(0.0, |left| (0.4 * left).min(600.0));
    let mut reserve_set = false;
    // An attempt's check comes on top of its routing (`check_charge`).
    let no_time_for_another =
        || work_left() - reserve.get() < last_attempt_work.get().max(120.0) + check_charge;
    // The reserve is a floor (2026-10-15): once kept, no rung, finer pitch
    // or seed round starts unless its projected work leaves the whole
    // reserve, and none may run into it (their work deadlines end where
    // it begins); the step negotiates at least what the first attempt did.
    // It used to be checked against the last attempt's work only, so a
    // finer pitch (four times a rung at half the pitch) or a rung run to
    // the pitch's deadline could eat it, and the step needed half of it.
    // What the ladder's first attempt negotiated (its budget, in work).
    let mut first_negotiation = 0.0f64;
    // Large multilayer boards: no fixed split of the budget suits them all
    // (one needs 600 s of negotiation in its first rung, another the last
    // rungs). Every rung negotiates briefly first; the one with the fewest
    // unfinished nets gets the rest of the budget and goes on from where
    // its probe stopped. Only the leading router is kept (memory).
    // The rung the probe continued: the regular ladder skips it.
    let mut probed_mode: Option<usize> = None;
    // The configuration of the best attempt so far (and whether its pours
    // connected), for the narrow-signal retry at the end.
    let mut best_attempt: Option<(KiCadBoardRouterConfig, bool)> = None;
    let mut probe_complete = false;
    if config.probe_ladder.unwrap_or(true)
        && modes.len() >= 3
        && config.negotiation_seconds.is_none()
    {
        let probe_seconds = config.probe_seconds.unwrap_or(75.0);
        let layer_names = LayerTable::from_pcb(&parsed)?.names;
        let mut leader: Option<(
            usize,
            usize,
            core::router::Router,
            core::Board,
            KiCadBoardRouterConfig,
        )> = None;
        // The rungs' boards, in rung order.
        let mut rungs: Vec<(usize, core::Board, KiCadBoardRouterConfig)> = Vec::new();
        for (mode, (connect, skeleton, exclusive, plane_stubs)) in modes.iter().enumerate() {
            let mut attempt = config.clone();
            attempt.plane_skeleton = Some(*skeleton);
            attempt.exclusive_planes = Some(*exclusive);
            attempt.fixed_plane_stubs = Some(*plane_stubs);
            if *exclusive && attempt.exclusive_layers.is_none() {
                attempt.exclusive_layers = Some(if layer_table.names.len() >= 6 { 2 } else { 1 });
            }
            let board = lower(&parsed, &attempt, *connect)?.board;
            let connections: usize = board
                .nets
                .iter()
                .map(|net| net.terminals.len().saturating_sub(1))
                .sum();
            if mode == 0 && (board.layer_count < 4 || connections < 120) {
                break;
            }
            rungs.push((mode, board, attempt));
        }
        // Route mode: the probes together take at most `PROBE_SHARE` of
        // the budget (in a layout, `probe_seconds` each as before).
        let probe_work = if work_bound.is_some() && !rungs.is_empty() {
            let each = bounded_probe_seconds(probe_seconds, ladder_budget, rungs.len());
            if each < probe_seconds {
                eprintln!(
                    "ladder: probes of {each:.1} s of work each ({:.0} % of the {ladder_budget:.0} s budget over {} rungs)",
                    PROBE_SHARE * 100.0,
                    rungs.len()
                );
            }
            each
        } else {
            probe_seconds
        };
        if !rungs.is_empty() && (past_deadline() || work_left() < probe_seconds * 3.0 + 180.0) {
            eprintln!("deadline too near for the ladder: no attempts");
            skipped.push(
                "the rung probes did not run: too little work left before the deadline".into(),
            );
            rungs.clear();
        }
        // The probes run side by side, each a router of its own: as many
        // at once as fit half the memory available (the first router
        // tells its size). The outcome is the one of probing one rung
        // after the other: the fewest unfinished nets lead, a tie goes to
        // the earlier rung, a rung that finishes everything ends the
        // probing, and only the probes up to it count as work. A probe's
        // budget is work, so its outcome does not depend on what runs
        // beside it.
        let mut first: Option<(core::router::Router, f64)> = None;
        // The first router's lattice maps (its memory report; what its
        // building added to the resident size where the report does not
        // say).
        let mut router_mb_estimate: Option<f64> = None;
        let mut next = 0;
        while next < rungs.len() {
            // A probe takes its seconds (more on a busy machine), the
            // continuation and the board's check more: no probe starts
            // without that much wall clock left (PolyKybd right's ladder
            // started at 1500 s and the harness killed the board at
            // 1800 s).
            if leader.is_some() && (past_deadline() || work_left() < probe_seconds * 2.0 + 120.0) {
                break;
            }
            if router_mb_estimate.is_none() {
                let before = resident_mb();
                let started = std::time::Instant::now();
                let router =
                    core::router::Router::new(&rungs[next].1, &core_config(&rungs[next].2));
                let megabytes = maps_mb_of(&router.memory_report())
                    .unwrap_or_else(|| (resident_mb() - before).max(1.0));
                // The leading router stays in memory while the next probe
                // runs: under a limit of the process's own that two do not
                // fit (decided by the limit, not by what is free), the
                // rungs run one after the other in the regular ladder.
                if let Some(limit) = memory_at_start.too_small_for(megabytes, 2) {
                    eprintln!(
                        "ladder: no rung probes: two routers of about {:.0} MB each ({megabytes:.0} MB of maps) do not fit {} of {:.0} MB",
                        limit.router_mb(megabytes),
                        limit.source,
                        limit.limit_mb
                    );
                    drop(router);
                    rungs.clear();
                    break;
                }
                first = Some((router, started.elapsed().as_secs_f64()));
                router_mb_estimate = Some(megabytes);
            }
            // The search scratches and the via reduction's copy come on
            // top of the maps a new router holds (`router_peak_mb`).
            let (fitting, why) =
                side_by_side_for(router_mb_estimate.unwrap_or(1.0), rungs.len() - next);
            let side_by_side = fitting
                // A cap for experiments (1: one after the other).
                .min(
                    std::env::var("PCB_PROBES_SIDE_BY_SIDE")
                        .ok()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(usize::MAX),
                );
            eprintln!(
                "ladder: {side_by_side} of {} probes side by side ({why})",
                rungs.len() - next
            );
            let wave = &rungs[next..next + side_by_side];
            let first_router = first.take();
            let probed: Vec<(usize, usize, core::router::Router, f64)> = std::thread::scope(
                |scope| {
                    let mut first_router = first_router;
                    let handles: Vec<_> = wave
                    .iter()
                    .enumerate()
                    .map(|(index, (mode, board, attempt))| {
                        let built = first_router.take();
                        scope.spawn(move || {
                            let started = std::time::Instant::now();
                            let (mut router, built_seconds) = built.unwrap_or_else(|| {
                                let mut quiet = core_config(attempt);
                                // One probe's lines in the log; the others'
                                // would interleave with them.
                                quiet.verbose = quiet.verbose && index == 0;
                                (core::router::Router::new(board, &quiet), 0.0)
                            });
                            let unfinished = router.probe(probe_work);
                            // Experiment hook (the ladder's judge,
                            // docs/congestion-model.md): the probe's other
                            // statistics.
                            if std::env::var_os("PCB_LADDER_STATS").is_some() {
                                let labels = router.congestion_labels();
                                let nets = labels.net_routable.len();
                                let incomplete = (0..nets).filter(|net| labels.net_routable[*net] && !labels.net_complete[*net]).count();
                                let conflicted = (0..nets).filter(|net| labels.net_routable[*net] && labels.net_conflicted[*net]).count();
                                // Nets conflicted or incomplete; the rest of
                                // `unfinished` is pour pads the pour does not
                                // reach yet.
                                let either = (0..nets)
                                    .filter(|net| labels.net_routable[*net] && (labels.net_conflicted[*net] || !labels.net_complete[*net]))
                                    .count();
                                let stranded = unfinished.saturating_sub(either);
                                eprintln!(
                                    "ladder stats: rung {mode} unfinished {unfinished} conflicted {conflicted} incomplete {incomplete} nets {either} stranded {stranded}"
                                );
                            }
                            (*mode, unfinished, router, built_seconds + started.elapsed().as_secs_f64())
                        })
                    })
                    .collect();
                    handles
                        .into_iter()
                        .map(|handle| handle.join().expect("probe thread"))
                        .collect()
                },
            );
            let mut out_of_work = false;
            for ((mode, unfinished, mut router, seconds), (_, board, attempt)) in
                probed.into_iter().zip(wave)
            {
                // As one probe after the other: a probe the work left would
                // not have started counts for nothing, however many ran side
                // by side (that number follows the memory available).
                if leader.is_some() && work_left() < probe_seconds * 2.0 + 120.0 {
                    out_of_work = true;
                    break;
                }
                let (connect, _, exclusive, plane_stubs) = modes[mode];
                ladder_work_now.set(ladder_work_now.get() + work_of(router.expansions()));
                eprintln!(
                    "probe pours={}{}: {unfinished} nets unfinished after {:.0} s{}",
                    if connect { "connect" } else { "tracks" },
                    if exclusive {
                        " (exclusive planes)"
                    } else if plane_stubs {
                        " (plane stubs)"
                    } else {
                        ""
                    },
                    seconds,
                    if side_by_side > 1 {
                        format!(" ({side_by_side} side by side)")
                    } else {
                        String::new()
                    }
                );
                if leader.as_ref().is_none_or(|(best, ..)| unfinished < *best) {
                    router.config_mut().verbose = core_config(attempt).verbose;
                    leader = Some((unfinished, mode, router, board.clone(), attempt.clone()));
                }
                // A rung that already finishes everything needs no rival
                // (the probes beside it after it count for nothing).
                if leader.as_ref().is_some_and(|(best, ..)| *best == 0) {
                    break;
                }
            }
            next += side_by_side;
            if out_of_work || leader.as_ref().is_some_and(|(best, ..)| *best == 0) {
                break;
            }
        }
        if let Some((_, mode, mut router, board, attempt)) = leader {
            let started = std::time::Instant::now();
            let remaining = bound_left()
                .unwrap_or_else(|| (ladder_budget - spent(ladder_work_now.get())).max(60.0));
            let probed = router.expansions();
            // The router's own probe is in its work already.
            router.config_mut().run_work = Some((
                ladder_budget,
                ladder_budget - spent(ladder_work_now.get()) + work_of(probed),
            ));
            // It stops where the run's work ends, not where the wall
            // clock does (the deadline stays as the guard against load).
            if config.deadline_work.is_some() {
                router.config_mut().deadline_work =
                    Some(work_of(probed) + (work_left() - check_charge).max(0.0));
            } else if let Some(left) = bound_left() {
                // Route mode: it stops where the ladder's bound ends.
                router.config_mut().deadline_work = Some(work_of(probed) + left);
            }
            let resumed = if work_bound.is_some() {
                bounded_negotiation(remaining / 3.0, remaining)
            } else {
                (remaining / 3.0).clamp(60.0, 900.0)
            };
            let routed = router.resume(resumed);
            let router_expansions = router.expansions();
            ladder_work_now.set(ladder_work_now.get() + work_of(router_expansions - probed));
            drop(router);
            if output_directory.exists() {
                fs::remove_dir_all(output_directory).map_err(|error| error.to_string())?;
            }
            let (connect, skeleton, exclusive, plane_stubs) = modes[mode];
            let mut result = write_attempt(
                parsed.clone(),
                &board,
                routed,
                &layer_names,
                source_directory,
                board_id,
                output_directory,
                &unchecked(&attempt),
                [0.0, started.elapsed().as_secs_f64()],
            )?;
            result.pours = if !has_pours {
                "none"
            } else if connect && skeleton {
                "connect+skeleton"
            } else if connect {
                "connect"
            } else {
                "tracks"
            }
            .into();
            result.maps_mb = router_mb_estimate.unwrap_or(0.0);
            router_maps = Some((result.maps_mb, result.grid_pitch_mm));
            let judged = judge(&mut result, output_directory, None)?;
            let opens = judged.unwrap_or((router_open(&result), 0));
            eprintln!(
                "attempt pours={}{} pitch=regular (probed, continued): {} open{}, {} starved, {} vias, {:.1} s",
                result.pours,
                if exclusive {
                    " (exclusive planes)"
                } else if plane_stubs {
                    " (plane stubs)"
                } else {
                    ""
                },
                opens.0,
                by_router(&judged),
                opens.1,
                result.vias,
                ladder_started.elapsed().as_secs_f64()
            );
            slowest = Some((work_of(result.expansions), result.grid_pitch_mm));
            last_attempt_work.set(work_of(router_expansions));
            last_attempt_pitch.set(result.grid_pitch_mm);
            if reserve_wanted && !reserve_set && opens.0 > 0 {
                reserve_set = true;
                first_negotiation = probe_seconds + resumed;
                reserve.set(reserve_with_check(
                    narrow_step_reserve(work_of(router_expansions), reserve_share, reserve_cap),
                    check_charge,
                ));
                eprintln!(
                    "ladder: {:.0} s of work kept for the narrow-signal step ({:.0} s left, the attempt took {:.0} s)",
                    reserve.get(),
                    work_left(),
                    work_of(router_expansions)
                );
            }
            if route_reserve_wanted && !route_reserve_set && opens.0 > 0 {
                route_reserve_set = true;
                keep_for_route_step(work_of(router_expansions));
            }
            probe_complete = opens.0 == 0;
            best = Some((opens, result));
            best_attempt = Some((attempt.clone(), connect));
            probed_mode = Some(mode);
        }
    }
    // Whether the best attempt's rung had its seed retry.
    let mut best_seeded = false;
    'ladder: for pitch in &pitches {
        if probe_complete {
            continue;
        }
        if let (Some(pitch), Some((seconds, previous))) = (pitch, slowest) {
            let projected = seconds * (previous / pitch[0]).powi(2);
            if projected > budget {
                eprintln!(
                    "skipping pitch {:?}: projected {projected:.0} s exceeds the {budget:.0} s refinement budget",
                    pitch
                );
                skipped.push(format!(
                    "the finer lattice ({} mm) was not tried: projected {projected:.0} s of routing exceeds refine_budget_seconds ({budget:.0} s)",
                    pitch[0]
                ));
                break;
            }
        }
        // Route mode: a finer lattice whose attempt would take more than is
        // left under the ladder's bound is not started.
        if let (Some(pitch), Some((seconds, previous)), Some(left)) = (pitch, slowest, rung_left())
        {
            let projected = seconds * (previous / pitch[0]).powi(2);
            if projected > left {
                eprintln!(
                    "skipping pitch {:?}: projected {projected:.0} s exceeds the {left:.0} s left of the ladder's bound",
                    pitch
                );
                skipped.push(format!(
                    "the finer lattice ({} mm) was not tried: projected {projected:.0} s of routing exceeds the {left:.0} s left of the ladder's work bound (ladder_budget_seconds)",
                    pitch[0]
                ));
                break;
            }
        }
        // A limit of the process's own (its address space, its cgroup, an
        // explicit one) one router of the finer lattice does not fit.
        if let (Some(pitch), Some((maps, previous))) = (pitch, router_maps) {
            let projected = maps * (previous / pitch[0]).powi(2);
            if let Some(limit) = memory_at_start.too_small_for(projected, 1) {
                eprintln!(
                    "skipping pitch {:?}: a router of about {:.0} MB ({projected:.0} MB of maps) does not fit {} of {:.0} MB",
                    pitch,
                    limit.router_mb(projected),
                    limit.source,
                    limit.limit_mb
                );
                skipped.push(format!(
                    "the finer lattice ({} mm) was not tried: a router of about {:.0} MB does not fit {} of {:.0} MB",
                    pitch[0],
                    limit.router_mb(projected),
                    limit.source,
                    limit.limit_mb
                ));
                break;
            }
        }
        if let (Some(pitch), Some((megabytes, previous))) = (pitch, router_mb) {
            let projected = megabytes * (previous / pitch[0]).powi(2);
            // Against the machine's memory, not what is available now: the
            // attempts may not depend on what else runs.
            if memory_total_mb().is_some_and(|total| projected > total * 0.4) {
                eprintln!(
                    "skipping pitch {:?}: a router of about {:.1} GB does not fit the machine's memory",
                    pitch,
                    projected / 1024.0
                );
                skipped.push(format!(
                    "the finer lattice ({} mm) was not tried: a router of about {:.1} GB does not fit the machine's memory",
                    pitch[0],
                    projected / 1024.0
                ));
                break;
            }
        }
        // Every rung of this pitch gets the same budget, decided before
        // any of them runs (an equal share of what is left of the ladder's
        // budget; an attempt spends up to about three times its
        // negotiation in repair and polish, so negotiation gets a third of
        // the share: zpn_devboard's two rungs took 1300 s and the two that
        // complete such boards never ran). A rung's outcome then depends
        // on nothing that runs before or beside it, and the rungs after
        // the first run side by side, as many as fit the memory; they are
        // judged in rung order as one after another, a rung the order
        // would not have reached counting for nothing.
        let pitch_negotiation = config.negotiation_seconds.is_none().then(|| {
            if let Some(left) = rung_left() {
                bounded_negotiation(left / modes.len() as f64 / 3.0, left)
            } else {
                let remaining = (ladder_budget - spent(ladder_work_now.get())).max(0.0);
                (remaining / modes.len() as f64 / 3.0).clamp(60.0, 900.0)
            }
        });
        // Route mode: each rung's work deadline (`rung_share`): the
        // ladder's first attempt has all that is left under the bound, the
        // rungs of a pitch after it an even share of what is left when the
        // first of them starts, decided before they run.
        let route_deadline: std::cell::Cell<Option<f64>> = std::cell::Cell::new(None);
        let mut share_fixed = false;
        let pitch_left = (ladder_budget - spent(ladder_work_now.get())).max(0.0);
        let pitch_work_left = work_left().max(0.0);
        let rung_attempt = |mode: usize| -> KiCadBoardRouterConfig {
            let (_, skeleton, exclusive, plane_stubs) = modes[mode];
            let mut attempt = config.clone();
            attempt.plane_skeleton = Some(skeleton);
            attempt.exclusive_planes = Some(exclusive);
            attempt.fixed_plane_stubs = Some(plane_stubs);
            if exclusive && attempt.exclusive_layers.is_none() {
                attempt.exclusive_layers = Some(if layer_table.names.len() >= 6 { 2 } else { 1 });
            }
            if let Some(seconds) = pitch_negotiation {
                attempt.negotiation_seconds = Some(seconds);
            }
            attempt.run_work = Some((ladder_budget, pitch_left));
            // Every rung of the pitch stops where the run's work ends as
            // the pitch starts (decided before they run, as the budget),
            // less the work kept for the narrow-signal step.
            // Its check follows: the router stops where that still fits.
            attempt.router_deadline_work = config
                .deadline_work
                .map(|_| {
                    if reserve.get() > 0.0 {
                        deadline_before_reserve(pitch_work_left - check_charge, reserve.get(), 1)
                    } else {
                        (pitch_work_left - check_charge).max(0.0)
                    }
                })
                .or(route_deadline.get());
            if let Some(pitch) = pitch {
                attempt.grid_pitches_mm = Some(pitch.clone());
            }
            attempt
        };
        let rung_directory = |mode: usize| output_directory.with_extension(format!("rung{mode}"));
        // A probed rung continued with what it had is not run again; the
        // other rungs still get their turn when it left connections open.
        let candidates: Vec<usize> = (0..modes.len())
            .filter(|mode| !(pitch.is_none() && probed_mode == Some(*mode)))
            .collect();
        let mut ran: Vec<Option<Result<KiCadBoardRouterResult, String>>> =
            (0..modes.len()).map(|_| None).collect();
        for (position, &mode) in candidates.iter().enumerate() {
            let (connect, skeleton, exclusive, plane_stubs) = &modes[mode];
            // With work kept for the narrow-signal step, the rung's work
            // as projected from the last attempt (on a finer lattice, by
            // the square of the pitch ratio: DSP-ADAU1452's 0.05 mm rung
            // after its 0.075 mm rungs) must leave all of it.
            let short_of_work = if work_bound.is_some() {
                !bound_allows()
            } else if reserve.get() > 0.0 {
                let cost = pitch.as_ref().map_or(last_attempt_work.get(), |pitch| {
                    projected_attempt_work(
                        last_attempt_work.get(),
                        last_attempt_pitch.get(),
                        pitch[0],
                    )
                });
                !fits_before_reserve(work_left() - check_charge, reserve.get(), cost)
            } else {
                no_time_for_another()
            };
            if best.is_some() && (past_deadline() || short_of_work) {
                if work_bound.is_some() && short_of_work {
                    eprintln!(
                        "ladder: its work bound is reached ({:.0} s left of {:.0} s, {:.0} s of it kept for the narrow-signal step): no further attempts",
                        bound_left().unwrap_or(0.0),
                        work_bound.unwrap_or(0.0),
                        route_reserve.get()
                    );
                    skipped.push(format!(
                        "further rungs of the attempt ladder did not run: its work bound ({:.0} s, ladder_budget_seconds and {:.0} %) was reached",
                        work_bound.unwrap_or(0.0),
                        LADDER_WORK_TOLERANCE * 100.0
                    ));
                } else {
                    eprintln!("deadline reached: no further attempts");
                    skipped.push("further rungs of the attempt ladder did not run: the run's end was near (its deadline, or too little work left for another attempt)".into());
                }
                break 'ladder;
            }
            // The skeleton and the plane stubs only help when the plain
            // pour connection left pads of a pour net open; elsewhere they
            // just take room.
            if ((*skeleton && !config.plane_skeleton.unwrap_or(false))
                || (*plane_stubs && !config.fixed_plane_stubs.unwrap_or(false)))
                && best.as_ref().is_some_and(|(_, result)| {
                    result.pours == "connect"
                        && !result.nets.iter().any(|net| {
                            net.unconnected_terminals > 0 && pour_nets.contains(&net.connection)
                        })
                })
            {
                if rung_directory(mode).exists() {
                    fs::remove_dir_all(rung_directory(mode)).map_err(|error| error.to_string())?;
                }
                continue;
            }
            if work_bound.is_some() {
                if best.is_none() {
                    route_deadline.set(bound_left());
                } else if !share_fixed {
                    share_fixed = true;
                    route_deadline
                        .set(rung_left().map(|left| rung_share(left, candidates.len() - position)));
                }
            }
            let attempt = rung_attempt(mode);
            if ran[mode].is_none() {
                // The first rung of a pitch without a board yet runs alone
                // (it often completes the board); after it, the rest run
                // side by side, each a router of this pitch and a KiCad
                // check.
                let side_by_side = if best.is_none() {
                    1
                } else {
                    let wanted = candidates.len() - position;
                    match router_maps {
                        Some((maps, previous)) => {
                            let each = maps
                                * (previous / pitch.as_ref().map_or(previous, |pitch| pitch[0]))
                                    .powi(2);
                            let (count, why) = side_by_side_for(each, wanted);
                            eprintln!("ladder: {count} of {wanted} rungs side by side ({why})");
                            count
                        }
                        // No maps known (an engine without a memory
                        // report): one after the other.
                        None => 1,
                    }
                };
                let wave: Vec<usize> = candidates[position..position + side_by_side].to_vec();
                let routed: Vec<(usize, Result<KiCadBoardRouterResult, String>)> =
                    std::thread::scope(|scope| {
                        let handles: Vec<_> = wave
                            .iter()
                            .map(|&other| {
                                let attempt = unchecked(&rung_attempt(other));
                                let directory = rung_directory(other);
                                let connect = modes[other].0;
                                scope.spawn(move || {
                                    if directory.exists() {
                                        fs::remove_dir_all(&directory)
                                            .map_err(|error| error.to_string())?;
                                    }
                                    route_kicad_board_once(
                                        source_directory,
                                        board_id,
                                        &directory,
                                        &attempt,
                                        connect,
                                    )
                                })
                            })
                            .collect();
                        wave.iter()
                            .copied()
                            .zip(
                                handles
                                    .into_iter()
                                    .map(|handle| handle.join().expect("rung thread")),
                            )
                            .collect()
                    });
                for (other, result) in routed {
                    ran[other] = Some(result);
                }
            }
            let directory = rung_directory(mode);
            let mut result = ran[mode].take().expect("the rung ran")?;
            result.pours = if !has_pours {
                "none"
            } else if *connect && *skeleton {
                "connect+skeleton"
            } else if *connect {
                "connect"
            } else {
                "tracks"
            }
            .into();
            let mut judged = judge(&mut result, &directory, quality_of(&best))?;
            let mut opens = judged.unwrap_or((router_open(&result), 0));
            if router_mb.is_none() {
                router_mb = memory_mb().map(|(peak, _)| (peak, result.grid_pitch_mm));
            }
            if router_maps.is_none() && result.maps_mb > 0.0 {
                router_maps = Some((result.maps_mb, result.grid_pitch_mm));
            }
            // Seeds run side by side, each a router of this pitch, as many
            // at once as fit the memory (`MemoryLimits`); how many seeds
            // route does not depend on it.
            let seeds_that_fit = |wanted: usize| -> usize {
                let Some((maps, previous)) = router_maps else {
                    return 1;
                };
                let each = maps * (previous / result.grid_pitch_mm).powi(2);
                let (count, why) = side_by_side_for(each, wanted);
                if count < wanted {
                    eprintln!("ladder: {count} of {wanted} seeds side by side ({why})");
                }
                count
            };
            // What one attempt at this pitch costs, for projecting finer
            // pitches; a seed retry does not change it.
            // In work, as the ladder's budget.
            let attempt_seconds = work_of(result.expansions);
            ladder_work_now.set(ladder_work_now.get() + attempt_seconds);
            // Open connections often depend on the order: route the same
            // attempt a few perturbed ways and keep the better board.
            let retry = config.retry_seeds.unwrap_or(4);
            // Four seeds in parallel cost about twice the attempt; on a
            // board whose attempt takes minutes that time is better spent
            // on the next rung (MIDAS-MK2: two retries, 740 s, both worse).
            // The seeds wait while other rungs remain: another way of
            // connecting the pours often finishes the board (olimex-c3,
            // stickhub and interf-u spent 40-130 s on seeds of rungs a
            // later one beat); the best rung gets them at the end.
            let mut seeds_tried = false;
            if opens.0 > 0
                && retry > 0
                && attempt.seeds.unwrap_or(1) <= 1
                && attempt.first_seed.unwrap_or(0) == 0
                && attempt_seconds <= budget / 2.0
                && mode + 1 == modes.len()
                && !past_deadline()
                // The seeds count as `retry` attempts of work: not out of
                // the work kept for the narrow-signal step.
                && (reserve.get() == 0.0 || work_left() - check_charge - reserve.get() >= retry as f64 * attempt_seconds)
                // Route mode: nor beyond the ladder's bound.
                && rung_left().is_none_or(|left| left >= retry as f64 * attempt_seconds)
            {
                seeds_tried = true;
                let mut seeded = attempt.clone();
                seeded.seeds = Some(retry);
                seeded.seeds_side_by_side = Some(seeds_that_fit(retry));
                seeded.first_seed = Some(1);
                // Their work counts `retry` times: each stops at its share
                // of what is left beyond the reserve.
                if reserve.get() > 0.0 {
                    seeded.router_deadline_work = Some(deadline_before_reserve(
                        work_left() - check_charge,
                        reserve.get(),
                        retry,
                    ));
                } else if let Some(left) = rung_left() {
                    seeded.router_deadline_work = Some(rung_share(left, retry));
                }
                let retry_directory = output_directory.with_extension("seeds");
                if retry_directory.exists() {
                    fs::remove_dir_all(&retry_directory).map_err(|error| error.to_string())?;
                }
                let mut other = route_kicad_board_once(
                    source_directory,
                    board_id,
                    &retry_directory,
                    &unchecked(&seeded),
                    *connect,
                )?;
                other.pours = result.pours.clone();
                // The seeds run side by side: about `retry` times one's work.
                ladder_work_now
                    .set(ladder_work_now.get() + work_of(other.expansions) * retry as f64);
                let other_judged = judge(&mut other, &retry_directory, quality_of(&best))?;
                let other_opens = other_judged.unwrap_or((router_open(&other), 0));
                eprintln!(
                    "attempt again with {retry} seeds: {} open{}, {} vias, {:.1} s",
                    other_opens.0,
                    by_router(&other_judged),
                    other.vias,
                    other.routing_seconds
                );
                // Boards KiCad checked compare as before; one it did not
                // check cannot beat the best so far, so it never displaces
                // one it did (and which of two unchecked ones stays does
                // not matter: neither beats the best).
                let other_better = match (judged.is_some(), other_judged.is_some()) {
                    (true, true) => (other_opens, other.vias, other.length_mm)
                        .partial_cmp(&(opens, result.vias, result.length_mm))
                        .is_some_and(|order| order.is_lt()),
                    (false, true) => true,
                    (_, false) => false,
                };
                if other_better {
                    fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
                    fs::rename(&retry_directory, &directory).map_err(|error| error.to_string())?;
                    other.routing_seconds += result.routing_seconds;
                    result = other;
                    opens = other_opens;
                    judged = other_judged;
                } else {
                    fs::remove_dir_all(&retry_directory).map_err(|error| error.to_string())?;
                }
            }
            eprintln!(
                "attempt pours={}{} pitch={}: {} open{}, {} starved, {} vias, {:.1} s",
                result.pours,
                if *exclusive {
                    " (exclusive planes)"
                } else if *plane_stubs {
                    " (plane stubs)"
                } else {
                    ""
                },
                pitch
                    .as_ref()
                    .map_or("regular".to_string(), |pitch| format!("{:?}", pitch)),
                opens.0,
                by_router(&judged),
                opens.1,
                result.vias,
                result.routing_seconds
            );
            last_attempt_work.set(attempt_seconds);
            last_attempt_pitch.set(result.grid_pitch_mm);
            if reserve_wanted && !reserve_set && opens.0 > 0 {
                reserve_set = true;
                first_negotiation = attempt.negotiation_seconds.unwrap_or(900.0);
                reserve.set(reserve_with_check(
                    narrow_step_reserve(attempt_seconds, reserve_share, reserve_cap),
                    check_charge,
                ));
                eprintln!(
                    "ladder: {:.0} s of work kept for the narrow-signal step ({:.0} s left, the attempt took {attempt_seconds:.0} s)",
                    reserve.get(),
                    work_left()
                );
            }
            if route_reserve_wanted && !route_reserve_set && opens.0 > 0 {
                route_reserve_set = true;
                keep_for_route_step(attempt_seconds);
            }
            let seconds = attempt_seconds;
            let used = (attempt_seconds, result.grid_pitch_mm);
            if slowest.is_none_or(|(seconds, _)| used.0 > seconds) {
                slowest = Some(used);
            }
            // Equally open boards: fewer vias, then less copper. A board
            // KiCad did not check cannot beat the best (`CheckGate`).
            let better = best.as_ref().is_none_or(|(best_opens, best_result)| {
                judged.is_some()
                    && (opens, result.vias, result.length_mm)
                        .partial_cmp(&(*best_opens, best_result.vias, best_result.length_mm))
                        .is_some_and(|order| order.is_lt())
            });
            if better {
                if output_directory.exists() {
                    fs::remove_dir_all(output_directory).map_err(|error| error.to_string())?;
                }
                fs::rename(&directory, output_directory).map_err(|error| error.to_string())?;
                best = Some((opens, result));
                best_attempt = Some((attempt.clone(), *connect));
                best_seeded = seeds_tried || attempt.seeds.unwrap_or(1) > 1;
            } else if directory.exists() {
                fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
            }
            // A complete board needs no narrow-signal step: what was kept
            // for it is the next rung's again (a clean skeleton rung).
            if best.as_ref().is_some_and(|(opens, _)| opens.0 == 0) && reserve.get() > 0.0 {
                eprintln!(
                    "ladder: the board is complete: the {:.0} s kept for the narrow-signal step are released",
                    reserve.get()
                );
                reserve.set(0.0);
            }
            if best.as_ref().is_some_and(|(opens, _)| opens.0 == 0) && route_reserve.get() > 0.0 {
                eprintln!(
                    "ladder: the board is complete: the {:.0} s kept for the narrow-signal step are released",
                    route_reserve.get()
                );
                route_reserve.set(0.0);
            }
            // Another way of connecting the pours is always worth trying
            // while connections are open (a complete board matters more
            // than the time; the finer pitches below are what the budget
            // limits). A clean board from the plane skeleton still gets the
            // next rung when time allows: the fixed tree often costs many
            // vias that routing the pour nets as tracks does not.
            let skeleton_clean = opens.0 == 0 && *connect && *skeleton && !extra_rung_tried;
            if skeleton_clean && mode + 1 < modes.len() && seconds <= budget / 2.0 {
                extra_rung_tried = true;
                continue;
            }
            if opens.0 == 0 || extra_rung_tried {
                break 'ladder;
            }
            if spent(ladder_work_now.get()) > ladder_budget {
                eprintln!("ladder budget of {ladder_budget:.0} s used: no further attempts");
                skipped.push(format!(
                    "further rungs of the attempt ladder did not run: its budget of {ladder_budget:.0} s of work was used (ladder_budget_seconds)"
                ));
                break 'ladder;
            }
        }
    }
    // Rungs that ran beside one the order stopped at.
    for mode in 0..modes.len() {
        let directory = output_directory.with_extension(format!("rung{mode}"));
        if directory.exists() {
            fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
        }
    }
    // The best rung's deferred seed retries: rounds of fresh seeds while
    // connections stay open and the ladder budget pays for them, keeping
    // one attempt's worth for the narrow-signal step after (an attempt
    // over half the refine budget used to get none: eurorack-pmod ended
    // 4 open with 700 s of its budget left; three seeds took it to 2).
    let mut next_seed = 1u64;
    for _round in 0..3 {
        let Some((opens, best_result)) = best.as_ref() else {
            break;
        };
        let Some((attempt, connect)) = best_attempt.as_ref() else {
            break;
        };
        let cost = work_of(best_result.expansions).max(1.0);
        // A round's seeds and its one check.
        let mut affordable = ((ladder_budget - spent(ladder_work_now.get()) - cost - check_charge)
            / cost)
            .floor()
            .max(0.0) as usize;
        if reserve.get() > 0.0 {
            affordable = affordable.min(
                ((work_left() - check_charge - reserve.get()) / cost)
                    .floor()
                    .max(0.0) as usize,
            );
        }
        if let Some(left) = rung_left() {
            affordable = affordable.min((left / cost).floor().max(0.0) as usize);
        }
        let retry = config.retry_seeds.unwrap_or(4).min(affordable);
        if opens.0 == 0
            || best_seeded
            || retry == 0
            || past_deadline()
            || no_time_for_another()
            || !bound_allows()
        {
            break;
        }
        // As many at once as fit the memory (`MemoryLimits`); how many
        // seeds route does not depend on it.
        let side_by_side = router_maps.map_or(1, |(maps, previous)| {
            let each = maps * (previous / best_result.grid_pitch_mm).powi(2);
            let (count, why) = side_by_side_for(each, retry);
            if count < retry {
                eprintln!("ladder: {count} of {retry} seeds side by side ({why})");
            }
            count
        });
        let mut seeded = attempt.clone();
        seeded.seeds = Some(retry);
        seeded.seeds_side_by_side = Some(side_by_side);
        seeded.first_seed = Some(next_seed);
        if reserve.get() > 0.0 {
            seeded.router_deadline_work = Some(deadline_before_reserve(
                work_left() - check_charge,
                reserve.get(),
                retry,
            ));
        } else if let Some(left) = rung_left() {
            seeded.router_deadline_work = Some(rung_share(left, retry));
        }
        next_seed += retry as u64;
        if scratch.exists() {
            fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
        }
        let connect = *connect;
        let quality = quality_of(&best);
        let mut other = route_kicad_board_once(
            source_directory,
            board_id,
            &scratch,
            &unchecked(&seeded),
            connect,
        )?;
        other.pours = best_result.pours.clone();
        ladder_work_now.set(ladder_work_now.get() + work_of(other.expansions) * retry as f64);
        let other_judged = judge(&mut other, &scratch, quality)?;
        let other_opens = other_judged.unwrap_or((router_open(&other), 0));
        eprintln!(
            "best rung again with {retry} seeds: {} open{}, {} starved, {} vias, {:.1} s",
            other_opens.0,
            by_router(&other_judged),
            other_opens.1,
            other.vias,
            other.routing_seconds
        );
        let better = other_judged.is_some()
            && (other_opens, other.vias, other.length_mm)
                .partial_cmp(&(*opens, best_result.vias, best_result.length_mm))
                .is_some_and(|order| order.is_lt());
        if better {
            fs::remove_dir_all(output_directory).map_err(|error| error.to_string())?;
            fs::rename(&scratch, output_directory).map_err(|error| error.to_string())?;
            best = Some((other_opens, other));
        } else {
            fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
        }
    }
    let mut narrow_step_relaxed = false;
    // The work kept for the step is its own now. With it, the step needs
    // half of it still there (the last rung may have run over its share),
    // and at least 180 s (three times the negotiation's floor: jetson's
    // ladder kept 129 s with 70 left, and a step of 60 s negotiation took
    // 232 s and left 268 open), and negotiates in a third of what is
    // left, so that it ends in the work deadline whatever the machine's
    // load; without it, the step needs an attempt's work as before (route
    // mode: no work deadline).
    let kept = reserve.replace(0.0);
    // Route mode: what was kept for the step is its own now.
    let route_kept = route_reserve.replace(0.0);
    // The step's own check is in what was kept: its routing has the rest
    // (and of what is left, what its check leaves).
    let kept_routing = (kept - check_charge).max(0.0);
    let routing_left = || work_left() - check_charge;
    let step_has_time = || {
        if kept > 0.0 {
            routing_left() >= narrow_step_minimum(kept_routing)
        } else if work_bound.is_some() {
            bound_left().is_some_and(|left| route_step_has_time(left, route_kept, ladder_budget))
        } else {
            !no_time_for_another()
        }
    };
    let step_budget = |step: &mut KiCadBoardRouterConfig| {
        if kept > 0.0 {
            step.negotiation_seconds = Some(narrow_step_negotiation_floored(
                routing_left(),
                step.negotiation_seconds,
                first_negotiation,
                kept_routing,
            ));
            step.run_work = Some((ladder_budget, routing_left().max(0.0)));
            // The attempt it varies may carry its pitch's work deadline,
            // taken when that pitch started: the step's is what is left now.
            step.router_deadline_work = Some(routing_left().max(0.0));
        } else if let Some(left) = bound_left() {
            // Route mode: within what is left of the ladder's bound, its
            // negotiation at most half of that.
            step.negotiation_seconds =
                Some(step.negotiation_seconds.unwrap_or(900.0).min(0.5 * left));
            step.router_deadline_work = Some(left);
        }
    };
    // Why a step for a board with connections open does not run, in the
    // order its conditions are checked.
    let blocked = || -> Option<&'static str> {
        if spent(ladder_work_now.get()) >= 1.5 * ladder_budget {
            Some("the ladder's budget was used (ladder_budget_seconds)")
        } else if past_deadline() {
            Some("the deadline was reached")
        } else if !step_has_time() {
            Some(if kept > 0.0 {
                "too little work was left of what was kept for it (half of that, at least 180 s)"
            } else if work_bound.is_some() {
                "too little work was left under the ladder's work bound (ladder_budget_seconds and its tolerance)"
            } else {
                "too little work was left for another attempt"
            })
        } else {
            None
        }
    };
    if let (Some((opens, _)), Some(_), Some(narrow)) = (
        best.as_ref(),
        best_attempt.as_ref(),
        config.narrow_signal_mm,
    ) && opens.0 > 0
        && config.use_narrow_signals.is_none()
        && config
            .connection_rules
            .values()
            .chain(config.default_rules.iter())
            .any(|rules| rules.trace_width_mm > narrow + 1.0e-9)
        && let Some(why) = blocked()
    {
        eprintln!(
            "the narrow-signal step did not run: {why} ({:.1} s of work left)",
            bound_left().unwrap_or_else(work_left)
        );
        skipped.push(format!(
            "the narrow-signal step (signals at {narrow} mm) did not run: {why}"
        ));
    }
    // Connections still open at the class widths: the best attempt once
    // more with signal nets at the project's smallest predefined width
    // (designers often keep the default class at a power width and draw
    // signals narrower).
    if let (Some((opens, _)), Some((attempt, connect)), Some(narrow)) = (
        best.as_ref(),
        best_attempt.as_ref(),
        config.narrow_signal_mm,
    ) && opens.0 > 0
        && config.use_narrow_signals.is_none()
        && spent(ladder_work_now.get()) < 1.5 * ladder_budget
        && !past_deadline()
        && step_has_time()
        && config
            .connection_rules
            .values()
            .chain(config.default_rules.iter())
            .any(|rules| rules.trace_width_mm > narrow + 1.0e-9)
    {
        let mut narrowed = attempt.clone();
        narrowed.use_narrow_signals = Some(true);
        step_budget(&mut narrowed);
        // A project that takes clearance findings as warnings gets its
        // minimum clearance in the same attempt (the A13 module's designer
        // drew 0.1016 tracks 0.149 from balls in classes of 0.2).
        if let Some(relaxed) = config.relaxed_clearance_mm {
            narrowed.relaxed_clearance_mm = None;
            for rules in narrowed
                .connection_rules
                .values_mut()
                .chain(narrowed.default_rules.iter_mut())
            {
                rules.clearance_mm = rules.clearance_mm.min(relaxed);
            }
            narrow_step_relaxed = true;
        }
        if scratch.exists() {
            fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
        }
        let mut result = route_kicad_board_once(
            source_directory,
            board_id,
            &scratch,
            &unchecked(&narrowed),
            *connect,
        )?;
        ladder_work_now.set(ladder_work_now.get() + work_of(result.expansions));
        result.pours = best
            .as_ref()
            .map(|(_, best)| best.pours.clone())
            .unwrap_or_default();
        let narrow_judged = judge(&mut result, &scratch, quality_of(&best))?;
        let narrow_opens = narrow_judged.unwrap_or((router_open(&result), 0));
        eprintln!(
            "attempt with signals at {narrow} mm{}: {} open{}, {} starved, {} vias, {:.1} s, work {:.0} s",
            config
                .relaxed_clearance_mm
                .map(|relaxed| format!(" and clearance {relaxed} mm"))
                .unwrap_or_default(),
            narrow_opens.0,
            by_router(&narrow_judged),
            narrow_opens.1,
            result.vias,
            result.routing_seconds,
            work_of(result.expansions)
        );
        let better = narrow_judged.is_some()
            && best.as_ref().is_some_and(|(best_opens, best_result)| {
                (narrow_opens, result.vias, result.length_mm)
                    .partial_cmp(&(*best_opens, best_result.vias, best_result.length_mm))
                    .is_some_and(|order| order.is_lt())
            });
        if better {
            fs::remove_dir_all(output_directory).map_err(|error| error.to_string())?;
            fs::rename(&scratch, output_directory).map_err(|error| error.to_string())?;
            best = Some((narrow_opens, result));
            best_attempt = Some((narrowed, *connect));
        } else {
            fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
        }
    }
    // Still open, and the project takes clearance findings as warnings:
    // the best attempt once more at the board's minimum clearance, which is
    // what the designer held to.
    if let (Some((opens, _)), Some(_), Some(relaxed)) = (
        best.as_ref(),
        best_attempt.as_ref(),
        config.relaxed_clearance_mm,
    ) && opens.0 > 0
        && !narrow_step_relaxed
        && config
            .connection_rules
            .values()
            .chain(config.default_rules.iter())
            .any(|rules| rules.clearance_mm > relaxed + 1.0e-9)
        && let Some(why) = blocked()
    {
        skipped.push(format!(
            "the minimum-clearance step (clearance {relaxed} mm) did not run: {why}"
        ));
    }
    if let (Some((opens, _)), Some((attempt, connect)), Some(relaxed)) = (
        best.as_ref(),
        best_attempt.as_ref(),
        config.relaxed_clearance_mm,
    ) && opens.0 > 0
        && !narrow_step_relaxed
        && spent(ladder_work_now.get()) < 1.5 * ladder_budget
        && !past_deadline()
        && step_has_time()
        && config
            .connection_rules
            .values()
            .chain(config.default_rules.iter())
            .any(|rules| rules.clearance_mm > relaxed + 1.0e-9)
    {
        let mut relaxed_attempt = attempt.clone();
        step_budget(&mut relaxed_attempt);
        relaxed_attempt.relaxed_clearance_mm = None;
        for rules in relaxed_attempt
            .connection_rules
            .values_mut()
            .chain(relaxed_attempt.default_rules.iter_mut())
        {
            rules.clearance_mm = rules.clearance_mm.min(relaxed);
        }
        if scratch.exists() {
            fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
        }
        let mut result = route_kicad_board_once(
            source_directory,
            board_id,
            &scratch,
            &unchecked(&relaxed_attempt),
            *connect,
        )?;
        ladder_work_now.set(ladder_work_now.get() + work_of(result.expansions));
        result.pours = best
            .as_ref()
            .map(|(_, best)| best.pours.clone())
            .unwrap_or_default();
        let relaxed_judged = judge(&mut result, &scratch, quality_of(&best))?;
        let relaxed_opens = relaxed_judged.unwrap_or((router_open(&result), 0));
        eprintln!(
            "attempt at the minimum clearance {relaxed} mm: {} open{}, {} starved, {} vias, {:.1} s",
            relaxed_opens.0,
            by_router(&relaxed_judged),
            relaxed_opens.1,
            result.vias,
            result.routing_seconds
        );
        let better = relaxed_judged.is_some()
            && best.as_ref().is_some_and(|(best_opens, best_result)| {
                (relaxed_opens, result.vias, result.length_mm)
                    .partial_cmp(&(*best_opens, best_result.vias, best_result.length_mm))
                    .is_some_and(|order| order.is_lt())
            });
        if better {
            fs::remove_dir_all(output_directory).map_err(|error| error.to_string())?;
            fs::rename(&scratch, output_directory).map_err(|error| error.to_string())?;
            best = Some((relaxed_opens, result));
        } else {
            fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
        }
    }
    let (final_open, mut result) = best.expect("at least one routing attempt");
    result.pin_swaps = pin_swaps;
    if prepared {
        fs::remove_dir_all(&poured_directory).map_err(|error| error.to_string())?;
    }
    if config.pin_swaps.is_some() {
        fs::remove_dir_all(&swapped_directory).map_err(|error| error.to_string())?;
    }
    if config.solid_starved_thermals == Some(true) && !config.skip_native_verification {
        let starved: Vec<(String, String)> = result
            .diagnostics
            .starved_thermals
            .iter()
            .filter(|starved| starved.spokes.is_some())
            .filter_map(|starved| {
                let text = &starved.pad;
                let number = text.split("ad ").nth(1)?.split(" [").next()?.to_string();
                let reference = text.split(" of ").nth(1)?.split(" on ").next()?.to_string();
                Some((reference, number))
            })
            .collect();
        if !starved.is_empty() {
            let board_path = output_directory.join(format!("{board_id}.kicad_pcb"));
            let text = fs::read_to_string(&board_path)
                .map_err(|error| format!("failed to read {}: {error}", board_path.display()))?;
            let mut pcb = parse(&text)?;
            let mut changed = 0;
            if let Expr::List(items) = &mut pcb {
                for footprint in items
                    .iter_mut()
                    .filter(|item| item.head() == Some("footprint"))
                {
                    let reference = footprint_reference(footprint)
                        .unwrap_or_default()
                        .to_string();
                    let Expr::List(children) = footprint else {
                        continue;
                    };
                    for pad in children
                        .iter_mut()
                        .filter(|child| child.head() == Some("pad"))
                    {
                        let number = pad
                            .children()
                            .get(1)
                            .and_then(Expr::atom)
                            .unwrap_or("")
                            .to_string();
                        if !starved.iter().any(|(r, n)| *r == reference && *n == number) {
                            continue;
                        }
                        if let Expr::List(fields) = pad {
                            fields.retain(|field| field.head() != Some("zone_connect"));
                            fields.push(Expr::List(vec![
                                Expr::Atom("zone_connect".into()),
                                Expr::Atom("2".into()),
                            ]));
                            changed += 1;
                        }
                    }
                }
            }
            if changed > 0 {
                fs::write(&board_path, format!("{}\n", encode(&pcb))).map_err(|error| {
                    format!("failed to write {}: {error}", board_path.display())
                })?;
                result.native = Some(verify_materialized_rung(output_directory, board_id)?);
                charge_checks(1);
                result.diagnostics.starved_thermals = starved_thermal_diagnostics(output_directory);
                eprintln!(
                    "{changed} starved pads connected solid: {} starved thermals left",
                    result.diagnostics.starved_thermals.len()
                );
            }
        }
    }
    // The neck repair improves a checked board; past the deadline it is
    // skipped (up to six more KiCad checks, uncounted as work).
    if result.native.is_some() && !past_deadline() {
        let checks = repair_necks(&mut result, output_directory, board_id, config)?;
        charge_checks(checks);
    }
    result.cut = core::router::wall_cuts();
    result.work_seconds = ladder_work_now.get();
    result.work_bound_seconds = work_bound;
    result.memory_limits = memory_at_start.describe();
    if let Some(bound) = work_bound {
        eprintln!(
            "ladder: {:.1} s of work of its bound of {bound:.0} s (a budget of {ladder_budget:.0} s)",
            ladder_work_now.get()
        );
    }
    result.kicad_checks_charged = checks_charged.get().checks;
    result.kicad_check_work_seconds = checks_charged.get().work();
    // What did not run matters only to a board left incomplete.
    if final_open.0 > 0 || final_open.1 > 0 {
        result.skipped = skipped;
    }
    let report_path = output_directory.join("board-router.json");
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", report_path.display()))?;
    Ok(result)
}

/// Routes `board` with seeds `first..first + count` in parallel (seed 0 is
/// the deterministic order) and returns the best result, with the largest
/// router's lattice maps in megabytes (`route_measured`).
pub(super) fn route_seeds(
    board: &core::Board,
    config: &core::Config,
    first: u64,
    count: usize,
    side_by_side: usize,
) -> (core::RoutingResult, f64) {
    // In waves of `side_by_side` (memory), in seed order.
    let seeds: Vec<u64> = (first..first + count.max(1) as u64).collect();
    let mut results: Vec<(Option<u64>, core::RoutingResult)> = Vec::new();
    let mut maps_mb = 0.0f64;
    for wave in seeds.chunks(side_by_side.max(1)) {
        let routed: Vec<(Option<u64>, (core::RoutingResult, f64))> = std::thread::scope(|scope| {
            let handles: Vec<_> = wave
                .iter()
                .map(|&index| {
                    let mut seeded = config.clone();
                    seeded.seed = (index > 0).then_some(index);
                    seeded.verbose = config.verbose && index == first;
                    scope.spawn(move || (seeded.seed, route_measured(board, &seeded)))
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("routing thread"))
                .collect()
        });
        for (seed, (result, maps)) in routed {
            maps_mb = maps_mb.max(maps);
            results.push((seed, result));
        }
    }
    let key = |result: &core::RoutingResult| {
        let open: usize = result
            .status
            .iter()
            .map(|status| match status {
                core::NetStatus::Partial {
                    unconnected_terminals,
                } => *unconnected_terminals,
                core::NetStatus::Unreachable => 1,
                _ => 0,
            })
            .sum::<usize>()
            + core::verify(board, &result.routes).len();
        let vias: usize = result.routes.iter().map(|route| route.vias.len()).sum();
        let length: f64 = result
            .routes
            .iter()
            .flat_map(|route| route.segments.iter())
            .map(|segment| distance_squared(segment.start, segment.end).sqrt())
            .sum();
        (open, vias, length)
    };
    let scored: Vec<_> = results
        .into_iter()
        .map(|(seed, result)| (key(&result), seed, result))
        .collect();
    for (score, seed, _) in &scored {
        eprintln!(
            "seed {seed:?}: {} open, {} vias, {:.0} mm",
            score.0, score.1, score.2
        );
    }
    let best = scored
        .into_iter()
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .expect("at least one seed")
        .2;
    (best, maps_mb)
}

/// The core router configuration for a KiCad configuration.
pub(crate) fn core_config(config: &KiCadBoardRouterConfig) -> core::Config {
    let mut router_config = core::Config {
        verbose: true,
        deadline: config.deadline,
        deadline_work: config.router_deadline_work,
        ..core::Config::default()
    };
    if let Some(via_cost) = config.via_cost_mm {
        router_config.via_cost = via_cost;
    }
    if let Some(strict) = config.strict_via_budget {
        router_config.strict_via_budget = strict;
    }
    router_config.run_work = config.run_work;
    if let Some(seconds) = config.cleanup_seconds {
        router_config.cleanup_seconds = router_config.cleanup_seconds.min(seconds);
    }
    if let Some(against) = config.against_direction_cost {
        router_config.against_direction = against;
    }
    if let Some(iterations) = config.maximum_iterations {
        router_config.max_iterations = iterations;
    }
    if let Some(pitches) = &config.grid_pitches_mm {
        router_config.pitches = pitches.clone();
    }
    if let Some(weight) = config.heuristic_weight {
        router_config.heuristic_weight = weight;
    }
    if let Some(cap) = config.present_cap {
        router_config.present_cap = cap;
    }
    if let Some(factor) = config.present_factor {
        router_config.present_factor = factor;
    }
    if let Some(growth) = config.present_growth {
        router_config.present_growth = growth;
    }
    if let Some(drop) = config.stall_drop {
        router_config.stall_drop = drop;
    }
    if let Some(stall) = config.stall_at_cap {
        router_config.stall_at_cap = stall;
    }
    if let Some(seconds) = config.negotiation_seconds {
        // A budget given in seconds is turned into work (search
        // expansions), so that results do not depend on the machine's
        // load; the clock stays as a guard.
        router_config.negotiation_expansions =
            (seconds * core::router::expansions_per_second()) as u64;
        router_config.negotiation_seconds = seconds * core::router::GUARD;
        // The polish after it may not take longer than the negotiation was
        // allowed.
        router_config.via_reduction_seconds = seconds.min(300.0);
    } else {
        // The router's default budget, in work too: left in seconds it was
        // the only limit of an attempt the ladder repeats from a probed
        // rung (its seeds, the narrow-signal and relaxed-clearance steps),
        // and video's narrow-signal attempt stopped at 900 s of wall clock
        // wherever the load had brought it (1124 s of work on one run).
        let seconds = router_config.negotiation_seconds;
        router_config.negotiation_expansions =
            (seconds * core::router::expansions_per_second()) as u64;
        router_config.negotiation_seconds = seconds * core::router::GUARD;
    }
    if let Some(length) = config.escape_stub_mm {
        router_config.escape_stub_mm = length;
    }
    if let Some(fixed) = config.fixed_plane_stubs {
        router_config.fixed_plane_stubs = fixed;
    }
    if let Some(cost) = config.thermal_guard_cost {
        router_config.thermal_guard_cost = cost;
    }
    if let Some(global) = config.global_routing {
        router_config.global_routing = global;
    }
    if let Some(present) = config.via_reduction_present {
        router_config.via_reduction_present = present;
    }
    if let Some(exclusive) = config.exclusive_planes {
        router_config.exclusive_planes = exclusive;
    }
    if let Some(islands) = config.pour_islands {
        router_config.pour_islands = islands;
    }
    if let Some(kicad_spokes) = config.kicad_spokes {
        router_config.kicad_spokes = kicad_spokes;
    }
    if let Some(manual_spokes) = config.manual_spokes {
        router_config.manual_spokes = manual_spokes;
    }
    if let Some(reach) = config.neck_reach_mm {
        router_config.neck_reach = reach;
    }
    if let Some(cost) = config.plane_cut_cost {
        router_config.plane_cut_cost = cost;
    }
    if let Some(cost) = config.cleanup_via_cost_mm {
        router_config.cleanup_via_cost = cost;
    }
    if let Some(passes) = config.cleanup_passes {
        router_config.cleanup_passes = passes;
    }
    if let Some(cost) = config.bend_cost_mm {
        router_config.bend_cost = cost;
    }
    if let Some(rounds) = config.via_reduction_rounds {
        router_config.via_reduction_rounds = rounds;
    }
    if let Some(skeleton) = config.plane_skeleton {
        router_config.plane_skeleton = skeleton;
    }
    if let Some(bias) = config.skeleton_bias {
        router_config.skeleton_bias = bias;
    }
    router_config
}

/// Writes `pcb` (with its copper) as the project in `output_directory`,
/// verifies it natively unless told not to, and builds the report.
#[allow(clippy::too_many_arguments)]
pub(crate) fn finish_routed_board(
    pcb: &Expr,
    board: &core::Board,
    result: &core::RoutingResult,
    nets: Vec<KiCadBoardRouterNet>,
    layer_names: &[String],
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardRouterConfig,
    timings: [f64; 2],
) -> Result<KiCadBoardRouterResult, String> {
    let verify_started = std::time::Instant::now();
    let violations = core::verify(board, &result.routes);
    let verify_seconds = verify_started.elapsed().as_secs_f64();
    if output_directory.exists() {
        return Err(format!(
            "output directory {} already exists",
            output_directory.display()
        ));
    }
    copy_directory_tree(source_directory, output_directory)?;
    let output_board = output_directory.join(format!("{board_id}.kicad_pcb"));
    fs::write(&output_board, format!("{}\n", encode(pcb)))
        .map_err(|error| format!("failed to write {}: {error}", output_board.display()))?;
    let native_started = std::time::Instant::now();
    let native = if config.skip_native_verification {
        None
    } else {
        Some(verify_materialized_rung(output_directory, board_id)?)
    };
    let native_verification_seconds = native_started.elapsed().as_secs_f64();
    let drc_cache = crate::drc_cache_counts();
    let (lowerings, lowering_total) = lowering_totals();
    let (builds, build_total) = core::router::build_totals();
    eprintln!(
        "board check: internal verification {verify_seconds:.1}s, KiCad {} (lowering {:.1}s, routing {:.1}s); drc_cache: {{calls {}, hits {}}}; so far {lowerings} lowerings {lowering_total:.1}s, {builds} lattices {build_total:.1}s",
        if native.is_some() {
            format!("{native_verification_seconds:.1}s")
        } else {
            "not run".to_string()
        },
        timings[0],
        timings[1],
        drc_cache.calls,
        drc_cache.hits
    );
    let routable: Vec<_> = nets.iter().filter(|net| net.terminals >= 2).collect();
    let report = KiCadBoardRouterResult {
        board_id: board_id.into(),
        routable_connections: routable.len(),
        routed_connections: routable.iter().filter(|net| net.status == "routed").count(),
        unconnected_terminals: nets.iter().map(|net| net.unconnected_terminals).sum(),
        vias: nets.iter().map(|net| net.vias).sum(),
        length_mm: nets.iter().map(|net| net.length_mm).sum(),
        grid_pitch_mm: result.grid.pitch,
        grid_nodes: [result.grid.nx, result.grid.ny],
        iterations: result.iterations,
        searches: result.searches,
        expansions: result.expansions,
        lowering_seconds: timings[0],
        routing_seconds: timings[1],
        internal_verification_seconds: 0.0,
        native_verification_seconds,
        internal_violations: violations
            .iter()
            .map(|violation| KiCadBoardRouterViolation {
                connection: board.nets[violation.net as usize].name.clone(),
                other: violation.other.clone(),
                layer: layer_names[violation.layer].clone(),
                at: violation.at,
                required_mm: violation.required,
                actual_mm: violation.actual,
            })
            .collect(),
        native,
        pours: String::new(),
        nets,
        congestion: result.congestion.clone(),
        grid_origin: result.grid.origin,
        pin_swaps: None,
        diagnostics: KiCadRoutingDiagnostics {
            starved_thermals: starved_thermal_diagnostics(output_directory),
            ..diagnostics(board, result, layer_names)
        },
        cut: Vec::new(),
        work_seconds: 0.0,
        kicad_check_work_seconds: 0.0,
        kicad_checks_charged: 0,
        skipped: Vec::new(),
        neck_repair: None,
        rules: KiCadRulesSummary::of(board, config),
        maps_mb: 0.0,
        work_bound_seconds: None,
        memory_limits: String::new(),
    };
    let report_path = output_directory.join("board-router.json");
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", report_path.display()))?;
    Ok(report)
}

/// Lays tracks over KiCad's connection_width necks on the finished board
/// in `directory` (`necks::repair_connection_width`) and, when the board
/// changed, takes its new native check and starved thermals.
/// Returns the KiCad runs it made: its DRC checks and, when it repaired
/// anything, KiCad's fill of a scratch copy (each a check's work,
/// `check_work`).
pub(crate) fn repair_necks(
    result: &mut KiCadBoardRouterResult,
    directory: &Path,
    board_id: &str,
    config: &KiCadBoardRouterConfig,
) -> Result<usize, String> {
    if config.skip_native_verification {
        return Ok(0);
    }
    let calls = crate::drc_cache_counts().calls;
    let mut fills = 0;
    if let Some(repair) = crate::necks::repair_connection_width(directory, board_id, config)? {
        fills = repair.kicad_fills;
        if repair.kept {
            result.native = Some(verify_materialized_rung(directory, board_id)?);
            result.diagnostics.starved_thermals = starved_thermal_diagnostics(directory);
        }
        result.neck_repair = Some(repair);
    }
    let checks = crate::drc_cache_counts().calls.saturating_sub(calls) as usize;
    NECK_CHECKS.fetch_add(checks, std::sync::atomic::Ordering::Relaxed);
    Ok(checks + fills)
}

/// KiCad checks the neck repair ran in this process (`repair_necks`).
static NECK_CHECKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// How many KiCad checks the neck repair ran in this process.
pub(crate) fn neck_checks() -> usize {
    NECK_CHECKS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Number of `starved_thermal` findings in the directory's native DRC report.
pub(crate) fn starved_thermals(directory: &Path) -> usize {
    let Ok(text) = fs::read_to_string(directory.join("drc.json")) else {
        return 0;
    };
    let Ok(report) = serde_json::from_str::<serde_json::Value>(&text) else {
        return 0;
    };
    report["violations"].as_array().map_or(0, |violations| {
        violations
            .iter()
            .filter(|violation| violation["type"].as_str() == Some("starved_thermal"))
            .count()
    })
}
