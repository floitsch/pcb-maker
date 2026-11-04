// Copyright (C) 2026 Toit contributors.

//! Pads without a way out (`pad_exits`, off by default; docs/placer.md
//! "Pads without a way out (2026-11-04)"): the placement lowered as the
//! layout routes it, and the router's test of each pad's way out on the
//! statics (`pcb_router::router::ExitProbe`) for the placer's
//! repair (`pcb_placer::exits`), the place check's report and the
//! census command (`pad-exits-kicad-board`).

use super::guidance::{lowered_at, reference_index, routed_base};
use super::*;
use crate::board_router::core_config;
use pcb_placer as core;
use pcb_router as router;
use std::sync::Arc;

/// `KiCadBoardPlacerConfig::pad_exits`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadPadExits {
    /// How far a part may move to open a pad, mm.
    pub reach_mm: f64,
    /// Candidate poses asked about per part with a closed pad, the
    /// cheapest in wirelength first.
    pub candidates: usize,
}

impl Default for KiCadPadExits {
    fn default() -> Self {
        Self {
            reach_mm: 1.0,
            candidates: 16,
        }
    }
}

/// `PCB_PLACER_PAD_EXITS` when set: overrides `pad_exits` (`0` or empty
/// off, anything else on, with the default settings where the config has
/// none).
pub fn pad_exits_override() -> Option<bool> {
    let value = std::env::var("PCB_PLACER_PAD_EXITS").ok()?;
    Some(!(value.is_empty() || value == "0"))
}

/// The settings in force: the config's, overridden by `PCB_PLACER_PAD_EXITS`.
pub(crate) fn pad_exits_settings(config: &KiCadBoardPlacerConfig) -> Option<KiCadPadExits> {
    match pad_exits_override() {
        Some(true) => Some(config.pad_exits.clone().unwrap_or_default()),
        Some(false) => None,
        None => config.pad_exits.clone(),
    }
}

/// A closed pad as the reports give it.
#[derive(Clone, Debug, Serialize, serde::Deserialize, PartialEq)]
pub struct KiCadClosedPad {
    /// `REF.PAD`, its net and copper layer.
    pub pad: String,
    pub net: String,
    pub layer: String,
    /// What closes its ways (other parts' items first).
    pub blocked_by: Vec<String>,
    /// Only its own part closes it: the footprint's doing, no placement
    /// opens it.
    pub footprint_only: bool,
    /// Why, in a sentence (`pad X has no way out (blocked by Y)`).
    pub reason: String,
}

/// A part the repair moved or turned.
#[derive(Clone, Debug, Serialize, serde::Deserialize, PartialEq)]
pub struct KiCadExitMove {
    pub part: String,
    /// x, y (mm) and angle (degrees), before and after.
    pub from: [f64; 3],
    pub to: [f64; 3],
    pub opened: Vec<String>,
}

/// `board-placer.json`'s `pad_exits`: the kept placement's.
#[derive(Clone, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct KiCadPadExitsReport {
    /// Closed pads before the repair, and those left.
    pub closed_before: usize,
    pub closed: Vec<KiCadClosedPad>,
    pub moves: Vec<KiCadExitMove>,
    /// Candidate poses asked about.
    pub tried: usize,
    /// The test's and the repair's work, seconds of the reference machine.
    pub work_seconds: f64,
}

/// Reference nanoseconds a lowering for the test takes per expression of
/// the board (`board_router::expression_count`): the census's lowerings of
/// the night's placements of 2026-11-04 after the process's first (whose
/// one-time costs other lowerings of the layout pay), 80 to 270 ns on the
/// pour boards, beside the router's 26 ns model of route mode's lowerings.
const LOWERING_EXPRESSION_NANOS: f64 = 100.0;

/// A session's lowering of `base`, in placement units.
fn lowering_units(base: &Expr) -> u64 {
    (crate::board_router::expression_count(base) as f64 * LOWERING_EXPRESSION_NANOS / 1.0e9
        * pcb_router::router::reference_time_scale()
        * core::work_per_second()) as u64
}

