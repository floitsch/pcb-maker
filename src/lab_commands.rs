// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Research and measurement commands: the memory ledger, the inspectors,
//! the probes and the calibration, mostly over `pcb_kicad_lab`. Each takes
//! the arguments after its name.

use std::path::Path;

use crate::{Arguments, research_usage};

/// What `pcb-maker ledger` says of a reservation's KiCad checks and of the
/// harness's reservation it runs inside.
fn ledger_notes(reservation: &pcb_kicad::ledger::Reservation) -> String {
    let mut notes = String::new();
    if let Some(check) = reservation.check_mb {
        notes.push_str(&format!(
            " [KiCad's check: {check:.0} MB, {}]",
            if reservation.checking {
                "running"
            } else {
                "reserved"
            }
        ));
    }
    if let Some(pid) = reservation.inside {
        notes.push_str(&format!(" [inside pid {pid}'s reservation]"));
    }
    if let Some(granted) = reservation.granted_mb {
        notes.push_str(&format!(
            " [at least the {granted:.0} MB its harness reserved]"
        ));
    }
    notes
}

/// `pcb-maker ledger`: the memory ledger's live reservations, who holds
/// what, as text or JSON.
fn print_ledger(json: bool) -> Result<(), String> {
    use pcb_kicad::ledger;
    let Some(book) = ledger::Ledger::from_env() else {
        println!("memory ledger: off (PCB_MAKER_LEDGER=off)");
        return Ok(());
    };
    let live = book
        .live()
        .map_err(|error| format!("{}: {error}", book.path().display()))?;
    let capacity_described = ledger::capacity_described();
    let capacity = capacity_described.as_ref().map(|(capacity, _)| *capacity);
    let total = ledger::total_mb(&live);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64());
    // The first waiter and whether the ledger drains for it (a new
    // request's view: every entry is another's).
    let all: Vec<&ledger::Reservation> = live.iter().collect();
    let available = ledger::available_less_growth(&all);
    let first = ledger::first_waiter(&all).map(|waiter| {
        (
            waiter,
            ledger::drain_state(waiter, &all, now, capacity, available),
        )
    });
    // The waiter a new request is held to a drain for: the first the ledger
    // drains for, which need not be the first waiter (since 2026-10-28).
    let drained = ledger::drained_for(&all, u32::MAX, now, now, capacity, available)
        .filter(|waiter| first.is_none_or(|(first, _)| !std::ptr::eq(*waiter, first)))
        .map(|waiter| {
            (
                waiter,
                ledger::drain_state(waiter, &all, now, capacity, available),
            )
        });
    // run.py's floor of available memory (before its board's tier's
    // overshoot): measured from the headroom samples, else the default.
    let headroom = book.headroom();
    let (floor, floor_text) = ledger::measured_floor_mb(headroom.as_ref(), 0.0, now);
    if json {
        let value = serde_json::json!({
            "path": book.path(),
            "capacity_mb": capacity,
            "capacity": capacity_described.as_ref().map(|(_, how)| how),
            "checks_mb": live.iter().filter_map(|r| r.check_mb).sum::<f64>(),
            "reserved_mb": total,
            "headroom": headroom,
            "floor_mb": floor,
            "reservations": live,
            "first_waiter": first.map(|(waiter, state)| serde_json::json!({
                "pid": waiter.pid,
                "label": waiter.label,
                "waiting_mb": waiter.waiting_mb,
                "waited_seconds": (now - waiter.waiting_since()).max(0.0),
                "drained_for": state.drains(),
                "drain": state.describe(),
            })),
            "drained_for": drained.map(|(waiter, state)| serde_json::json!({
                "pid": waiter.pid,
                "label": waiter.label,
                "waiting_mb": waiter.waiting_mb,
                "waited_seconds": (now - waiter.waiting_since()).max(0.0),
                "drain": state.describe(),
            })),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    println!(
        "memory ledger {}: {total:.0} MB reserved of {} by {} run{}",
        book.path().display(),
        capacity_described
            .as_ref()
            .map_or("an unknown capacity".to_string(), |(c, how)| format!(
                "{c:.0} MB ({how})"
            )),
        live.len(),
        if live.len() == 1 { "" } else { "s" }
    );
    for reservation in &live {
        println!(
            "  pid {:>7}  {:>6.0} MB{}  {:>6.0} s  {}{}{}",
            reservation.pid,
            reservation.megabytes,
            reservation
                .waiting_mb
                .map_or(String::new(), |wanted| format!(
                    " (waits for {wanted:.0} MB)"
                )),
            (now - reservation.started).max(0.0),
            reservation.label,
            if reservation.nightly() {
                " [nightly]"
            } else if reservation.short {
                " [short]"
            } else {
                ""
            },
            ledger_notes(reservation)
        );
    }
    println!(
        "run.py's floor of available memory: {:.1} GB before a board's tier's overshoot ({floor_text}; {})",
        floor / 1024.0,
        book.headroom_path().display()
    );
    if let Some((waiter, state)) = first {
        println!(
            "first waiter: pid {}, {}, waiting {:.0} s for {:.0} MB: {}",
            waiter.pid,
            waiter.label,
            (now - waiter.waiting_since()).max(0.0),
            waiter.waiting_mb.unwrap_or(0.0),
            state.describe()
        );
    }
    if let Some((waiter, state)) = drained {
        println!(
            "drained for: pid {}, {}, waiting {:.0} s for {:.0} MB: {}",
            waiter.pid,
            waiter.label,
            (now - waiter.waiting_since()).max(0.0),
            waiter.waiting_mb.unwrap_or(0.0),
            state.describe()
        );
    }
    Ok(())
}

/// `ledger`: lists the memory ledger's reservations and waiters.
pub(crate) fn cmd_ledger(mut arguments: Arguments) -> Result<(), String> {
    let json = match arguments.next().as_deref() {
        None => false,
        Some("--json") => true,
        Some(_) => return Err("usage: pcb-maker ledger [--json]".into()),
    };
    print_ledger(json)
}

/// `inspect-kicad-board`: summarizes a board file.
pub(crate) fn cmd_inspect_kicad_board(mut arguments: Arguments) -> Result<(), String> {
    let usage = "usage: pcb-maker inspect-kicad-board <board.kicad_pcb>";
    let board = arguments.next().ok_or_else(|| usage.to_string())?;
    if arguments.next().is_some() {
        return Err(usage.into());
    }
    let statistics = pcb_kicad::inspect_kicad_board(Path::new(&board))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&statistics).map_err(|error| error.to_string())?
    );
    Ok(())
}

