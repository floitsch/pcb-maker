// Copyright (C) 2026 Toit contributors.

//! The coarse probe (`pcb-maker probe-coarse-placement`, a research
//! command; docs/placer.md, "A coarse whole first route as the race probe
//! (2026-10-27)"): a placed board routed whole, as the layout's first
//! route routes it, at a reduced fidelity: a coarser lattice (the router's
//! candidate pitches times a factor), fewer negotiation iterations, the
//! hard repair and the trouble repair off or on, all of it within a work
//! budget that is a share of the task budget. Its open terminals rank the
//! placement. Deterministic: the budget is work.

use super::*;
use crate::global_route::lower_placed;
use pcb_router as core;

#[derive(Clone, Debug)]
pub struct KiCadCoarseProbeConfig {
    /// The router's candidate lattice pitches (0.127, 0.125 and 0.1 mm)
    /// are multiplied by it.
    pub pitch_factor: f64,
    /// Negotiation iterations at most (`None`: the router's 80).
    pub iterations: Option<usize>,
    /// The first route's hard repair after the negotiation (off: the nets
    /// in conflict lose their conflicted branches and the open terminals
    /// are counted then).
    pub hard_repair: bool,
    /// The trouble repair after the hard repair (with `hard_repair`).
    pub trouble_repair: bool,
    /// The negotiation's part of the budget with `hard_repair` (the rest
    /// is the repair's).
    pub negotiation_share: f64,
    /// The work budget: `share` of `total_seconds`, or `work_seconds`.
    pub share: f64,
    pub total_seconds: f64,
    pub work_seconds: Option<f64>,
}

impl Default for KiCadCoarseProbeConfig {
    fn default() -> Self {
        Self {
            pitch_factor: 1.0,
            iterations: None,
            hard_repair: false,
            trouble_repair: false,
            negotiation_share: 0.5,
            share: 0.05,
            total_seconds: 1500.0,
            work_seconds: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadCoarseProbeReport {
    pub board_id: String,
    pub pitch_factor: f64,
    /// The lattice pitch the router chose (mm) and its nodes per layer.
    pub pitch_mm: f64,
    pub lattice_nodes: usize,
    pub max_iterations: usize,
    pub hard_repair: bool,
    pub trouble_repair: bool,
    pub budget_seconds: f64,
    /// The negotiation's iterations and why it ended.
    pub iterations: usize,
    pub stop: String,
    /// When the negotiation ended (`Router::probe_standing`): the open
    /// terminals plus one per complete net in conflict and every pour pad
    /// off its pour; nets unfinished; nets in conflict.
    pub negotiated_open_terminals: usize,
    pub unfinished: usize,
    pub conflicted: usize,
    /// After the conflicted nets lost their conflicted branches (the hard
    /// repair's rip-up, without its searches): the nets ripped and the
    /// board's open terminals (with the pour pads off their pour).
    pub ripped: usize,
    pub ripped_open_terminals: usize,
    /// After the hard repair (and the trouble repair where on): the open
    /// terminals as the layout counts its first route's (statuses plus
    /// the verifier's findings). `None` without `hard_repair`.
    pub repaired_open_terminals: Option<usize>,
    /// The ranked measure: `repaired_open_terminals` where the repair ran,
    /// else `ripped_open_terminals`.
    pub open_terminals: usize,
    /// Work (s, at the pinned rates): the negotiation's search, the
    /// repairs' search, the router's build, the pour analyses, all of it.
    pub search_work_seconds: f64,
    pub repair_work_seconds: f64,
    pub build_work_seconds: f64,
    pub pour_work_seconds: f64,
    pub work_seconds: f64,
    pub wall_seconds: f64,
}

/// Routes the placed board `board_path` (a `.kicad_pcb`; tracks are
/// stripped, planned pours added) whole at the reduced fidelity of
/// `config`.
pub fn probe_coarse_placement(
    board_path: &Path,
    router_config: &KiCadBoardRouterConfig,
    config: &KiCadCoarseProbeConfig,
) -> Result<KiCadCoarseProbeReport, String> {
    let started = Instant::now();
    let board_id = board_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("{}: no board id", board_path.display()))?
        .to_string();
    if !(config.pitch_factor.is_finite() && config.pitch_factor > 0.0) {
        return Err(format!(
            "--pitch-factor: a positive number, not {}",
            config.pitch_factor
        ));
    }
    let text = fs::read_to_string(board_path)
        .map_err(|error| format!("failed to read {}: {error}", board_path.display()))?;
    let pcb = crate::canonical_order::parse_board(&text)?;
    let board = lower_placed(&pcb, router_config)?;
    // The race's and the first route's settings (`race_core_config`), at
    // the reduced fidelity.
    let mut core_config = crate::congestion_samples::race_core_config(router_config);
    core_config.verbose = false;
    core_config.pitches = core_config
        .pitches
        .iter()
        .map(|pitch| pitch * config.pitch_factor)
        .collect();
    if let Some(iterations) = config.iterations {
        core_config.max_iterations = iterations.max(1);
    }
    core_config.trouble_repair = config.hard_repair && config.trouble_repair;
    let budget = config
        .work_seconds
        .unwrap_or(config.share * config.total_seconds)
        .max(0.0);
    // The whole probe's search within the budget (the router's own work,
    // its build and pour analyses aside, as the race's probes count).
    core_config.deadline_work = Some(budget);
    let negotiation = if config.hard_repair {
        budget * config.negotiation_share.clamp(0.0, 1.0)
    } else {
        budget
    };
    let mut router = core::router::Router::new(&board, &core_config);
    let grid = router.grid().clone();
    let rate = core::router::expansions_per_second();
    let before = router.expansions();
    router.probe(negotiation);
    let negotiated = router.expansions();
    let (iterations, stop) = router.negotiation_progress();
    let standing = router.probe_standing();
    let (ripped, after) = router.rip_up_standing();
    let repaired = config.hard_repair.then(|| {
        let result = router.finish_probe();
        result
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
            + core::verify(&board, &result.routes).len()
    });
    let search = negotiated.saturating_sub(before) as f64 / rate;
    let repair = router.expansions().saturating_sub(negotiated) as f64 / rate;
    let build = router.build_work();
    let pour = router.pour_work();
    Ok(KiCadCoarseProbeReport {
        board_id,
        pitch_factor: config.pitch_factor,
        pitch_mm: grid.pitch,
        lattice_nodes: grid.nx * grid.ny,
        max_iterations: core_config.max_iterations,
        hard_repair: config.hard_repair,
        trouble_repair: core_config.trouble_repair,
        budget_seconds: budget,
        iterations,
        stop: stop.to_string(),
        negotiated_open_terminals: standing.open_terminals,
        unfinished: standing.unfinished,
        conflicted: standing.conflicted,
        ripped,
        ripped_open_terminals: after.open_terminals,
        repaired_open_terminals: repaired,
        open_terminals: repaired.unwrap_or(after.open_terminals),
        search_work_seconds: search,
        repair_work_seconds: repair,
        build_work_seconds: build,
        pour_work_seconds: pour,
        work_seconds: search + repair + build + pour,
        wall_seconds: started.elapsed().as_secs_f64(),
    })
}
