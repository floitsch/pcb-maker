// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

mod agent_cli;
mod freerouting_backend;

use std::{env, fs, path::Path, process::ExitCode};

/// The last panic's message (its first line) and place, for the one line
/// `main` prints: the uncaught panic is the last (a panic the ladder
/// caught and isolated came before it; until 2026-10-21 the first was
/// kept, and an uncaught panic after an isolated attempt's was reported
/// with the attempt's message).
static PANIC: std::sync::Mutex<Option<(String, String)>> = std::sync::Mutex::new(None);

/// The commit the binary was built from and the build date (UTC), from
/// build.rs; "unknown" without git.
const VERSION: &str = concat!(env!("BUILD_COMMIT"), " ", env!("BUILD_DATE"));

/// The system allocator, counting the live heap under the memory profile
/// (`PCB_ROUTER_MEM_PROFILE`; otherwise one relaxed load an allocation).
#[global_allocator]
static ALLOCATOR: pcb_kicad::mem_profile::CountingAllocator =
    pcb_kicad::mem_profile::CountingAllocator;

fn main() -> ExitCode {
    // Large blocks go back to the system when freed (a router's maps).
    pcb_kicad::configure_allocator();
    pcb_kicad::mem_profile::start();
    // A panic is reported as one line (and in the run's report) by `main`;
    // RUST_BACKTRACE keeps the default report with its backtrace too.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| text.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".into());
        let place = info
            .location()
            .map(|location| format!(" at {}:{}", location.file(), location.line()))
            .unwrap_or_default();
        if let Ok(mut last) = PANIC.lock() {
            *last = Some((message.lines().next().unwrap_or("").to_string(), place));
        }
        if env::var_os("RUST_BACKTRACE").is_some() && !is_closed_stdout(&message) {
            default_hook(info);
        }
    }));
    pcb_kicad::set_build_version(VERSION);
    // A run that grows past its reservation in the memory ledger and finds
    // no room for more ends with exit 4 and the verdict saying so.
    pcb_kicad::ledger::set_exceeded_handler(|message| {
        agent_cli::report_failure(pcb_kicad::KiCadRunStatus::InternalError, message);
    });
    let arguments: Vec<String> = env::args().skip(1).collect();
    // Every command that runs kicad-cli does so inside a reservation in the
    // memory ledger: one that is not armed (a route, a layout, a check
    // arm themselves) arms when its first check comes (since 2026-10-27).
    if let Some((command, rest)) = arguments.split_first() {
        pcb_kicad::ledger::arm_on_demand(&agent_cli::ledger_label(command, rest));
    }
    let outcome = std::panic::catch_unwind(|| dispatch(arguments));
    // The reservation goes with the run (a panic included; a signal's
    // death by the ledger's pid rule).
    pcb_kicad::ledger::release();
    pcb_kicad::mem_profile::finish();
    let code = match outcome {
        Ok(code) => code,
        Err(payload) => {
            let payload = payload_message(payload.as_ref());
            // A closed stdout (`pcb-maker --help | head`) ends the process
            // quietly: exit 0, or the verdict's code when the run had one.
            // Tested on the uncaught panic's own message.
            if is_closed_stdout(&payload) {
                return ExitCode::from(
                    agent_cli::decided_exit().unwrap_or(pcb_kicad::EXIT_COMPLETE),
                );
            }
            let last = PANIC.lock().ok().and_then(|last| last.clone());
            let message = format!("internal error (panic): {}", uncaught(&payload, last));
            agent_cli::report_failure(pcb_kicad::KiCadRunStatus::InternalError, &message);
            eprintln!("error: {message}");
            pcb_kicad::EXIT_INTERNAL
        }
    };
    ExitCode::from(code)
}

/// Whether a panic message is std's for printing to a closed stdout.
fn is_closed_stdout(message: &str) -> bool {
    message.starts_with("failed printing to stdout") && message.contains("Broken pipe")
}

/// A panic payload's message.
fn payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| text.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

/// The line for the uncaught panic of message `payload`: its first line,
/// with the place the hook recorded for the last panic when that panic is
/// this one (a panic re-raised from a thread runs no hook again).
fn uncaught(payload: &str, last: Option<(String, String)>) -> String {
    let first_line = payload.lines().next().unwrap_or("");
    match last {
        Some((message, place)) if message == first_line => format!("{message}{place}"),
        _ => first_line.to_string(),
    }
}