/// The sentence of a closed pad.
pub(crate) fn closed_pad_reason(pad: &str, blocked_by: &[String], footprint_only: bool) -> String {
    format!(
        "pad {pad} has no way out (blocked by {}){}",
        blocked_by.join(", "),
        if footprint_only {
            ": its own footprint walls it in, no placement opens it"
        } else {
            ""
        }
    )
}

fn closed_pad(pad: &router::router::ClosedPad, layer_names: &[String]) -> KiCadClosedPad {
    let blocked_by = pad.blocked_by();
    let footprint_only = pad.own_only();
    KiCadClosedPad {
        reason: closed_pad_reason(&pad.label, &blocked_by, footprint_only),
        pad: pad.label.clone(),
        net: pad.net.clone(),
        layer: layer_names
            .get(pad.layer)
            .cloned()
            .unwrap_or_else(|| pad.layer.to_string()),
        blocked_by,
        footprint_only,
    }
}

/// The copper layers' names by the router's index.
fn layer_names(pcb: &Expr) -> Vec<String> {
    crate::board_router::LayerTable::from_pcb(pcb)
        .map(|table| table.names)
        .unwrap_or_default()
}

/// The placer's test (`core::exits::ExitFactory`): each session lowers
/// the first placement it is asked about as the layout routes it (pours
/// added, copper texts as the layout keeps them) and carries the parts
/// that move since into that board rigidly (`move_descent::apply`).
pub(super) fn exit_factory(
    pcb: &Expr,
    references: &[String],
    router_config: &KiCadBoardRouterConfig,
    keep_board_marks: bool,
) -> Result<Arc<core::exits::ExitFactory>, String> {
    let (base, connect) = routed_base(pcb, router_config, keep_board_marks)?;
    let lowering = lowering_units(&base);
    let layers = Arc::new(layer_names(&base));
    let base = Arc::new(base);
    let index = Arc::new(reference_index(references));
    let references = Arc::new(references.to_vec());
    let router_config = router_config.clone();
    let core_config = Arc::new(core_config(&router_config));
    Ok(Arc::new(move || -> core::exits::ExitSession {
        let (base, index, references, layers) = (
            base.clone(),
            index.clone(),
            references.clone(),
            layers.clone(),
        );
        let (router_config, core_config) = (router_config.clone(), core_config.clone());
        let mut state: Option<(
            router::router::ExitProbe,
            Vec<crate::move_descent::PartCopper>,
            Vec<core::Pose>,
        )> = None;
        Box::new(move |poses: &[core::Pose], parts: &[usize]| {
            let mut work = 0u64;
            match state.as_mut() {
                None => {
                    let board = lowered_at(&base, poses, &router_config, connect)?;
                    work += lowering;
                    let probe = router::router::ExitProbe::new(&board, &core_config);
                    let copper =
                        crate::move_descent::part_copper(probe.board(), &index, references.len());
                    state = Some((probe, copper, poses.to_vec()));
                }
                Some((probe, copper, last)) => {
                    if last.len() != poses.len() {
                        return None;
                    }
                    let moved: Vec<usize> = (0..references.len().min(poses.len()))
                        .filter(|part| last[*part] != poses[*part])
                        .collect();
                    if !moved.is_empty() {
                        crate::move_descent::apply(probe.board_mut(), copper, last, poses, &moved);
                        last.copy_from_slice(poses);
                    }
                }
            }
            let (probe, _, _) = state.as_mut()?;
            let mut closed = Vec::new();
            for &part in parts {
                let Some(reference) = references.get(part) else {
                    continue;
                };
                for pad in probe.closed_pads(reference) {
                    let mut blockers: Vec<usize> = pad
                        .blockers
                        .iter()
                        .filter_map(|blocker| {
                            index
                                .get(router::router::label_part(&blocker.label))
                                .copied()
                        })
                        .filter(|blocker| *blocker != part)
                        .collect();
                    blockers.sort_unstable();
                    blockers.dedup();
                    let report = closed_pad(&pad, &layers);
                    closed.push(core::exits::ClosedPad {
                        part,
                        pad: report.pad,
                        net: report.net,
                        layer: pad.layer,
                        blocked_by: report.blocked_by,
                        blockers,
                        own_only: report.footprint_only,
                    });
                }
            }
            work += (probe.take_work_seconds() * core::work_per_second()) as u64;
            Some((closed, work))
        })
    }))
}