/// `inspect-kicad-connections`: reports the connectivity of a board file.
pub(crate) fn cmd_inspect_kicad_connections(mut arguments: Arguments) -> Result<(), String> {
    let usage = "usage: pcb-maker inspect-kicad-connections <board.kicad_pcb>";
    let board = arguments.next().ok_or_else(|| usage.to_string())?;
    if arguments.next().is_some() {
        return Err(usage.into());
    }
    let connections = pcb_kicad::inspect_kicad_routable_connections(Path::new(&board))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&connections).map_err(|error| error.to_string())?
    );
    Ok(())
}

/// `pad-exits-kicad-board`: checks that every pad has an exit.
pub(crate) fn cmd_pad_exits_kicad_board(mut arguments: Arguments) -> Result<(), String> {
    let usage = "usage: pcb-maker pad-exits-kicad-board <project-dir> <board-id> [router-config.json|auto] [--out <report.json>] [--repair]";
    let source = arguments.next().ok_or_else(|| usage.to_string())?;
    let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
    pcb_kicad::check_board_id(&board_id)?;
    let mut router_path = None;
    let mut out = None;
    let mut repair = false;
    while let Some(argument) = arguments.next() {
        if argument == "--repair" {
            repair = true;
        } else if argument == "--out" {
            out = Some(arguments.next().ok_or_else(|| usage.to_string())?);
        } else if router_path.is_none() {
            router_path = Some(argument);
        } else {
            return Err(usage.into());
        }
    }
    let router = board_router_config(router_path.as_deref(), &source, &board_id)?;
    let settings = pcb_kicad::KiCadPadExits::default();
    let census = pcb_kicad::pad_exits_kicad_board(
        Path::new(&source),
        &board_id,
        &router,
        repair.then_some(&settings),
    )?;
    for pad in &census.closed {
        println!("{} {} {}: {}", pad.pad, pad.net, pad.layer, pad.reason);
    }
    println!(
        "{} pads without a way out of {} pads tested ({} left by escape stubs, not judged) on {} parts ({} by other parts or the edge); work {:.3} s (lowering {:.3}, probe {:.3}), cpu lowering {:.3} s, probe {:.3} s",
        census.closed.len(),
        census.pads,
        census.escape_pads,
        census.parts,
        census
            .closed
            .iter()
            .filter(|pad| !pad.footprint_only)
            .count(),
        census.work_seconds,
        census.lower_work_seconds,
        census.probe_work_seconds,
        census.lower_seconds,
        census.probe_seconds
    );
    if let Some(repaired) = &census.repaired {
        for moved in &repaired.moves {
            println!(
                "repair: {} from ({:.2}, {:.2}, {}) to ({:.2}, {:.2}, {}) opens {}",
                moved.part,
                moved.from[0],
                moved.from[1],
                moved.from[2],
                moved.to[0],
                moved.to[1],
                moved.to[2],
                moved.opened.join(", ")
            );
        }
        println!(
            "repair: {} closed before, {} after ({} by other parts or the edge), {} moves, {} candidates, work {:.3} s",
            repaired.closed_before,
            repaired.closed.len(),
            repaired
                .closed
                .iter()
                .filter(|pad| !pad.footprint_only)
                .count(),
            repaired.moves.len(),
            repaired.tried,
            repaired.work_seconds
        );
    }
    if let Some(out) = out {
        let text = serde_json::to_string_pretty(&census).map_err(|error| error.to_string())?;
        std::fs::write(&out, format!("{text}\n"))
            .map_err(|error| format!("failed to write {out}: {error}"))?;
    }
    Ok(())
}