/// Runs a command and returns its exit code (`agent_cli::EXIT_CODES`).
fn dispatch(arguments: Vec<String>) -> u8 {
    let command = arguments.first().cloned().unwrap_or_else(|| "help".into());
    if command == "--version" || command == "-V" || command == "version" {
        println!("pcb-maker {VERSION}");
        return pcb_kicad::EXIT_COMPLETE;
    }
    let rest: Vec<String> = arguments.iter().skip(1).cloned().collect();
    // pcb-maker <command> --help, pcb-maker help <command>.
    let asked = if command == "help" || command == "--help" || command == "-h" {
        Some(rest.first().cloned())
    } else if rest
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        Some(Some(command.clone()))
    } else {
        None
    };
    if let Some(message) =
        agent_cli::removed(asked.clone().flatten().as_deref().unwrap_or(&command))
    {
        eprintln!("error: {message}");
        return pcb_kicad::EXIT_USAGE;
    }
    if let Some(topic) = asked {
        return match topic {
            None => {
                print_help();
                pcb_kicad::EXIT_COMPLETE
            }
            Some(topic) => match agent_cli::help(&topic).or_else(|| research_usage(&topic)) {
                Some(help) => {
                    println!("{help}");
                    pcb_kicad::EXIT_COMPLETE
                }
                None => {
                    eprintln!("error: unknown command {topic:?}; pcb-maker --help lists them");
                    pcb_kicad::EXIT_USAGE
                }
            },
        };
    }
    if let Some(outcome) = agent_cli::run(&command, rest) {
        return match outcome {
            Ok(status) => status.exit_code(),
            Err(agent_cli::Failure::Usage(message)) => {
                agent_cli::report_failure(pcb_kicad::KiCadRunStatus::UsageError, &message);
                eprintln!("error: {}", one_line(&message));
                pcb_kicad::EXIT_USAGE
            }
            Err(agent_cli::Failure::Internal(message)) => {
                agent_cli::report_failure(pcb_kicad::KiCadRunStatus::InternalError, &message);
                eprintln!("error: {}", one_line(&message));
                pcb_kicad::EXIT_INTERNAL
            }
        };
    }
    // Research and benchmark commands: usage errors exit 2, anything else
    // that fails (or is incomplete) 1.
    match run(arguments) {
        Ok(()) => pcb_kicad::EXIT_COMPLETE,
        Err(error) => {
            eprintln!("error: {}", one_line(&error));
            if error.starts_with("usage:") || error.starts_with("unknown command") {
                pcb_kicad::EXIT_USAGE
            } else {
                pcb_kicad::EXIT_INCOMPLETE
            }
        }
    }
}

/// An error message as one line (kicad-cli transcripts span many).
fn one_line(message: &str) -> String {
    let mut lines = message.lines().filter(|line| !line.trim().is_empty());
    let first = lines.next().unwrap_or("").to_string();
    let more = lines.count();
    if more == 0 {
        first
    } else {
        format!("{first} (+{more} more lines)")
    }
}

fn print_help() {
    println!(
        "pcb-maker: automatic placement and routing for KiCad boards, judged by KiCad's own ERC and DRC.\n"
    );
    println!("Version {VERSION} (pcb-maker --version: the commit and the build date).\n");
    println!("Commands for agents (pcb-maker <command> --help for its arguments and defaults):");
    for (name, line, _) in agent_cli::AGENT_COMMANDS {
        println!("  {name:<24} {line}");
    }
    println!(
        "\nA typical run: import-kicad-netlist, edit constraints.json, layout-kicad-board, fab-kicad-board"
    );
    println!(
        "(docs/agents.md). Rules come from the project's .kicad_pro; kicad-cli (KiCad 10) must be"
    );
    println!("on the PATH.\n");
    println!("{}\n", agent_cli::EXIT_CODES);
    println!("Environment (docs/handover.md, \"Switches\", lists every variable):");
    for (name, effect) in ENVIRONMENT_HELP {
        let mut lines = effect.lines();
        println!("  {name:<26} {}", lines.next().unwrap_or(""));
        for line in lines {
            println!("  {:<26} {line}", "");
        }
    }
    println!();
    println!(
        "Research and benchmark commands (no detailed help; exit 0 done, 1 failed or incomplete, 2 usage,"
    );
    println!("4 panic):");
    for line in RESEARCH_USAGE {
        println!("  pcb-maker {line}");
    }
}

