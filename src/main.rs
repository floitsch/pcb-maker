// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

mod agent_cli;
mod kicad_commands;
mod lab_commands;

pub(crate) use lab_commands::board_router_config;

use std::{env, process::ExitCode};

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
    // no room for more waits for it, and when the memory runs short ends
    // with exit 4 and the verdict saying so (ledger::overgrown_ends).
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
    for line in COMMANDS.iter().filter_map(|entry| entry.usage) {
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
        "place-kicad-board's work budget in seconds of work; default the layout's share, 40 % of total_seconds (600).",
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

/// The arguments after a command's name.
type Arguments = std::vec::IntoIter<String>;

/// A research or utility command: what it is called, its line in
/// `--help` (without the leading `pcb-maker`; none for commands whose help
/// `agent_cli::help` gives), and what runs it.
struct Command {
    name: &'static str,
    usage: Option<&'static str>,
    run: fn(Arguments) -> Result<(), String>,
}

/// The commands without the agent contract (and the agent commands that
/// only read and write files, which `agent_cli` checks first), in the
/// order `--help` lists the ones with a usage line. The dispatch and the
/// usage list both read this table.
const COMMANDS: &[Command] = &[
    Command {
        name: "ledger",
        usage: Some("ledger [--json]"),
        run: lab_commands::cmd_ledger,
    },
    Command {
        name: "verify-kicad-rung",
        usage: Some("verify-kicad-rung <directory> <board-id>"),
        run: kicad_commands::cmd_verify_kicad_rung,
    },
    Command {
        name: "normalize-kicad-footprint-links",
        usage: Some("normalize-kicad-footprint-links <directory> <board-id> [report.json]"),
        run: kicad_commands::cmd_normalize_kicad_footprint_links,
    },
    Command {
        name: "shrink-kicad-board",
        usage: Some("shrink-kicad-board <source.kicad_pcb> <output.kicad_pcb> <factor>"),
        run: kicad_commands::cmd_shrink_kicad_board,
    },
    Command {
        name: "inspect-kicad-board",
        usage: Some("inspect-kicad-board <board.kicad_pcb>"),
        run: lab_commands::cmd_inspect_kicad_board,
    },
    Command {
        name: "inspect-kicad-connections",
        usage: Some("inspect-kicad-connections <board.kicad_pcb>"),
        run: lab_commands::cmd_inspect_kicad_connections,
    },
    Command {
        name: "pad-exits-kicad-board",
        usage: Some(
            "pad-exits-kicad-board <project-dir> <board-id> [router-config.json|auto] [--out <report.json>] [--repair]",
        ),
        run: lab_commands::cmd_pad_exits_kicad_board,
    },
    Command {
        name: "legality-kicad-board",
        usage: Some(
            "legality-kicad-board <project-dir> <board-id> <report.json> [--poses <board.kicad_pcb>] [--drc <drc.json>] [--placer <placer.json>]",
        ),
        run: lab_commands::cmd_legality_kicad_board,
    },
    Command {
        name: "ablate-kicad-placement",
        usage: Some(
            "ablate-kicad-placement <designer.kicad_pcb> <placed-directory> <board-id> <output-directory> [ablation.json] [router-config.json|auto]",
        ),
        run: lab_commands::cmd_ablate_kicad_placement,
    },
    Command {
        name: "global-route-kicad-board",
        usage: Some(
            "global-route-kicad-board <source-directory> <board-id> [router-config.json|auto] [--global <global.json>] [--probe <work-seconds>] [--corridors <out.json>]",
        ),
        run: lab_commands::cmd_global_route_kicad_board,
    },
    Command {
        name: "probe-trouble-placement",
        usage: Some(
            "probe-trouble-placement <placed.kicad_pcb> [router-config.json|auto] (--trouble-from <run-dir|board-router.json|probe.json> | --trouble <names.txt> | --auto)... [--nets <n>] [--background none|stubs|global|full] [--share <0.05>] [--total-seconds <1500>] [--work-seconds <s>] [--stub-mm <1.0>] [--global-weight <2.0>] [--out <report.json>]",
        ),
        run: lab_commands::cmd_probe_trouble_placement,
    },
    Command {
        name: "probe-coarse-placement",
        usage: Some(
            "probe-coarse-placement <placed.kicad_pcb> [router-config.json|auto] [--pitch-factor <1.0>] [--iterations <n>] [--hard-repair] [--trouble-repair] [--negotiation-share <0.5>] [--share <0.05>] [--total-seconds <1500>] [--work-seconds <s>] [--out <report.json>]",
        ),
        run: lab_commands::cmd_probe_coarse_placement,
    },
    Command {
        name: "dump-pour-map",
        usage: Some(
            "dump-pour-map <source-directory> <board-id> <output-directory> [config.json|auto]",
        ),
        run: lab_commands::cmd_dump_pour_map,
    },
    Command {
        name: "calibrate-work-rate",
        usage: Some("calibrate-work-rate [rounds]"),
        run: lab_commands::cmd_calibrate_work_rate,
    },
    Command {
        name: "sub-boards-kicad-board",
        usage: Some("sub-boards-kicad-board <board.kicad_pcb>"),
        run: lab_commands::cmd_sub_boards_kicad_board,
    },
    Command {
        name: "repair-kicad-silkscreen",
        usage: None,
        run: kicad_commands::cmd_repair_kicad_silkscreen,
    },
    Command {
        name: "strip-kicad-tracks",
        usage: None,
        run: kicad_commands::cmd_strip_kicad_tracks,
    },
    Command {
        name: "strip-kicad-copper",
        usage: None,
        run: kicad_commands::cmd_strip_kicad_copper,
    },
    Command {
        name: "import-kicad-netlist",
        usage: None,
        run: kicad_commands::cmd_import_kicad_netlist,
    },
    Command {
        name: "export-kicad-fab",
        usage: None,
        run: kicad_commands::cmd_export_kicad_fab,
    },
    Command {
        name: "score-kicad-board",
        usage: None,
        run: kicad_commands::cmd_score_kicad_board,
    },
    Command {
        name: "describe-kicad-board",
        usage: None,
        run: kicad_commands::cmd_describe_kicad_board,
    },
    Command {
        name: "swap-kicad-pins",
        usage: None,
        run: kicad_commands::cmd_swap_kicad_pins,
    },
    Command {
        name: "repair-kicad-necks",
        usage: None,
        run: kicad_commands::cmd_repair_kicad_necks,
    },
];

/// The usage line of a research command.
fn research_usage(command: &str) -> Option<String> {
    COMMANDS
        .iter()
        .find(|entry| entry.name == command)
        .and_then(|entry| entry.usage)
        .map(|line| format!("usage: pcb-maker {line}"))
}

/// Runs a command of `COMMANDS`; an unknown name is a usage error.
pub(crate) fn run(arguments: Vec<String>) -> Result<(), String> {
    let mut arguments = arguments.into_iter();
    let command = arguments.next().unwrap_or_else(|| "help".into());
    match COMMANDS.iter().find(|entry| entry.name == command) {
        Some(entry) => (entry.run)(arguments),
        None => Err(format!("unknown command {command:?}")),
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

    #[test]
    fn the_commands_table_names_match_their_usage_lines() {
        let mut seen = std::collections::HashSet::new();
        for entry in COMMANDS {
            assert!(seen.insert(entry.name), "{} twice", entry.name);
            if let Some(line) = entry.usage {
                assert_eq!(line.split_whitespace().next(), Some(entry.name));
            }
        }
    }
}