/// `legality-kicad-board`: checks a placement's legality against a routing report.
pub(crate) fn cmd_legality_kicad_board(mut arguments: Arguments) -> Result<(), String> {
    let usage = "usage: pcb-maker legality-kicad-board <project-dir> <board-id> <report.json> [--poses <board.kicad_pcb>] [--drc <drc.json>] [--placer <placer.json>]";
    let directory = arguments.next().ok_or_else(|| usage.to_string())?;
    let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
    pcb_kicad::check_board_id(&board_id)?;
    let output = arguments.next().ok_or_else(|| usage.to_string())?;
    let mut drc = None;
    let mut poses = None;
    let mut config = pcb_kicad::KiCadBoardPlacerConfig::default();
    while let Some(flag) = arguments.next() {
        let value = arguments.next().ok_or_else(|| usage.to_string())?;
        match flag.as_str() {
            "--drc" => drc = Some(value),
            "--poses" => poses = Some(value),
            "--placer" => {
                config = serde_json::from_str(
                    &std::fs::read_to_string(&value)
                        .map_err(|error| format!("failed to read {value}: {error}"))?,
                )
                .map_err(|error| format!("{value}: {error}"))?
            }
            _ => return Err(usage.into()),
        }
    }
    let report = pcb_kicad_lab::legality_kicad_board(
        Path::new(&directory),
        &board_id,
        &config,
        poses.as_deref().map(Path::new),
        drc.as_deref().map(Path::new),
    )?;
    let text = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    std::fs::write(&output, format!("{text}\n"))
        .map_err(|error| format!("failed to write {output}: {error}"))?;
    for level in &report.levels {
        println!(
            "level {}: halo {} spacing {:.3} grid {} inset {} tight {} courtyards touch {}: {} of {} movable illegal{}",
            level.index,
            level.halo_scale,
            level.spacing_mm,
            level.grid_mm,
            level.edge_inset,
            level.tight,
            level.courtyard_spacing,
            level.illegal,
            report.movable,
            match (level.kicad_agrees, level.model_stricter) {
                (Some(agrees), Some(stricter)) =>
                    format!(" ({agrees} with a KiCad error, {stricter} without)"),
                _ => String::new(),
            }
        );
    }
    println!("{output}");
    Ok(())
}