/// The user-facing environment variables `--help` lists, with their
/// effect (tools/config-reference checks that every user-facing variable
/// of its index is here).
const ENVIRONMENT_HELP: &[(&str, &str)] = &[
    (
        "RAYON_NUM_THREADS",
        "search threads of the router and the placer; default: every core.",
    ),
    (
        "PCB_WORK_RATE",
        "<expansions/s>[,<placer units/s>]: the work rates pinned, not measured at the\nstart (a verdict's work_rate and placer_work_rate repeat its run).",
    ),
    (
        "PCB_ROUTER_SEEDS",
        "with router config auto: route that many ways and keep the best; default 1.",
    ),
    (
        "PCB_SKIP_NATIVE",
        "skip KiCad's check (the verdict is then router_complete_unverified at\nbest, exit 1); default unset.",
    ),
    (
        "PCB_KICAD_CLI",
        "the kicad-cli program (KiCad 10); default kicad-cli on the PATH.",
    ),
    (
        "KICAD10_FOOTPRINT_DIR",
        "KiCad's footprint libraries for import-kicad-netlist; default: the\nstandard install folders.",
    ),
    (
        "PCB_ROUTER_NO_WALL_GUARD",
        "the work alone ends runs, never the wall clock; default unset.",
    ),
    (
        "PCB_PLACER_WORK_SECONDS",
        "place-kicad-board's work budget in seconds of work; default 600.",
    ),
    (
        "PCB_KICAD_TIMEOUT",
        "seconds one kicad-cli report may take before it is killed; default 900.",
    ),
    (
        "PCB_MIN_CONNECTION_FLOOR",
        "errors: floor track widths at the minimum connection width only when KiCad\nrates connection_width an error; off: never; default: at error and warning severity.",
    ),
    (
        "PCB_KICAD_NO_NECK_REPAIR",
        "no connection-width neck repair after the final DRC; default unset.",
    ),
    (
        "PCB_ROUTER_ISLAND_STUBS",
        "pads get straight tracks into fill islands their spokes reach (silences\nKiCad's isolated-island finding); default unset (off).",
    ),
    (
        "PCB_MAKER_CACHE_DIR",
        "root of the DRC cache; default $XDG_CACHE_HOME/pcb-maker or ~/.cache/pcb-maker.",
    ),
    ("PCB_MAKER_NO_CACHE", "1: no DRC cache; default unset."),
    (
        "PCB_MAKER_KICAD_USER_LIBS",
        "1: KiCad's checks load the user's global libraries; default: empty tables.",
    ),
    (
        "PCB_MAKER_MEMORY_MB",
        "a memory limit in MB: the ladder runs as many routers at once as fit.",
    ),
    (
        "PCB_MAKER_LEDGER",
        "the machine's memory ledger (routes, layouts and KiCad's checks reserve their\nmemory in it and wait for room; pcb-maker ledger lists it); off: none; default\n$XDG_RUNTIME_DIR/pcb-maker-ledger.json, else /tmp/pcb-maker-ledger.json.",
    ),
    (
        "PCB_MAKER_LEDGER_MARGIN_MB",
        "the least memory the ledger keeps free of reservations: its capacity is MemTotal\nless the memory outside it (the headroom samples' median, else 25 % of\nMemTotal), at most MemTotal less this margin; default 6144.",
    ),
    (
        "PCB_MAKER_LEDGER_TOTAL_MB",
        "what the ledger's reservations may take together, in place of the capacity\nfrom the machine's memory (tests); default unset.",
    ),
    (
        "PCB_MAKER_LEDGER_INSIDE",
        "`<pid> <started>`: the reservation this process runs inside (run.py sets it for\nthe binaries of a board it holds the memory of); only what the process needs\nbeyond it is its own; default unset.",
    ),
    (
        "PCB_MAKER_LEDGER_SHORT",
        "1: this command's requests of up to 2 GB beside a KiCad check are short (they\ngo once the memory is there, never behind a drain); 0: none are; default:\nevery armed command but a route or a layout.",
    ),
];

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
    let first = ledger::first_waiter(&all).map(|waiter| {
        let available = ledger::available_less_growth(&all);
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
                "drained_for": state == ledger::Drain::Drains,
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
    Ok(())
}

/// The usage line of a research command.
fn research_usage(command: &str) -> Option<String> {
    RESEARCH_USAGE
        .iter()
        .find(|line| line.split_whitespace().next() == Some(command))
        .map(|line| format!("usage: pcb-maker {line}"))
}

/// Usage lines of the research and benchmark commands, without the
/// leading `pcb-maker`.
const RESEARCH_USAGE: &[&str] = &[
    "ledger [--json]",
    "verify-kicad-rung <directory> <board-id>",
    "normalize-kicad-footprint-links <directory> <board-id> [report.json]",
    "shrink-kicad-board <source.kicad_pcb> <output.kicad_pcb> <factor>",
    "inspect-kicad-board <board.kicad_pcb>",
    "inspect-kicad-connections <board.kicad_pcb>",
    "render-kicad-layers <board.kicad_pcb> <output-prefix> [config.json]",
    "route-kicad-board-freerouting <source-directory> <board-id> <output-directory> <config.json>",
    "export-congestion-sample <source-directory> <board-id> <output-directory> [job.json] [router-config.json|auto]",
    "score-congestion-sample <sample.json> <model.onnx>",
    "score-congestion <source-directory> <board-id> <model.onnx> [router-config.json|auto]",
    "legality-kicad-board <project-dir> <board-id> <report.json> [--poses <board.kicad_pcb>] [--drc <drc.json>] [--placer <placer.json>]",
    "ablate-kicad-placement <designer.kicad_pcb> <placed-directory> <board-id> <output-directory> [ablation.json] [router-config.json|auto]",
    "global-route-kicad-board <source-directory> <board-id> [router-config.json|auto] [--global <global.json>] [--probe <work-seconds>] [--corridors <out.json>]",
    "probe-trouble-placement <placed.kicad_pcb> [router-config.json|auto] (--trouble-from <run-dir|board-router.json|probe.json> | --trouble <names.txt> | --auto)... [--nets <n>] [--background none|stubs|global|full] [--share <0.05>] [--total-seconds <1500>] [--work-seconds <s>] [--stub-mm <1.0>] [--global-weight <2.0>] [--out <report.json>]",
    "dump-pour-map <source-directory> <board-id> <output-directory> [config.json|auto]",
    "bench-search <source-directory> <board-id> <iterations> [config.json|auto]",
    "calibrate-work-rate [rounds]",
];

/// The commands without the agent contract (and the agent commands that
/// only read and write files, which `agent_cli` checks first).
pub(crate) fn run(arguments: Vec<String>) -> Result<(), String> {
    let mut arguments = arguments.into_iter();
    let command = arguments.next().unwrap_or_else(|| "help".into());
    match command.as_str() {
        "ledger" => {
            let json = match arguments.next().as_deref() {
                None => false,
                Some("--json") => true,
                Some(_) => return Err("usage: pcb-maker ledger [--json]".into()),
            };
            print_ledger(json)
        }
        "verify-kicad-rung" => {
            let directory = arguments.next().ok_or_else(|| {
                "usage: pcb-maker verify-kicad-rung <directory> <board-id>".to_string()
            })?;
            let board_id = arguments.next().ok_or_else(|| {
                "usage: pcb-maker verify-kicad-rung <directory> <board-id>".to_string()
            })?;
            pcb_kicad::check_board_id(&board_id)?;
            if arguments.next().is_some() {
                return Err("usage: pcb-maker verify-kicad-rung <directory> <board-id>".into());
            }
            let report = pcb_kicad::verify_materialized_rung(Path::new(&directory), &board_id)?;
            println!(
                "{}: ERC={}, DRC={}, parity={}, selected-net-unconnected={}, intentional-no-connect-groups={}, library-metadata-warnings={}",
                if report.complete {
                    "complete"
                } else {
                    "incomplete"
                },
                report.erc_violations,
                report.drc_design_violations,
                report.schematic_parity_issues,
                report.selected_net_unconnected_items,
                report.intentional_no_connect_groups,
                report.library_metadata_warnings
            );
            if report.complete {
                Ok(())
            } else {
                Err("KiCad rung did not pass the completion policy".into())
            }
        }
        "repair-kicad-silkscreen" => {
            let usage = "usage: pcb-maker repair-kicad-silkscreen <source-directory> <board-id> <output-directory>";
            let source = arguments.next().ok_or(usage)?;
            let board_id = arguments.next().ok_or(usage)?;
            pcb_kicad::check_board_id(&board_id)?;
            let output = arguments.next().ok_or(usage)?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let report = pcb_kicad::repair_kicad_silkscreen(
                Path::new(&source),
                &board_id,
                Path::new(&output),
            )?;
            println!(
                "wrote {output}: silkscreen-complete={}, remaining-annotation-findings={}",
                report["silkscreen_complete"], report["remaining_annotation_findings"]
            );
            if report["silkscreen_complete"] == true {
                Ok(())
            } else {
                let report_path =
                    std::path::absolute(Path::new(&output).join("silkscreen-repair.json"))
                        .unwrap_or_else(|_| Path::new(&output).join("silkscreen-repair.json"));
                Err(format!(
                    "{}: {} silkscreen findings left; see {} (fields.blocked and proposal.unresolved_marks; KiCad's findings in drc.json and the rendering beside it): move those labels or marks by hand in KiCad, or give their parts room, and run repair-kicad-silkscreen {output} {board_id} <new-dir> again",
                    agent_cli::SILKSCREEN_INCOMPLETE,
                    report["remaining_annotation_findings"],
                    report_path.display()
                ))
            }
        }
        "normalize-kicad-footprint-links" => {
            let usage = "usage: pcb-maker normalize-kicad-footprint-links <directory> <board-id> [report.json]";
            let directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            pcb_kicad::check_board_id(&board_id)?;
            let report_path = arguments.next();
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let report =
                pcb_kicad::normalize_kicad_footprint_links(Path::new(&directory), &board_id)?;
            let serialized =
                serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
            if let Some(report_path) = report_path {
                fs::write(&report_path, format!("{serialized}\n"))
                    .map_err(|error| format!("failed to write {report_path}: {error}"))?;
                println!(
                    "wrote {report_path}: normalized {} footprint link(s)",
                    report.updated_references.len()
                );
            } else {
                println!("{serialized}");
            }
            Ok(())
        }
        "shrink-kicad-board" => {
            let usage = "usage: pcb-maker shrink-kicad-board <source.kicad_pcb> <output.kicad_pcb> <factor>";
            let source = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let factor: f64 = arguments
                .next()
                .ok_or_else(|| usage.to_string())?
                .parse()
                .map_err(|error| format!("invalid factor: {error}"))?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let report =
                pcb_kicad::shrink_kicad_board(Path::new(&source), Path::new(&output), factor)?;
            println!(
                "wrote {output}: {:.1} x {:.1} mm -> {:.1} x {:.1} mm, {} footprints, {} pushed back inside: {}",
                report.size_before_mm[0],
                report.size_before_mm[1],
                report.size_after_mm[0],
                report.size_after_mm[1],
                report.footprints,
                report.edge_bound.len(),
                report.edge_bound.join(" ")
            );
            Ok(())
        }
        "strip-kicad-tracks" => {
            let usage = "usage: pcb-maker strip-kicad-tracks <source.kicad_pcb> <output.kicad_pcb>";
            let source = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            pcb_kicad::write_kicad_board_without_tracks(Path::new(&source), Path::new(&output))?;
            println!("wrote {output} without tracks and vias; copper pours kept");
            Ok(())
        }
        "strip-kicad-copper" => {
            let usage = "usage: pcb-maker strip-kicad-copper <source.kicad_pcb> <output.kicad_pcb> [report.json]";
            let source = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let report_path = arguments.next();
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let report = pcb_kicad::write_kicad_board_without_copper(
                Path::new(&source),
                Path::new(&output),
            )?;
            let serialized =
                serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
            if let Some(report_path) = report_path {
                fs::write(&report_path, format!("{serialized}\n"))
                    .map_err(|error| format!("failed to write {report_path}: {error}"))?;
                println!(
                    "wrote {output} and {report_path}: removed {} tracks/arcs, {} vias, {} copper zones, and {} copper graphics",
                    report.removed_segments + report.removed_arcs,
                    report.removed_vias,
                    report.removed_copper_zones,
                    report.removed_copper_graphics,
                );
            } else {
                println!("{serialized}");
            }
            Ok(())
        }
        "import-kicad-netlist" => {
            let usage = "usage: pcb-maker import-kicad-netlist <schematic.kicad_sch|netlist.net> <out-dir> <board-id> [--layers N] [--footprints-from <board.kicad_pcb>]";
            let input = arguments.next().ok_or_else(|| usage.to_string())?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            pcb_kicad::check_board_id(&board_id)?;
            let mut layers = 2;
            let mut footprints_from = None;
            while let Some(argument) = arguments.next() {
                match argument.as_str() {
                    "--layers" => {
                        layers = arguments
                            .next()
                            .and_then(|value| value.parse::<usize>().ok())
                            .filter(|value| *value >= 2 && value % 2 == 0)
                            .ok_or_else(|| usage.to_string())?;
                    }
                    "--footprints-from" => {
                        footprints_from = Some(std::path::PathBuf::from(
                            arguments.next().ok_or_else(|| usage.to_string())?,
                        ));
                    }
                    _ => return Err(usage.into()),
                }
            }
            let report = pcb_kicad::import_kicad_netlist_from(
                Path::new(&input),
                Path::new(&output),
                &board_id,
                layers,
                footprints_from.as_deref(),
            )?;
            println!(
                "{}",
                serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
            );
            if !report.missing.is_empty() {
                return Err(agent_cli::IMPORT_INCOMPLETE.into());
            }
            Ok(())
        }
        "export-kicad-fab" => {
            let usage = "usage: pcb-maker export-kicad-fab <project-dir> <board-id> <out-dir>";
            let directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            pcb_kicad::check_board_id(&board_id)?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let report =
                pcb_kicad::export_kicad_fab(Path::new(&directory), &board_id, Path::new(&output))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
            );
            Ok(())
        }
        "score-kicad-board" => {
            let usage = "usage: pcb-maker score-kicad-board <project-dir> <board-id>";
            let directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            pcb_kicad::check_board_id(&board_id)?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let report = pcb_kicad::score_kicad_board(Path::new(&directory), &board_id)?;
            let mut json = serde_json::to_value(&report).map_err(|error| error.to_string())?;
            // The pours' pieces by the router's pour model (the verdict's
            // `pours`), with the project's rules.
            let config = board_router_config(None, &directory, &board_id)?;
            let pours = pcb_kicad::measure_pours(Path::new(&directory), &board_id, &config)?;
            json["pours"] = serde_json::to_value(&pours).map_err(|error| error.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json).map_err(|error| error.to_string())?
            );
            Ok(())
        }
        "describe-kicad-board" => {
            let usage = "usage: pcb-maker describe-kicad-board <project-dir> <board-id> [--constraints-template <file>]";
            let mut positional = Vec::new();
            let mut template = None;
            while let Some(argument) = arguments.next() {
                if argument == "--constraints-template" {
                    template = Some(arguments.next().ok_or_else(|| usage.to_string())?);
                } else {
                    positional.push(argument);
                }
            }
            let [directory, board_id] =
                <[String; 2]>::try_from(positional).map_err(|_| usage.to_string())?;
            pcb_kicad::check_board_id(&board_id)?;
            let description = pcb_kicad::describe_kicad_board(Path::new(&directory), &board_id)?;
            if let Some(file) = template {
                let constraints =
                    pcb_kicad::constraints_template(Path::new(&directory), &board_id)?;
                fs::write(
                    &file,
                    serde_json::to_string_pretty(&constraints)
                        .map_err(|error| error.to_string())?
                        + "\n",
                )
                .map_err(|error| format!("failed to write {file}: {error}"))?;
                eprintln!(
                    "constraints template: {file} ({} edge, {} near entries; read its comments)",
                    constraints["edge"].as_array().map_or(0, Vec::len),
                    constraints["near"].as_array().map_or(0, Vec::len)
                );
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&description).map_err(|error| error.to_string())?
            );
            Ok(())
        }
        "inspect-kicad-board" => {
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
        "inspect-kicad-connections" => {
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
        "score-congestion" => {
            let usage = "usage: pcb-maker score-congestion <source-directory> <board-id> <model.onnx> [router-config.json|auto]";
            let source = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            pcb_kicad::check_board_id(&board_id)?;
            let model = arguments.next().ok_or_else(|| usage.to_string())?;
            let router = board_router_config(arguments.next().as_deref(), &source, &board_id)?;
            let score = pcb_kicad::score_kicad_congestion(
                Path::new(&source),
                &board_id,
                Path::new(&model),
                &router,
            )?;
            println!(
                "{}",
                serde_json::to_string(&score).map_err(|error| error.to_string())?
            );
            Ok(())
        }
        "legality-kicad-board" => {
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
            let report = pcb_kicad::legality_kicad_board(
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
        "ablate-kicad-placement" => {
            let usage = "usage: pcb-maker ablate-kicad-placement <designer.kicad_pcb> <placed-directory> <board-id> <output-directory> [ablation.json] [router-config.json|auto]";
            let designer = arguments.next().ok_or_else(|| usage.to_string())?;
            let placed = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            pcb_kicad::check_board_id(&board_id)?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let config: pcb_kicad::KiCadAblationConfig = match arguments.next().as_deref() {
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
            let report = pcb_kicad::ablate_kicad_placement(
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
        "global-route-kicad-board" => {
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
            let report = pcb_kicad::global_route_kicad_board(
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
        "probe-trouble-placement" => {
            let usage = research_usage("probe-trouble-placement").unwrap_or_default();
            let board = arguments.next().ok_or_else(|| usage.clone())?;
            let mut router_path = None;
            let mut config = pcb_kicad::KiCadTroubleProbeConfig::default();
            let mut out = None;
            let number = |name: &str, value: Option<String>| -> Result<f64, String> {
                let value = value.ok_or_else(|| format!("{name}: a value"))?;
                value
                    .parse::<f64>()
                    .map_err(|_| format!("{name}: not a number: {value}"))
            };
            while let Some(argument) = arguments.next() {
                match argument.as_str() {
                    "--trouble-from" => config.sources.push(pcb_kicad::KiCadTroubleSource::From(
                        arguments.next().ok_or_else(|| usage.clone())?.into(),
                    )),
                    "--trouble" => config.sources.push(pcb_kicad::KiCadTroubleSource::Names(
                        arguments.next().ok_or_else(|| usage.clone())?.into(),
                    )),
                    "--auto" => config.sources.push(pcb_kicad::KiCadTroubleSource::Auto),
                    "--nets" => config.nets = Some(number("--nets", arguments.next())? as usize),
                    "--background" => {
                        config.background = pcb_kicad::KiCadTroubleBackground::parse(
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
            let report = pcb_kicad::probe_trouble_placement(board, &router, &config)?;
            let text = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
            match out {
                Some(path) => std::fs::write(&path, format!("{text}\n"))
                    .map_err(|error| format!("failed to write {path}: {error}"))?,
                None => println!("{text}"),
            }
            Ok(())
        }
        "score-congestion-sample" => {
            let usage = "usage: pcb-maker score-congestion-sample <sample.json> <model.onnx>";
            let sample = arguments.next().ok_or_else(|| usage.to_string())?;
            let model = arguments.next().ok_or_else(|| usage.to_string())?;
            let (open, overflow, planes) =
                pcb_kicad::score_congestion_sample(Path::new(&sample), Path::new(&model))?;
            println!(
                "{}",
                serde_json::json!({"open": open, "overflow": overflow, "planes": planes})
            );
            Ok(())
        }
        "export-congestion-sample" => {
            let usage = "usage: pcb-maker export-congestion-sample <source-directory> <board-id> <output-directory> [job.json] [router-config.json|auto]";
            let source = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            pcb_kicad::check_board_id(&board_id)?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let job: pcb_kicad::KiCadCongestionJob = match arguments.next() {
                Some(path) => serde_json::from_str(
                    &std::fs::read_to_string(&path)
                        .map_err(|error| format!("failed to read {path}: {error}"))?,
                )
                .map_err(|error| format!("failed to parse {path}: {error}"))?,
                None => Default::default(),
            };
            let router = board_router_config(arguments.next().as_deref(), &source, &board_id)?;
            let reports = pcb_kicad::export_congestion_samples(
                Path::new(&source),
                &board_id,
                Path::new(&output),
                &job,
                &router,
            )?;
            println!(
                "{}",
                serde_json::to_string(&reports).map_err(|error| error.to_string())?
            );
            Ok(())
        }
        "swap-kicad-pins" => {
            let usage = "usage: pcb-maker swap-kicad-pins <source-directory> <board-id> <output-directory> <pin-swaps.json> [config.json]";
            let source = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            pcb_kicad::check_board_id(&board_id)?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let spec = pcb_kicad::read_pin_swap_spec(Path::new(
                &arguments.next().ok_or_else(|| usage.to_string())?,
            ))?;
            let config: pcb_kicad::KiCadPinSwapConfig = match arguments.next() {
                Some(path) => serde_json::from_str(
                    &std::fs::read_to_string(&path)
                        .map_err(|error| format!("failed to read {path}: {error}"))?,
                )
                .map_err(|error| format!("failed to parse {path}: {error}"))?,
                None => Default::default(),
            };
            let result = pcb_kicad::swap_kicad_pins(
                Path::new(&source),
                &board_id,
                Path::new(&output),
                &spec,
                &config,
            )?;
            println!(
                "{} groups, {} units: ratsnest {:.0} mm with {} crossings -> {:.0} mm with {} crossings; {} pins changed, {} schematic pins relabelled; {:.2}s",
                result.groups,
                result.units,
                result.before.length_mm,
                result.before.crossings,
                result.after.length_mm,
                result.after.crossings,
                result.changes.len(),
                result.schematic_labels_changed,
                result.seconds,
            );
            Ok(())
        }
        "dump-pour-map" => {
            let usage = "usage: pcb-maker dump-pour-map <source-directory> <board-id> <output-directory> [config.json|auto]";
            let source = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            pcb_kicad::check_board_id(&board_id)?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let config = board_router_config(arguments.next().as_deref(), &source, &board_id)?;
            pcb_kicad::dump_kicad_pour_map(
                Path::new(&source),
                &board_id,
                Path::new(&output),
                &config,
            )
        }
        "repair-kicad-necks" => {
            let usage = "usage: pcb-maker repair-kicad-necks <routed-directory> <board-id> [config.json|auto] (changes the directory in place)";
            let directory = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            pcb_kicad::check_board_id(&board_id)?;
            let config = board_router_config(arguments.next().as_deref(), &directory, &board_id)?;
            let repair = pcb_kicad::repair_kicad_necks(Path::new(&directory), &board_id, &config)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&repair).map_err(|error| error.to_string())?
            );
            Ok(())
        }
        "calibrate-work-rate" => {
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
        "bench-search" => {
            let usage = "usage: pcb-maker bench-search <source-directory> <board-id> <iterations> [config.json|auto]";
            let source = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            pcb_kicad::check_board_id(&board_id)?;
            let iterations: usize = arguments
                .next()
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| usage.to_string())?;
            let config = board_router_config(arguments.next().as_deref(), &source, &board_id)?;
            println!(
                "{}",
                pcb_kicad::bench_kicad_search(Path::new(&source), &board_id, iterations, &config)?
            );
            Ok(())
        }
        "route-kicad-board-freerouting" => {
            let usage = "usage: pcb-maker route-kicad-board-freerouting <source-directory> <board-id> <output-directory> <config.json>";
            let source = arguments.next().ok_or_else(|| usage.to_string())?;
            let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
            pcb_kicad::check_board_id(&board_id)?;
            let output = arguments.next().ok_or_else(|| usage.to_string())?;
            let config = arguments.next().ok_or_else(|| usage.to_string())?;
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            freerouting_backend::run(
                Path::new(&source),
                &board_id,
                Path::new(&output),
                Path::new(&config),
            )
        }
        "render-kicad-layers" => {
            let usage = "usage: pcb-maker render-kicad-layers <board.kicad_pcb> <output-prefix> [config.json]";
            let pcb_path = arguments.next().ok_or_else(|| usage.to_string())?;
            let output_prefix = arguments.next().ok_or_else(|| usage.to_string())?;
            let config = match arguments.next() {
                Some(path) => {
                    let source = fs::read_to_string(&path)
                        .map_err(|error| format!("failed to read {path}: {error}"))?;
                    serde_json::from_str(&source)
                        .map_err(|error| format!("failed to parse {path}: {error}"))?
                }
                None => pcb_kicad::KiCadLayerRenderConfig::default(),
            };
            if arguments.next().is_some() {
                return Err(usage.into());
            }
            let report = pcb_kicad::render_board_layers(
                Path::new(&pcb_path),
                Path::new(&output_prefix),
                &config,
            )?;
            println!(
                "wrote {} and {}",
                report.front.display(),
                report.back.display()
            );
            Ok(())
        }
        other => Err(format!("unknown command {other:?}")),
    }
}

/// A router configuration from a file, or resolved from the KiCad project
/// when the path is absent or `auto`.
fn board_router_config(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_uncaught_panic_is_reported_with_its_own_message() {
        // The last panic the hook saw is the uncaught one: its place.
        let last = Some(("index out of bounds".to_string(), " at a.rs:3".to_string()));
        assert_eq!(
            uncaught("index out of bounds\nmore", last.clone()),
            "index out of bounds at a.rs:3"
        );
        // A panic the hook saw before (an isolated attempt's) is not this
        // one: the payload's message, without a place.
        assert_eq!(uncaught("the board's limit", last), "the board's limit");
        assert_eq!(uncaught("x", None), "x");
        // The closed-stdout test reads the uncaught panic's message.
        assert!(is_closed_stdout(
            "failed printing to stdout: Broken pipe (os error 32)"
        ));
        assert!(!is_closed_stdout("index out of bounds"));
        let payload: Box<dyn std::any::Any + Send> = Box::new(String::from("owned"));
        assert_eq!(payload_message(payload.as_ref()), "owned");
    }
}
