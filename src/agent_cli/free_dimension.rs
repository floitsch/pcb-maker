// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! `layout-kicad-board` with a `free_dimension`: the board laid out at
//! several sizes of one edge, each size a layout of its own (this binary,
//! run as a child process with the board and constraints at that size),
//! the smallest clean size kept as the run's result.
//!
//! `<out-dir>` ends up as a layout's (`result/`, `placed/`,
//! `board-layout.json`, `verdict.json`: the chosen run's, its paths
//! rewritten), plus `free-dimension.json` (every run) and
//! `free-dimension/<size>mm-<kind>/` (each run's project, config, log and
//! layout). The verdict gains `free_dimension`.

use super::*;
use pcb_kicad::free_dimension::{
    KiCadFreeDimension, KiCadFreeDimensionPlan, KiCadFreeDimensionRun, KiCadFreeDimensionSearch,
    KiCadFreeDimensionStep, format_mm, spawn_probe, write_probe_project,
};
use std::process::Command;

/// What the command was given.
pub(super) struct FreeDimensionLayout<'a> {
    pub source: &'a str,
    pub board_id: &'a str,
    pub output: &'a Path,
    pub router_argument: Option<&'a str>,
    pub router: &'a pcb_kicad::KiCadBoardRouterConfig,
    /// The layout config file's JSON, as written.
    pub layout_json: serde_json::Value,
    pub config: &'a pcb_kicad::KiCadBoardLayoutConfig,
    pub free: &'a KiCadFreeDimension,
    pub reference: Option<&'a str>,
}

/// Memory one layout of a probe may take beside the others (a layout and
/// KiCad's check of a large board: up to about 3 GB).
const PROBE_MEMORY_BYTES: u64 = 3 << 30;

fn available_memory() -> Option<u64> {
    let text = fs::read_to_string("/proc/meminfo").ok()?;
    let line = text
        .lines()
        .find(|line| line.starts_with("MemAvailable:"))?;
    let kilobytes: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kilobytes * 1024)
}

/// The search threads this process may use (`RAYON_NUM_THREADS`, else
/// every core).
fn threads() -> usize {
    std::env::var("RAYON_NUM_THREADS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|count| *count > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|count| count.get())
                .unwrap_or(1)
        })
}

fn status_of(text: &str) -> KiCadRunStatus {
    match text {
        "complete" => KiCadRunStatus::Complete,
        "router_complete_unverified" => KiCadRunStatus::RouterCompleteUnverified,
        "cut" => KiCadRunStatus::Cut,
        "usage_error" => KiCadRunStatus::UsageError,
        "internal_error" => KiCadRunStatus::InternalError,
        _ => KiCadRunStatus::Incomplete,
    }
}

/// Replaces `from` at the start of every string below `value` by `to`.
fn rewrite_paths(value: &mut serde_json::Value, from: &str, to: &str) {
    match value {
        serde_json::Value::String(text) => {
            if let Some(rest) = text.strip_prefix(from) {
                *text = format!("{to}{rest}");
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                rewrite_paths(item, from, to);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values_mut() {
                rewrite_paths(item, from, to);
            }
        }
        _ => {}
    }
}

fn read_value(path: &Path) -> Option<serde_json::Value> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
}