/// `ablate-kicad-placement`: reruns a placement with parts of the pipeline switched off.
pub(crate) fn cmd_ablate_kicad_placement(mut arguments: Arguments) -> Result<(), String> {
    let usage = "usage: pcb-maker ablate-kicad-placement <designer.kicad_pcb> <placed-directory> <board-id> <output-directory> [ablation.json] [router-config.json|auto]";
    let designer = arguments.next().ok_or_else(|| usage.to_string())?;
    let placed = arguments.next().ok_or_else(|| usage.to_string())?;
    let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
    pcb_kicad::check_board_id(&board_id)?;
    let output = arguments.next().ok_or_else(|| usage.to_string())?;
    let config: pcb_kicad_lab::KiCadAblationConfig = match arguments.next().as_deref() {
        None | Some("default") => Default::default(),
        Some(path) => serde_json::from_str(
            &std::fs::read_to_string(path)
                .map_err(|error| format!("failed to read {path}: {error}"))?,
        )
        .map_err(|error| format!("{path}: {error}"))?,
    };
    let router_path = arguments.next();
    if arguments.next().is_some() {
        return Err(usage.into());
    }
    let router = board_router_config(router_path.as_deref(), &placed, &board_id)?;
    let report = pcb_kicad_lab::ablate_kicad_placement(
        Path::new(&designer),
        Path::new(&placed),
        &board_id,
        Path::new(&output),
        &config,
        &router,
    )?;
    let text = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    let path = Path::new(&output).join("ablation.json");
    std::fs::write(&path, format!("{text}\n"))
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    println!("{}", path.display());
    Ok(())
}

/// `global-route-kicad-board`: runs the global router and reports its corridors.
pub(crate) fn cmd_global_route_kicad_board(mut arguments: Arguments) -> Result<(), String> {
    let usage = "usage: pcb-maker global-route-kicad-board <source-directory> <board-id> [router-config.json|auto] [--global <global.json>] [--probe <work-seconds>] [--corridors <out.json>]";
    let source = arguments.next().ok_or_else(|| usage.to_string())?;
    let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
    let mut router_path = None;
    let mut config = pcb_global::GlobalConfig::default();
    let mut probe = None;
    let mut corridors = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--global" => {
                let path = arguments.next().ok_or_else(|| usage.to_string())?;
                config = serde_json::from_str(
                    &std::fs::read_to_string(&path)
                        .map_err(|error| format!("failed to read {path}: {error}"))?,
                )
                .map_err(|error| format!("{path}: {error}"))?;
            }
            "--probe" => {
                let value = arguments.next().ok_or_else(|| usage.to_string())?;
                probe = Some(
                    value
                        .parse::<f64>()
                        .map_err(|_| format!("--probe: not a number: {value}"))?,
                );
            }
            "--corridors" => {
                corridors = Some(arguments.next().ok_or_else(|| usage.to_string())?);
            }
            _ if router_path.is_none() && !argument.starts_with("--") => {
                router_path = Some(argument)
            }
            _ => return Err(usage.into()),
        }
    }
    let router = board_router_config(router_path.as_deref(), &source, &board_id)?;
    let report = pcb_kicad_lab::global_route_kicad_board(
        Path::new(&source),
        &board_id,
        &router,
        &config,
        probe,
        corridors.as_deref().map(Path::new),
    )?;
    println!(
        "{}",
        serde_json::to_string(&report).map_err(|error| error.to_string())?
    );
    Ok(())
}

/// `sub-boards-kicad-board`: reports the board's sub-boards.
pub(crate) fn cmd_sub_boards_kicad_board(mut arguments: Arguments) -> Result<(), String> {
    let usage = research_usage("sub-boards-kicad-board").unwrap_or_default();
    let board = arguments.next().ok_or_else(|| usage.clone())?;
    let report = pcb_kicad::describe_sub_boards(Path::new(&board))?;
    println!(
        "{}",
        serde_json::to_string(&report).map_err(|error| error.to_string())?
    );
    Ok(())
}