/// The kept placement's report from the placer's outcome.
pub(super) fn exits_report(
    outcome: &core::exits::ExitOutcome,
    references: &[String],
    pcb: &Expr,
) -> KiCadPadExitsReport {
    let layers = layer_names(pcb);
    let name = |part: usize| {
        references
            .get(part)
            .cloned()
            .unwrap_or_else(|| part.to_string())
    };
    KiCadPadExitsReport {
        closed_before: outcome.closed_before,
        closed: outcome
            .closed
            .iter()
            .map(|pad| KiCadClosedPad {
                reason: closed_pad_reason(&pad.pad, &pad.blocked_by, pad.own_only),
                pad: pad.pad.clone(),
                net: pad.net.clone(),
                layer: layers
                    .get(pad.layer)
                    .cloned()
                    .unwrap_or_else(|| pad.layer.to_string()),
                blocked_by: pad.blocked_by.clone(),
                footprint_only: pad.own_only,
            })
            .collect(),
        moves: outcome
            .moves
            .iter()
            .map(|moved| KiCadExitMove {
                part: name(moved.part),
                from: [
                    moved.from.position[0],
                    moved.from.position[1],
                    moved.from.angle,
                ],
                to: [moved.to.position[0], moved.to.position[1], moved.to.angle],
                opened: moved.opened.clone(),
            })
            .collect(),
        tried: outcome.tried,
        work_seconds: outcome.work as f64 / core::work_per_second(),
    }
}

/// `pad-exits-kicad-board`'s report: every pad of the board as placed
/// with no way out on the statics.
#[derive(Clone, Debug, Serialize)]
pub struct KiCadPadExitsCensus {
    pub board_id: String,
    pub parts: usize,
    pub closed: Vec<KiCadClosedPad>,
    /// The test's work (seconds of the reference machine: the lowering at
    /// the placer's rate, the probe's maps and pockets), and the CPU time
    /// of each (the thread's, the wall clock where that is not known).
    pub work_seconds: f64,
    pub lower_work_seconds: f64,
    pub probe_work_seconds: f64,
    pub lower_seconds: f64,
    pub probe_seconds: f64,
    /// With `--repair`: the placer's repair of the board as placed, at the
    /// loosest relaxation level where every movable part is legal there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repaired: Option<KiCadPadExitsReport>,
}

/// The placer's repair (`core::exits::repair`) of the board as placed: a
/// diagnostic of the repair on a placement (the night's), without placing
/// again.
fn repair_as_placed(
    source_directory: &Path,
    board_id: &str,
    router_config: &KiCadBoardRouterConfig,
    settings: &KiCadPadExits,
) -> Result<KiCadPadExitsReport, String> {
    let mut config = KiCadBoardPlacerConfig::default();
    if source_directory.join("constraints.json").exists() {
        config.constraints = Some(KiCadConstraintsSource::File("constraints.json".into()));
    }
    let prepared = prepare_board(source_directory, board_id, &config, None)?;
    let (pcb, config, lowered) = (&prepared.pcb, &prepared.config, &prepared.lowered);
    let mut placer_config = core::Config::new();
    placer_config.maximum_utilization = config.maximum_utilization;
    placer_config.overlap_far_side_pads = config.overlap_far_side_pads;
    placer_config.kicad_legality = config.legality == KiCadPlacerLegality::Kicad;
    placer_config.kicad_last_level = config.legality != KiCadPlacerLegality::Strict;
    let poses = lowered.problem.poses.clone();
    let levels = core::level_problems(&lowered.problem, &placer_config);
    let level = levels
        .iter()
        .find(|(_, problem)| core::legal::illegal_components(problem, &poses).is_empty())
        .or(levels.last())
        .map(|(_, problem)| problem)
        .ok_or("no relaxation level")?;
    let exits = core::exits::PadExits {
        factory: exit_factory(
            pcb,
            &lowered.references,
            router_config,
            config.keep_board_marks,
        )?,
        reach: settings.reach_mm,
        candidates: settings.candidates,
    };
    let mut poses = poses;
    let before = core::work();
    let mut outcome = core::exits::repair(level, &mut poses, &[], &exits)
        .ok_or("the pad-exits test could not lower the board")?;
    outcome.work = core::work() - before;
    Ok(exits_report(&outcome, &lowered.references, pcb))
}