fn write_value(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    fs::write(
        path,
        serde_json::to_string_pretty(value).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

/// One run to start: its size, kind, budget and threads.
struct Job {
    size: f64,
    kind: &'static str,
    budget: f64,
    threads: usize,
}

pub(super) fn layout(run: FreeDimensionLayout) -> Result<KiCadRunStatus, Failure> {
    let command = "layout-kicad-board";
    let started = Instant::now();
    let free = run.free;
    let accept = free.accept_rule().map_err(Failure::Usage)?.to_string();
    let ladder = free.ladder().map_err(Failure::Usage)?;
    let total = run.config.total_seconds;
    let (probe_seconds, confirm_seconds) =
        KiCadFreeDimensionSearch::budgets(free, total).map_err(Failure::Usage)?;
    // The board and its constraints as given.
    let source = Path::new(run.source);
    let board_path = source.join(format!("{}.kicad_pcb", run.board_id));
    let board_text = fs::read_to_string(&board_path).map_err(|error| {
        Failure::Usage(format!("failed to read {}: {error}", board_path.display()))
    })?;
    let constraints = match &run.config.placer.constraints {
        None => None,
        Some(pcb_kicad::KiCadConstraintsSource::Inline(constraints)) => Some(constraints.clone()),
        Some(pcb_kicad::KiCadConstraintsSource::File(path)) => Some(
            pcb_kicad::read_placement_constraints(&source.join(path)).map_err(Failure::Usage)?,
        ),
    };
    let plan = KiCadFreeDimensionPlan::new(
        free,
        &board_text,
        constraints,
        &run.config.placer.fixed,
        &run.config.placer.fixed_patterns,
    )
    .map_err(classify)?;
    // Every size's board, or why it cannot work.
    let mut variants = BTreeMap::new();
    let mut infeasible = Vec::new();
    for size in &ladder {
        match plan.variant(*size) {
            Ok(variant) => {
                variants.insert(format_mm(*size), variant);
            }
            Err(reasons) => infeasible.push(KiCadFreeDimensionRun {
                size_mm: *size,
                kind: "infeasible".into(),
                status: "infeasible".into(),
                reasons,
                ..Default::default()
            }),
        }
    }
    if variants.is_empty() {
        let lines: Vec<String> = infeasible.iter().map(|run| run.summary()).collect();
        return Err(Failure::Usage(format!(
            "free_dimension: no size of the ladder is feasible: {}",
            lines.join("; ")
        )));
    }
    fs::create_dir_all(run.output).map_err(|error| {
        Failure::Internal(format!(
            "failed to create {}: {error}",
            run.output.display()
        ))
    })?;
    let output = std::path::absolute(run.output).unwrap_or_else(|_| run.output.to_path_buf());
    let runs_directory = output.join("free-dimension");
    let outputs = BTreeMap::from([
        (
            "report".to_string(),
            path_text(&output.join("board-layout.json")),
        ),
        (
            "verdict".to_string(),
            path_text(&output.join("verdict.json")),
        ),
        (
            "free_dimension".to_string(),
            path_text(&output.join("free-dimension.json")),
        ),
    ]);
    calibrate_work_rate(Some(run.router))?;
    set_target(command, output.join("board-layout.json"), outputs);
    // The probes share the parent's rates, so that a size's probe is the
    // same however many run beside it.
    let rate = pcb_kicad::work_rate();
    let pinned_rate = format!(
        "{},{}",
        rate.expansions_per_second, rate.placer_work_per_second
    );
    let threads = threads();
    let parallel = free.parallel.unwrap_or_else(|| {
        let by_threads = (threads / 4).clamp(1, 4);
        let by_memory = available_memory()
            .map(|bytes| (bytes / PROBE_MEMORY_BYTES).max(1) as usize)
            .unwrap_or(by_threads);
        by_threads.min(by_memory)
    });
    let feasible: Vec<f64> = ladder
        .iter()
        .copied()
        .filter(|size| variants.contains_key(&format_mm(*size)))
        .collect();
    let mut search = KiCadFreeDimensionSearch::new(
        feasible,
        infeasible,
        probe_seconds,
        confirm_seconds,
        total,
        parallel,
    );
    println!(
        "free dimension: the {} edge, {} sizes from {} to {} mm ({} infeasible), probes at {:.0} s of work, {} side by side, the full run at {:.0} s, {:.0} s in all; accept {accept}",
        free.edge,
        ladder.len(),
        format_mm(ladder[0]),
        format_mm(ladder[ladder.len() - 1]),
        search.runs.len(),
        probe_seconds,
        parallel,
        confirm_seconds,
        total
    );
    for run in &search.runs {
        println!("free dimension: {}", run.summary());
    }
    let router_argument = match run.router_argument {
        None | Some("auto") => "auto".to_string(),
        Some(path) => path_text(Path::new(path)),
    };
    let executable = std::env::current_exe()
        .map_err(|error| Failure::Internal(format!("the running binary: {error}")))?;
    loop {
        let jobs: Vec<Job> = match search.next_step() {
            KiCadFreeDimensionStep::Done => break,
            KiCadFreeDimensionStep::Probe { sizes, budget } => {
                let count = sizes.len();
                sizes
                    .into_iter()
                    .map(|size| Job {
                        size,
                        kind: "probe",
                        budget,
                        threads: (threads / count).max(1),
                    })
                    .collect()
            }
            KiCadFreeDimensionStep::Full { size, budget, kind } => vec![Job {
                size,
                kind,
                budget,
                threads,
            }],
        };
        let mut children = Vec::new();
        for job in &jobs {
            let variant = &variants[&format_mm(job.size)];
            let directory = runs_directory.join(format!("{}mm-{}", format_mm(job.size), job.kind));
            fs::create_dir_all(&directory).map_err(|error| {
                Failure::Internal(format!("failed to create {}: {error}", directory.display()))
            })?;
            let input = directory.join("input");
            write_probe_project(source, run.board_id, &input, &variant.board)
                .map_err(Failure::Internal)?;
            // The layout config at this size: the given one without the
            // search, the run's budget, the constraints at this size.
            let mut config = run.layout_json.clone();
            if let Some(map) = config.as_object_mut() {
                map.remove("free_dimension");
                map.insert("total_seconds".into(), serde_json::json!(job.budget));
                if let Some(constraints) = &variant.constraints {
                    let placer = map.entry("placer").or_insert_with(|| serde_json::json!({}));
                    placer["constraints"] = serde_json::to_value(constraints)
                        .map_err(|error| Failure::Internal(error.to_string()))?;
                }
            }
            let config_path = directory.join("layout.json");
            write_value(&config_path, &config).map_err(Failure::Internal)?;
            let layout_directory = directory.join("layout");
            let mut process = Command::new(&executable);
            process
                .arg("layout-kicad-board")
                .arg(&input)
                .arg(run.board_id)
                .arg(&layout_directory)
                .arg(&router_argument)
                .arg(&config_path);
            if let Some(reference) = run.reference {
                process.arg("--reference").arg(reference);
            }
            process.env("RAYON_NUM_THREADS", job.threads.to_string());
            if std::env::var_os(pcb_kicad::WORK_RATE_VARIABLE).is_none() {
                process.env(pcb_kicad::WORK_RATE_VARIABLE, &pinned_rate);
            }
            let child = spawn_probe(&mut process, &directory.join("layout.log"))
                .map_err(Failure::Internal)?;
            children.push((child, layout_directory, Instant::now()));
        }
        let mut finished = Vec::new();
        for (job, (mut child, layout_directory, begun)) in jobs.iter().zip(children) {
            let status = child
                .wait()
                .map_err(|error| Failure::Internal(format!("waiting for a probe: {error}")))?;
            let verdict = read_value(&layout_directory.join("verdict.json"));
            let mut outcome = KiCadFreeDimensionRun::from_verdict(
                job.size,
                job.kind,
                job.budget,
                verdict.as_ref(),
                &accept,
            );
            if verdict.is_none() {
                outcome.wall_seconds = begun.elapsed().as_secs_f64();
                outcome
                    .reasons
                    .push(format!("the layout process ended with {status}"));
            }
            outcome.directory = path_text(&layout_directory);
            println!("free dimension: {}", outcome.summary());
            finished.push(outcome);
        }
        search.record(finished);
    }
    let Some((chosen, reason)) = search.choose(free) else {
        return Err(Failure::Internal(
            "free_dimension: no layout of the search left a verdict (see free-dimension/*/layout.log)"
                .into(),
        ));
    };
    // The chosen run's layout becomes the run's result.
    let chosen_directory = PathBuf::from(&search.runs[chosen].directory);
    let entries = fs::read_dir(&chosen_directory).map_err(|error| {
        Failure::Internal(format!(
            "failed to read {}: {error}",
            chosen_directory.display()
        ))
    })?;
    for entry in entries.flatten() {
        let target = output.join(entry.file_name());
        fs::rename(entry.path(), &target).map_err(|error| {
            Failure::Internal(format!(
                "failed to move {} to {}: {error}",
                entry.path().display(),
                target.display()
            ))
        })?;
    }
    let from = path_text(&chosen_directory);
    let to = path_text(&output);
    search.runs[chosen].directory = to.clone();
    let chosen_run = search.runs[chosen].clone();
    let work_seconds = search.spent();
    let wall_seconds = started.elapsed().as_secs_f64();
    let clean = chosen_run.clean;
    let lowest = ladder[0];
    let highest = ladder[ladder.len() - 1];
    let summary = serde_json::json!({
        "chosen": chosen_run.size_mm,
        "kind": chosen_run.kind,
        "clean": clean,
        "probes": search.runs.iter().filter(|run| run.kind != "infeasible").count(),
        "reason": reason,
        "edge": free.edge,
        "work_seconds": work_seconds,
        "wall_seconds": wall_seconds,
        "report": path_text(&output.join("free-dimension.json")),
    });
    let table = serde_json::json!({
        "edge": free.edge,
        "input_size_mm": plan.extent_mm,
        "band_mm": free.band(),
        "ladder": ladder,
        "prefer_below": free.prefer_below,
        "accept": accept,
        "total_seconds": total,
        "probe_seconds": probe_seconds,
        "confirm_seconds": confirm_seconds,
        "parallel": parallel,
        "threads": threads,
        "work_rate": pinned_rate,
        "chosen_mm": chosen_run.size_mm,
        "clean": clean,
        "reason": reason,
        "work_seconds": work_seconds,
        "wall_seconds": wall_seconds,
        "untried": search.untried(),
        "runs": search.runs,
    });
    write_value(&output.join("free-dimension.json"), &table).map_err(Failure::Internal)?;
    // The verdict and the reports: the chosen run's, at their new place.
    let verdict_path = output.join("verdict.json");
    let mut verdict = read_value(&verdict_path)
        .ok_or_else(|| Failure::Internal(format!("{} is missing", verdict_path.display())))?;
    rewrite_paths(&mut verdict, &from, &to);
    verdict["free_dimension"] = summary;
    if !clean {
        let action = format!(
            "no size from {} to {} mm was clean: {} mm came closest ({}); the next actions below are that layout's. Widen the range (max), give the probes more work (probe_seconds, total_seconds) or the board more layers",
            format_mm(lowest),
            format_mm(highest),
            format_mm(chosen_run.size_mm),
            verdict["reason"].as_str().unwrap_or("")
        );
        if let Some(actions) = verdict["next_actions"].as_array_mut() {
            actions.insert(0, serde_json::Value::String(action));
        } else {
            verdict["next_actions"] = serde_json::json!([action]);
        }
    }
    write_value(&verdict_path, &verdict).map_err(Failure::Internal)?;
    for report in [
        output.join("board-layout.json"),
        output.join("placed/board-placer.json"),
    ] {
        if let Some(mut value) = read_value(&report) {
            rewrite_paths(&mut value, &from, &to);
            if value.get("verdict").is_some() && report.ends_with("board-layout.json") {
                value["verdict"] = verdict.clone();
            }
            write_value(&report, &value).map_err(Failure::Internal)?;
        }
    }
    println!(
        "free dimension: chosen {} mm ({}): {reason}; {} runs, {:.0} s of work, {:.0} s wall; table {}",
        format_mm(chosen_run.size_mm),
        chosen_run.kind,
        search
            .runs
            .iter()
            .filter(|run| run.kind != "infeasible")
            .count(),
        work_seconds,
        wall_seconds,
        path_text(&output.join("free-dimension.json"))
    );
    let status = status_of(verdict["status"].as_str().unwrap_or(""));
    let exit = status.exit_code();
    DECIDED.store(i32::from(exit), std::sync::atomic::Ordering::Relaxed);
    println!(
        "verdict: {} (exit {exit}): {}",
        verdict["status"].as_str().unwrap_or(""),
        verdict["reason"].as_str().unwrap_or("")
    );
    for action in verdict["next_actions"].as_array().into_iter().flatten() {
        println!("next: {}", action.as_str().unwrap_or(""));
    }
    if status != KiCadRunStatus::Complete {
        eprintln!(
            "{}; see {}",
            verdict["reason"].as_str().unwrap_or(""),
            verdict_path.display()
        );
    }
    Ok(status)
}