/// `probe-trouble-placement`: probes how routable the trouble nets of a placement are.
pub(crate) fn cmd_probe_trouble_placement(mut arguments: Arguments) -> Result<(), String> {
    let usage = research_usage("probe-trouble-placement").unwrap_or_default();
    let board = arguments.next().ok_or_else(|| usage.clone())?;
    let mut router_path = None;
    let mut config = pcb_kicad_lab::KiCadTroubleProbeConfig::default();
    let mut out = None;
    let number = |name: &str, value: Option<String>| -> Result<f64, String> {
        let value = value.ok_or_else(|| format!("{name}: a value"))?;
        value
            .parse::<f64>()
            .map_err(|_| format!("{name}: not a number: {value}"))
    };
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--trouble-from" => config.sources.push(pcb_kicad_lab::KiCadTroubleSource::From(
                arguments.next().ok_or_else(|| usage.clone())?.into(),
            )),
            "--trouble" => config
                .sources
                .push(pcb_kicad_lab::KiCadTroubleSource::Names(
                    arguments.next().ok_or_else(|| usage.clone())?.into(),
                )),
            "--auto" => config.sources.push(pcb_kicad_lab::KiCadTroubleSource::Auto),
            "--nets" => config.nets = Some(number("--nets", arguments.next())? as usize),
            "--background" => {
                config.background = pcb_kicad_lab::KiCadTroubleBackground::parse(
                    &arguments.next().ok_or_else(|| usage.clone())?,
                )?
            }
            "--share" => config.share = number("--share", arguments.next())?,
            "--total-seconds" => {
                config.total_seconds = number("--total-seconds", arguments.next())?
            }
            "--work-seconds" => {
                config.work_seconds = Some(number("--work-seconds", arguments.next())?)
            }
            "--stub-mm" => config.stub_mm = number("--stub-mm", arguments.next())?,
            "--global-weight" => {
                config.global_weight = number("--global-weight", arguments.next())?
            }
            "--out" => out = Some(arguments.next().ok_or_else(|| usage.clone())?),
            _ if router_path.is_none() && !argument.starts_with("--") => {
                router_path = Some(argument)
            }
            _ => return Err(usage),
        }
    }
    let board = Path::new(&board);
    let directory = board
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".into());
    let board_id = board
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .ok_or_else(|| usage.clone())?;
    let router = board_router_config(router_path.as_deref(), &directory, &board_id)?;
    // A probe holds its router for up to minutes: not a short command.
    pcb_kicad::ledger::arm(
        &format!("probe-trouble-placement {}", board.display()),
        false,
    );
    // One router, about the size the race's probe takes; the
    // watchdog raises it while the ledger has room.
    pcb_kicad::ledger::reserve_at_least(1024.0, "the trouble-maker probe's router");
    let report = pcb_kicad_lab::probe_trouble_placement(board, &router, &config)?;
    let text = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    match out {
        Some(path) => std::fs::write(&path, format!("{text}\n"))
            .map_err(|error| format!("failed to write {path}: {error}"))?,
        None => println!("{text}"),
    }
    Ok(())
}

/// `probe-coarse-placement`: probes a placement on a coarse lattice.
pub(crate) fn cmd_probe_coarse_placement(mut arguments: Arguments) -> Result<(), String> {
    let usage = research_usage("probe-coarse-placement").unwrap_or_default();
    let board = arguments.next().ok_or_else(|| usage.clone())?;
    let mut router_path = None;
    let mut config = pcb_kicad_lab::KiCadCoarseProbeConfig::default();
    let mut out = None;
    let number = |name: &str, value: Option<String>| -> Result<f64, String> {
        let value = value.ok_or_else(|| format!("{name}: a value"))?;
        value
            .parse::<f64>()
            .map_err(|_| format!("{name}: not a number: {value}"))
    };
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--pitch-factor" => config.pitch_factor = number("--pitch-factor", arguments.next())?,
            "--iterations" => {
                config.iterations = Some(number("--iterations", arguments.next())? as usize)
            }
            "--hard-repair" => config.hard_repair = true,
            "--trouble-repair" => config.trouble_repair = true,
            "--negotiation-share" => {
                config.negotiation_share = number("--negotiation-share", arguments.next())?
            }
            "--share" => config.share = number("--share", arguments.next())?,
            "--total-seconds" => {
                config.total_seconds = number("--total-seconds", arguments.next())?
            }
            "--work-seconds" => {
                config.work_seconds = Some(number("--work-seconds", arguments.next())?)
            }
            "--out" => out = Some(arguments.next().ok_or_else(|| usage.clone())?),
            _ if router_path.is_none() && !argument.starts_with("--") => {
                router_path = Some(argument)
            }
            _ => return Err(usage),
        }
    }
    if config.trouble_repair && !config.hard_repair {
        return Err("--trouble-repair follows the hard repair: give --hard-repair".into());
    }
    let board = Path::new(&board);
    let directory = board
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".into());
    let board_id = board
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .ok_or_else(|| usage.clone())?;
    let router = board_router_config(router_path.as_deref(), &directory, &board_id)?;
    // A probe holds its router for up to minutes: not a short command.
    pcb_kicad::ledger::arm(
        &format!("probe-coarse-placement {}", board.display()),
        false,
    );
    pcb_kicad::ledger::reserve_at_least(1024.0, "the coarse probe's router");
    let report = pcb_kicad_lab::probe_coarse_placement(board, &router, &config)?;
    let text = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    match out {
        Some(path) => std::fs::write(&path, format!("{text}\n"))
            .map_err(|error| format!("failed to write {path}: {error}"))?,
        None => println!("{text}"),
    }
    Ok(())
}