/// The census of a board as it is placed (its tracks and vias left out):
/// the pads of every part with no way out on the statics.
pub fn pad_exits_kicad_board(
    source_directory: &Path,
    board_id: &str,
    router_config: &KiCadBoardRouterConfig,
    repair: Option<&KiCadPadExits>,
) -> Result<KiCadPadExitsCensus, String> {
    let path = source_directory.join(format!("{board_id}.kicad_pcb"));
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let pcb = crate::canonical_order::parse_board(&text)?;
    let router_config = if router_config.net_classes.is_empty() {
        router_config.clone()
    } else {
        crate::net_classes::with_net_classes(router_config, &pcb)?
    };
    let based = thread_seconds();
    let (base, connect) = routed_base(&pcb, &router_config, true)?;
    let base_seconds = thread_seconds() - based;
    let started = thread_seconds();
    let board = crate::board_router::lower(&base, &router_config, connect)?.board;
    let lower_seconds = thread_seconds() - started;
    let probed = thread_seconds();
    let mut probe = router::router::ExitProbe::new(&board, &core_config(&router_config));
    let new_seconds = thread_seconds() - probed;
    let layers = layer_names(&base);
    let parts = probe.parts();
    let mut closed = Vec::new();
    for part in &parts {
        for pad in probe.closed_pads(part) {
            closed.push(closed_pad(&pad, &layers));
        }
    }
    let probe_seconds = thread_seconds() - probed;
    if std::env::var_os("PCB_PAD_EXITS_COUNTS").is_some() {
        eprintln!(
            "pad exits counts: {:?} lowering_expressions {} obstacles {} cpu_new {new_seconds:.4} cpu_probe {probe_seconds:.4} cpu_lower {lower_seconds:.4} cpu_base {base_seconds:.4}",
            probe.counts(),
            crate::board_router::expression_count(&base),
            board.obstacles.len(),
        );
    }
    let probe_work_seconds = probe.take_work_seconds();
    let lower_work_seconds = lowering_units(&base) as f64 / core::work_per_second();
    let work_seconds = probe_work_seconds + lower_work_seconds;
    let repaired = match repair {
        Some(settings) => Some(repair_as_placed(
            source_directory,
            board_id,
            &router_config,
            settings,
        )?),
        None => None,
    };
    Ok(KiCadPadExitsCensus {
        board_id: board_id.to_string(),
        parts: parts.len(),
        closed,
        work_seconds,
        lower_work_seconds,
        probe_work_seconds,
        lower_seconds,
        probe_seconds,
        repaired,
    })
}

/// The CPU time this thread has run (Linux: `/proc/thread-self/schedstat`),
/// else the time since the first call.
fn thread_seconds() -> f64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    std::fs::read_to_string("/proc/thread-self/schedstat")
        .ok()
        .and_then(|text| text.split_whitespace().next()?.parse::<f64>().ok())
        .map(|nanoseconds| nanoseconds * 1.0e-9)
        .unwrap_or_else(|| {
            START
                .get_or_init(std::time::Instant::now)
                .elapsed()
                .as_secs_f64()
        })
}