/// `dump-pour-map`: writes the pour map of a board.
pub(crate) fn cmd_dump_pour_map(mut arguments: Arguments) -> Result<(), String> {
    let usage = "usage: pcb-maker dump-pour-map <source-directory> <board-id> <output-directory> [config.json|auto]";
    let source = arguments.next().ok_or_else(|| usage.to_string())?;
    let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
    pcb_kicad::check_board_id(&board_id)?;
    let output = arguments.next().ok_or_else(|| usage.to_string())?;
    let config = board_router_config(arguments.next().as_deref(), &source, &board_id)?;
    pcb_kicad::dump_kicad_pour_map(Path::new(&source), &board_id, Path::new(&output), &config)
}

/// `calibrate-work-rate`: measures the work rate alone.
pub(crate) fn cmd_calibrate_work_rate(mut arguments: Arguments) -> Result<(), String> {
    // The start-up calibration's measurement alone, `rounds` times
    // (default 1), one JSON line each (docs/router.md, "Determinism
    // and the wall clock").
    let usage = "usage: pcb-maker calibrate-work-rate [rounds]";
    let rounds: usize = match arguments.next() {
        Some(value) => value.parse().map_err(|_| usage.to_string())?,
        None => 1,
    };
    for _ in 0..rounds {
        let measured = pcb_kicad::measure_work_rate();
        println!(
            "{}",
            serde_json::to_string(&measured).map_err(|error| error.to_string())?
        );
    }
    Ok(())
}

/// A router configuration from a file, or resolved from the KiCad project
/// when the path is absent or `auto`.
pub(crate) fn board_router_config(
    path: Option<&str>,
    source: &str,
    board_id: &str,
) -> Result<pcb_kicad::KiCadBoardRouterConfig, String> {
    let mut config = board_router_config_of(path, source, board_id)?;
    // Experiment hook for machines without kicad-cli (with a config file
    // too).
    if std::env::var_os("PCB_SKIP_NATIVE").is_some() {
        config.skip_native_verification = true;
    }
    Ok(config)
}

/// `board_router_config` before the environment's switches.
fn board_router_config_of(
    path: Option<&str>,
    source: &str,
    board_id: &str,
) -> Result<pcb_kicad::KiCadBoardRouterConfig, String> {
    match path {
        None | Some("auto") => {
            let mut config = pcb_kicad::resolve_project_rules(
                &Path::new(source).join(format!("{board_id}.kicad_pro")),
                &Path::new(source).join(format!("{board_id}.kicad_pcb")),
            )?;
            if let Some(seeds) = std::env::var("PCB_ROUTER_SEEDS")
                .ok()
                .and_then(|v| v.parse().ok())
            {
                config.seeds = Some(seeds);
            }
            Ok(config)
        }
        Some(path) => {
            let file = serde_json::from_str(
                &std::fs::read_to_string(path)
                    .map_err(|error| format!("failed to read {path}: {error}"))?,
            )
            .map_err(|error| format!("failed to parse {path}: {error}"))?;
            // The project's rules are the base; the file's keys replace
            // them key by key.
            pcb_kicad::router_config_over_project(
                file,
                pcb_kicad::resolve_project_rules(
                    &Path::new(source).join(format!("{board_id}.kicad_pro")),
                    &Path::new(source).join(format!("{board_id}.kicad_pcb")),
                ),
            )
        }
    }
}
