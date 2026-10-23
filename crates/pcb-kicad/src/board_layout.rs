// Copyright (C) 2026 Toit contributors.

//! Coupled placement and routing.
//!
//! The board is placed, then routed once in full. From then on the router
//! and placer take turns on the same in-memory state: the router's
//! congestion history names the footprints sitting where nets fought for
//! room, the placer proposes small legal moves for them, and the router
//! reroutes only the nets the move invalidated. A move is kept when the
//! board improves (open connections first, then vias and copper). The final
//! board is verified natively once.

use super::*;
use crate::board_placer::{largest_clearance, lower_placement, write_footprint_pose};
use crate::board_router::{
    LayerTable, add_pour_zones, core_config, emit_routes, finish_routed_board, lower,
    planned_pours, pours,
};
use crate::pin_swap::{KiCadPinSwapComponent, KiCadPinSwapGroup};
use crate::placement_constraints::{
    KiCadConstraintStatus, constraint_report, default_area_factor, resolve_constraints,
};
use pcb_placer as placer;
use pcb_router as core;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadBoardLayoutConfig {
    /// Footprint moves to try after the first full route.
    pub moves: usize,
    /// Stop after this many consecutive moves without improvement.
    pub patience: usize,
    /// Move distances tried, in millimetres, in the four axis directions.
    pub steps_mm: Vec<f64>,
    /// Budget of the move phase, in seconds of work (search expansions
    /// and placement units at the run's work rates; docs/router.md,
    /// "Determinism and the wall clock").
    pub move_seconds: f64,
    /// Once every connection is routed, moves only polish (fewer vias,
    /// less copper): they stop after this many seconds of the move phase.
    pub polish_seconds: f64,
    /// Budget of the whole layout, in seconds of work (as `move_seconds`;
    /// the wall clock only guards it: `wall_guard`): the placement race
    /// stops at 40 % of it, the moves at 65 %, and the final ladder gets
    /// what is left. Every route stops by it (then only the native check
    /// runs). An agent waits for this; the best board so far is what it
    /// gets.
    pub total_seconds: f64,
    /// How far past `total_seconds` (counted in work) the wall clock may
    /// run before the layout's phases are cut short, as a factor of it
    /// (default 2: the layout stops at `total_seconds` of work; the wall
    /// clock only guards against a machine far busier or slower than the
    /// work rates say: `wall_guard_seconds`). A load margin, not a budget.
    pub wall_guard: f64,
    /// The settings of the layout's placement (as place-kicad-board's
    /// placer config; its constraints relative to the source directory).
    pub placer: KiCadBoardPlacerConfig,
    /// Interchangeable pins (`pin-swaps.json`; relative to the source
    /// directory). The nets on them are permuted after placement.
    pub pin_swaps: Option<PathBuf>,
    /// Interchangeable pins, the short way: per part, the pins (pad
    /// functions such as `GPIO*` or pad numbers, globs) whose nets may be
    /// permuted. Used when `pin_swaps` is not given.
    pub swappable: Vec<KiCadSwappable>,
    /// The pin-swap estimate's settings, for `pin_swaps` and `swappable`.
    pub pin_swap: KiCadPinSwapConfig,
    /// The placement race's pre-ranking (`docs/congestion-model.md`): the
    /// placer makes `congestion_candidates` placements (seeds), they are
    /// ranked by `congestion_score`, and the race probes only the best
    /// `congestion_probes`. `congestion_score`:
    /// - `rudy` (default): RUDY routing demand above the tiles' free
    ///   capacity; no model needed (quick tier: KiCad unconnected 98 -> 64
    ///   against the plain race of three seeds);
    /// - `none`: the plain race (the placer's `placement_seeds`, all
    ///   probed);
    /// - `global`: the coarse global route's usage above half the tile
    ///   boundaries' capacity (`pcb-global`, docs/global-route.md; an
    ///   experiment);
    /// - with `congestion_model` (an ONNX file, an experiment): `open`
    ///   (its predicted unfinished nets), `overflow` (its predicted
    ///   overflow summed over the board), or `open+rudy` (the sum of a
    ///   placement's ranks by `open` and by RUDY).
    pub congestion_model: Option<PathBuf>,
    /// With a pre-ranking score, the placer makes at least this many
    /// placements to rank (its `placement_seeds` when that is more).
    pub congestion_candidates: usize,
    /// With a pre-ranking score, how many of the best-ranked placements
    /// the race routes (at least one).
    pub congestion_probes: usize,
    /// How the race ranks the placements before probing them (the values
    /// above, at `congestion_model`).
    pub congestion_score: String,
    /// One edge of a rectangular outline the layout may move, to find the
    /// smallest board that comes out clean (docs/constraints.md, "A free
    /// dimension"): `layout-kicad-board` then lays the board out at
    /// several sizes, `total_seconds` being the whole search's budget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_dimension: Option<crate::free_dimension::KiCadFreeDimension>,
    /// The footprints' Reference and Value fields on the silkscreen, placed
    /// after routing where KiCad's silkscreen checks find nothing.
    pub silkscreen: KiCadSilkscreenConfig,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadSilkscreenConfig {
    /// Hide a field no spot around (or inside) its part is clean for
    /// (default false: it stays at the spot with the fewest findings, and
    /// the verdict's `silkscreen` fact lists it).
    pub hide_when_blocked: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadSwappable {
    /// The part's reference (`U1`).
    pub part: String,
    /// Pad functions (`GPIO*`, `IO?`) or pad numbers, as globs.
    pub pins: Vec<String>,
    /// Pads to leave alone (strapping pins, a UART the bootloader uses).
    #[serde(default)]
    pub except: Vec<String>,
}

/// A pin-swap spec from the short form: one group per part, each matching
/// pad a unit of its own.
fn swappable_spec(pcb: &Expr, swappable: &[KiCadSwappable]) -> Result<KiCadPinSwapSpec, String> {
    let mut components = BTreeMap::new();
    for entry in swappable {
        let footprint = pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
            .find(|footprint| {
                footprint_reference(footprint).as_deref() == Some(entry.part.as_str())
            })
            .ok_or_else(|| format!("swappable: no part {:?}", entry.part))?;
        let mut units = Vec::new();
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            let number = pad.children().get(1).and_then(Expr::atom).unwrap_or("");
            let function = form_atom(pad, "pinfunction", 1).unwrap_or("");
            let matches = |patterns: &[String]| {
                patterns.iter().any(|pattern| {
                    board_placer::glob_matches(pattern, number)
                        || board_placer::glob_matches(pattern, function)
                })
            };
            if matches(&entry.pins)
                && !matches(&entry.except)
                && !units.iter().any(|unit: &Vec<String>| unit[0] == number)
            {
                units.push(vec![number.to_string()]);
            }
        }
        if units.len() < 2 {
            return Err(format!(
                "swappable {}: fewer than two pins match {:?}",
                entry.part, entry.pins
            ));
        }
        components.insert(
            entry.part.clone(),
            KiCadPinSwapComponent {
                groups: vec![KiCadPinSwapGroup {
                    name: "swappable".into(),
                    units,
                    restrictions: BTreeMap::new(),
                    cross_component: false,
                }],
            },
        );
    }
    Ok(KiCadPinSwapSpec {
        version: 1,
        components,
    })
}

impl Default for KiCadBoardLayoutConfig {
    fn default() -> Self {
        Self {
            moves: 40,
            patience: 12,
            steps_mm: vec![0.5, 1.0, 2.0, 4.0],
            move_seconds: 600.0,
            polish_seconds: 60.0,
            total_seconds: 1500.0,
            wall_guard: DEFAULT_WALL_GUARD,
            placer: KiCadBoardPlacerConfig::default(),
            pin_swaps: None,
            swappable: Vec::new(),
            pin_swap: KiCadPinSwapConfig::default(),
            congestion_model: None,
            congestion_candidates: 16,
            congestion_probes: 3,
            congestion_score: "rudy".into(),
            free_dimension: None,
            silkscreen: KiCadSilkscreenConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadBoardLayoutMove {
    pub reference: String,
    pub from: [f64; 3],
    pub to: [f64; 3],
    pub rerouted_nets: usize,
    pub seconds: f64,
    pub kept: bool,
    pub unconnected_terminals: usize,
    pub vias: usize,
    pub length_mm: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadBoardLayoutResult {
    pub board_id: String,
    /// The work up to the final ladder (sizing, placement, routes and
    /// moves, KiCad's check of the board kept), in seconds of an idle
    /// machine: what the budget counts.
    pub work_seconds: f64,
    /// The work one KiCad check counts (`check_work`, modelled from the
    /// board), the checks counted (the board kept, the ladders' attempts,
    /// the neck repairs) and their work, part of `work_seconds` and
    /// `routed.work_seconds`.
    pub kicad_check_seconds: f64,
    pub kicad_checks_charged: usize,
    pub kicad_check_work_seconds: f64,
    pub placement_seconds: f64,
    pub first_route_seconds: f64,
    pub first_unconnected_terminals: usize,
    pub first_vias: usize,
    pub first_length_mm: f64,
    /// The work the final ladder (or the narrow-signal attempt standing in
    /// for it) had: `total_seconds` less the work so far and the seed
    /// pool's excess; `None` where it did not run. A route of the
    /// designer's placement at this budget gets the routing work this
    /// layout's last router got (review 2026-10-21, H3).
    pub final_ladder_budget_seconds: Option<f64>,
    pub moves: Vec<KiCadBoardLayoutMove>,
    /// Reference labels moved off pads and other silkscreen, and the
    /// board's own texts and silkscreen graphics parts lie on
    /// (`overlaps`).
    pub labels: crate::labels::KiCadLabelReport,
    /// The result's quality measures and cost estimate (as
    /// `score-kicad-board` reports them).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<crate::quality::KiCadQualityReport>,
    /// Outline sizes tried when the outline is sized from the parts,
    /// smallest first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub outline_sizing: Vec<KiCadOutlineTrial>,
    /// Nets unfinished after an equal probe of the kept placement and of
    /// each other seed's placement tried, the best routing on.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub placement_race: Vec<usize>,
    /// The race's pre-ranking: the score of every placement it ranked
    /// (the kept one first, then the other seeds by the placer's key) and
    /// the ones the race probed, best first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub congestion: Option<KiCadCongestionRanking>,
    pub pin_swaps: Option<KiCadPinSwapResult>,
    /// Every placement constraint with whether the final placement keeps it.
    pub constraints: Vec<KiCadConstraintStatus>,
    pub routed: KiCadBoardRouterResult,
    /// What the wall clock cut short (`pcb_router::router::WALL_CUTS`)
    /// rather than the work: the layout then depends on the machine's
    /// load. Empty when the work decided everything.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cut: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadCongestionRanking {
    pub scores: Vec<f64>,
    pub probed: Vec<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadOutlineTrial {
    pub area_factor: f64,
    pub size_mm: Option<[f64; 2]>,
    /// Whether every part found a legal place.
    pub legal: bool,
    /// Open terminals after a quick first route (legal placements only).
    pub open: Option<usize>,
    /// The size the layout went on with.
    pub used: bool,
}

/// How far past its total budget (counted in work) a layout may run on the
/// wall clock, on a busy or slow machine, before its phases are cut short
/// (`wall_guard`'s default; verification and writing the result follow).
/// It is a margin for load, not a budget: the work ends the layout, at
/// `total_seconds`, and since the work rates are measured at the start a
/// second of work is about a second of this machine's wall clock, so on a
/// machine as fast as its calibration the guard never fires. It was 1.1
/// with the reference's rates fixed: any machine slower than a 5950X core
/// by more than a tenth (a laptop, a CI runner, this one above load 45)
/// had every non-trivial layout cut (exit 3; 0xcb-static at 40 s pinned
/// to one CPU: 16.6 s of work at 48 s of wall). Twice the budget leaves
/// room for what the work does not count (KiCad's checks beyond their
/// model, the measured rate's drift with the load during a run) while a
/// run that hangs still ends; the harness's timeout follows it (run.py:
/// the budget times the guard plus 300 s, at least 1800 s).
pub const DEFAULT_WALL_GUARD: f64 = 2.0;

/// The wall clock a layout of `total_seconds` of work may take before its
/// phases are cut short (`wall_guard` times the budget). A guard below 1
/// would cut every layout before its work ends: it is an input error.
pub fn wall_guard_seconds(total_seconds: f64, wall_guard: f64) -> Result<f64, String> {
    if !(wall_guard.is_finite() && wall_guard >= 1.0) {
        return Err(crate::verdict::input_error(format!(
            "wall_guard {wall_guard}: the wall-clock guard is a factor of total_seconds of at least 1"
        )));
    }
    Ok(wall_guard * total_seconds.max(0.0))
}

/// For an outline sized from the parts (no size, no `area_factor`): starts
/// at the size designers use and grows it by a quarter while the parts do
/// not fit or a quick route leaves connections open; with `shrink`, when
/// the first size routes, shrinks it by a fifth while it still does. Sets
/// the chosen factor in `placer_config` and returns the sizes tried.
#[allow(
    clippy::too_many_arguments,
    reason = "the layout's work accounts (work, its deadline, the pool's excess) are threaded through by reference"
)]
fn size_outline(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    placer_config: &mut KiCadBoardPlacerConfig,
    router_config: &KiCadBoardRouterConfig,
    work: &mut f64,
    work_deadline: f64,
    pool_excess: &mut f64,
) -> Result<Vec<KiCadOutlineTrial>, String> {
    let Some(shrink) = automatic_outline(placer_config).map(|outline| outline.shrink) else {
        return Ok(Vec::new());
    };
    let board = source_directory.join(format!("{board_id}.kicad_pcb"));
    let start = default_area_factor(&crate::canonical_order::parse_board(
        &fs::read_to_string(&board)
            .map_err(|error| format!("failed to read {}: {error}", board.display()))?,
    )?);
    let mut quick = core_config(router_config);
    quick.verbose = false;
    quick.negotiation_seconds = quick.negotiation_seconds.min(60.0);
    quick.cleanup_passes = 0;
    quick.via_reduction_rounds = 0;
    let sizing = output_directory.join("sizing");
    fs::create_dir_all(&sizing)
        .map_err(|error| format!("failed to create {}: {error}", sizing.display()))?;
    let mut trials = Vec::new();
    let mut chosen = None;
    // Grow from the start; with `shrink`, once the start routes, shrink.
    let steps: Vec<f64> = (0..4)
        .map(|step| 1.25f64.powi(step))
        .chain((1..5).map(|step| 0.8f64.powi(step)))
        .collect();
    for scale in steps {
        let routed_first = trials
            .first()
            .is_some_and(|trial: &KiCadOutlineTrial| trial.open == Some(0));
        if scale < 1.0 && !(shrink && routed_first) {
            break;
        }
        if scale > 1.0 && routed_first {
            continue;
        }
        let factor = (start * scale * 100.0).round() / 100.0;
        let mut config = placer_config.clone();
        if let Some(outline) = automatic_outline(&mut config) {
            outline.area_factor = Some(factor);
        }
        let directory = sizing.join(format!("{factor}"));
        let placement = place_kicad_board(source_directory, board_id, &directory, &config)?;
        *work += placement.work_seconds;
        *pool_excess += (placement.pool_work_seconds - placement.work_seconds).max(0.0);
        let legal = placement.unplaced.is_empty() && placement.illegal.is_empty();
        let open = if legal {
            let placed = directory.join(format!("{board_id}.kicad_pcb"));
            let mut pcb = crate::canonical_order::parse_board(
                &fs::read_to_string(&placed)
                    .map_err(|error| format!("failed to read {}: {error}", placed.display()))?,
            )?;
            let planned = planned_pours(&pcb, router_config)?;
            if !planned.is_empty() {
                add_pour_zones(&mut pcb, &planned)?;
            }
            let layers = LayerTable::from_pcb(&pcb)?;
            let connect =
                !pours(&pcb, &layers)?.is_empty() && router_config.pours != KiCadPourMode::Tracks;
            let board = lower(
                &without_copper_texts(&pcb, placer_config.keep_board_marks),
                router_config,
                connect,
            )?
            .board;
            let mut quick = quick.clone();
            quick.deadline_work = Some((work_deadline - *work - *pool_excess).max(0.0));
            let mut router = core::router::Router::new(&board, &quick);
            let result = router.run_in_place();
            *work += router.expansions() as f64 / core::router::expansions_per_second();
            Some(score(&result, &board).0)
        } else {
            None
        };
        eprintln!(
            "outline at {factor} times the parts' area{}: {}",
            placement.outline_mm.map_or(String::new(), |size| format!(
                " ({:.1} x {:.1} mm)",
                size[0], size[1]
            )),
            match open {
                Some(open) => format!("{open} open"),
                None => "parts do not fit".into(),
            }
        );
        trials.push(KiCadOutlineTrial {
            area_factor: factor,
            size_mm: placement.outline_mm,
            legal,
            open,
            used: false,
        });
        if scale < 1.0 {
            // Shrinking: stop at the first size that no longer routes.
            if open != Some(0) {
                break;
            }
            chosen = Some(factor);
            continue;
        }
        chosen = Some(factor);
        // Growing stops at the first size that routes; the start size with
        // `shrink` goes on to smaller ones.
        if open == Some(0) && !(shrink && scale == 1.0) {
            break;
        }
    }
    // None routed completely: the size with the fewest open connections
    // (the larger one on a tie), for the moves to finish.
    if trials.iter().all(|trial| trial.open != Some(0))
        && let Some(best) = trials
            .iter()
            .filter(|trial| trial.open.is_some())
            .min_by(|a, b| {
                a.open
                    .cmp(&b.open)
                    .then(b.area_factor.total_cmp(&a.area_factor))
            })
    {
        chosen = Some(best.area_factor);
    }
    if let (Some(factor), Some(outline)) = (chosen, automatic_outline(placer_config)) {
        outline.area_factor = Some(factor);
        for trial in &mut trials {
            trial.used = trial.area_factor == factor;
        }
    }
    Ok(trials)
}

/// The constraints' outline when it is sized from the parts alone.
fn automatic_outline(
    config: &mut KiCadBoardPlacerConfig,
) -> Option<&mut crate::placement_constraints::KiCadOutlineConstraint> {
    match &mut config.constraints {
        Some(KiCadConstraintsSource::Inline(constraints)) => constraints
            .outline
            .as_mut()
            .filter(|outline| outline.automatic() && outline.area_factor.is_none()),
        _ => None,
    }
}

/// Lower ordering is better: open connections first, then vias and copper.
fn score(result: &core::RoutingResult, board: &core::Board) -> (usize, f64) {
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
    (open, length + 10.0 * vias as f64)
}

/// Mean congestion under a body plus a margin, on the routing lattice.
fn body_congestion(
    result: &core::RoutingResult,
    center: [f64; 2],
    half: [f64; 2],
    margin: f64,
) -> f64 {
    let grid = &result.grid;
    let range = |axis: usize, size: usize| {
        let low = center[axis] - half[axis] - margin;
        let high = center[axis] + half[axis] + margin;
        let first = ((low - grid.origin[axis]) / grid.pitch).ceil().max(0.0) as usize;
        let last = (((high - grid.origin[axis]) / grid.pitch).floor().max(0.0) as usize)
            .min(size.saturating_sub(1));
        (first, last)
    };
    let (x0, x1) = range(0, grid.nx);
    let (y0, y1) = range(1, grid.ny);
    let mut total = 0.0;
    let mut nodes = 0usize;
    for y in y0..=y1 {
        for x in x0..=x1 {
            total += result.congestion[y * grid.nx + x] as f64;
            nodes += 1;
        }
    }
    if nodes > 0 { total / nodes as f64 } else { 0.0 }
}

pub(crate) fn footprint_items(pcb: &mut Expr) -> Result<Vec<&mut Expr>, String> {
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    Ok(items
        .iter_mut()
        .filter(|item| item.head() == Some("footprint"))
        .collect())
}

/// Places and routes `<source>/<board_id>.kicad_pcb`. The placement goes to
/// `output/placed`, the final routed board to `output/result`.
/// The board the layout routes: with its copper texts where the input put
/// them (`keep`: `keep_board_marks`, obstacles to the router as to KiCad),
/// or else without them: decoration that moves off the copper afterwards
/// (`labels`), not obstacles at their old spots (before 2026-10-21).
pub(crate) fn without_copper_texts(pcb: &Expr, keep: bool) -> Expr {
    let Expr::List(items) = pcb else {
        return pcb.clone();
    };
    if keep {
        return pcb.clone();
    }
    // Exposed-copper artwork (a copper text with its mask twin) is not
    // moved afterwards, so it stays an obstacle here (Quanta75: routes
    // under its 7 mm logo, 11 bridges and a short).
    let twins = crate::labels::mask_twins(pcb);
    Expr::List(
        items
            .iter()
            .filter(|item| {
                !(item.head() == Some("gr_text")
                    && form_atom(item, "layer", 1).is_some_and(|layer| {
                        layer.ends_with(".Cu") && !crate::labels::has_mask_twin(&twins, item, layer)
                    }))
            })
            .cloned()
            .collect(),
    )
}

/// The open count past which a board the narrow-signal step applies to
/// counts as far from complete after its first route: 2 % of its
/// terminals, at least 10 (jetson: 31 open, the limit 27; the boards
/// the moves complete have a few open).
pub(crate) fn far_open_limit(terminals: usize) -> usize {
    ((terminals as f64 * 0.02) as usize).max(10)
}

/// Whether `open` connections after the first route are more than
/// `far_open_limit` of `terminals`.
pub(crate) fn far_from_complete(open: usize, terminals: usize) -> bool {
    open > far_open_limit(terminals)
}

/// The negotiation and the clean-up of a narrow-signal attempt run
/// alone, from the work left: the clean-up a tenth (30 to 120 s; it
/// shortens tracks and saves vias but closes nothing, and its default of
/// 180 s took a third of jetson's attempt), the negotiation three quarters
/// of the rest (60 to 900 s), the hard repair, stitching and spokes the
/// remainder (jetson: 302 s of negotiation left 28 nets in conflict and
/// 23 unconnected, 408 s 21 and 18; 0.85 of the rest, 462 s, plateaued at
/// 21 and ended at 21 unconnected).
///
/// The caps grow with the work left, as the ladder's budget does
/// (2026-10-21): the clean-up at most 120 s or a twentieth of what is
/// left, the negotiation at most 900 s or half of it. Up to 1800 s left
/// that is the split it always was; a layout of 3000 to 6000 s on this
/// path used to end near its first route and twice 1020 s, with the rest
/// of its budget unused, and now gives the attempt about half of what is
/// left and the attempt in the other pour mode most of the rest.
pub(crate) fn direct_narrow_budget(work_left: f64) -> (f64, f64) {
    let cleanup = (0.1 * work_left).clamp(30.0, (0.05 * work_left).max(120.0));
    let negotiation = (0.75 * (work_left - cleanup)).clamp(60.0, (0.5 * work_left).max(900.0));
    (negotiation, cleanup)
}

/// The KiCad checks the neck repair may make with `work_left` of a
/// layout's work, a check counting `check_seconds` (`None`: checks cost
/// nothing, the repair's own limit holds).
pub(crate) fn neck_check_bound(work_left: f64, check_seconds: f64) -> Option<usize> {
    (check_seconds > 0.0).then(|| (work_left.max(0.0) / check_seconds).floor() as usize)
}

/// The work the attempt after the narrow-signal attempt run alone (that
/// attempt again in the other pour mode) needs left: an attempt with a
/// negotiation of a few minutes, its repair and KiCad's check.
pub(crate) const NARROW_CONTINUATION_FLOOR: f64 = 240.0;

/// How many of the final ladder's attempts, by the router's measures with
/// the layout's own board among them, KiCad checks (`CheckGate`).
pub(crate) const KICAD_CHECK_TOP: usize = 2;

/// Whether the attempt after the narrow-signal attempt run alone runs:
/// when connections are open, the wall clock has not passed the layout's
/// guard and at least `NARROW_CONTINUATION_FLOOR` of work is left.
pub(crate) fn narrow_continuation_runs(work_left: f64, open: usize, wall_passed: bool) -> bool {
    !(open == 0 || wall_passed || work_left < NARROW_CONTINUATION_FLOOR)
}

/// The steps of a layout after placement that improve the board, in
/// their order. The steps that finish the result (labels, KiCad's check
/// of the board kept, the report) run whatever the wall clock says and
/// do not ask the guard: asking would only fire it later than an
/// improving step saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayoutStep {
    /// The other seeds' placements probed against the kept one.
    Race,
    /// A move trial.
    Move,
    /// The first route resumed at the full patience.
    Resume,
    /// The clean-up, via reduction and spokes of the kept routes.
    Polish,
    /// The final ladder (or the narrow-signal attempt run alone).
    Ladder,
    /// The attempt after the narrow-signal attempt.
    Continuation,
    /// The repair of KiCad's connection_width necks on the board kept.
    NeckRepair,
}

impl LayoutStep {
    fn name(self) -> &'static str {
        match self {
            Self::Race => "the placement race",
            Self::Move => "the move trials",
            Self::Resume => "the resume of the first route",
            Self::Polish => "the polish",
            Self::Ladder => "the final ladder",
            Self::Continuation => "the attempt after the narrow-signal attempt",
            Self::NeckRepair => "the neck repair",
        }
    }
}

/// Once the wall clock passes the layout's deadline (`passed`), the layout
/// goes straight to finishing: the board kept so far, its labels, KiCad's
/// check and the report; every step that would improve the board is
/// skipped (`allows`). What ran after the guard used to take the harness's
/// margin and more: poly_kybd_split72_left in v16 ran three move trials of
/// 60-200 s past it and was killed at 1800 s without a result. The skip is
/// a wall-clock decision, noted as a `layout` cut; without the guard the
/// layout is as before.
pub(crate) struct WallStop<F: Fn() -> bool> {
    passed: F,
    fired: std::cell::Cell<bool>,
    skipped: std::cell::RefCell<Vec<LayoutStep>>,
}

impl<F: Fn() -> bool> WallStop<F> {
    pub(crate) fn new(passed: F) -> Self {
        Self {
            passed,
            fired: std::cell::Cell::new(false),
            skipped: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// Whether `step` runs: until the wall clock has passed the deadline.
    pub(crate) fn allows(&self, step: LayoutStep) -> bool {
        if !self.fired.get() && (self.passed)() {
            self.fired.set(true);
        }
        if !self.fired.get() {
            return true;
        }
        let mut skipped = self.skipped.borrow_mut();
        if !skipped.contains(&step) {
            skipped.push(step);
            eprintln!(
                "layout: past the deadline, {} skipped: finishing with the board kept so far",
                step.name()
            );
            core::router::note_wall_cut(
                "layout",
                format_args!("{} skipped past the deadline", step.name()),
            );
        }
        false
    }

    /// Whether the wall clock has passed the deadline (as `allows` last saw it).
    pub(crate) fn fired(&self) -> bool {
        self.fired.get()
    }

    /// The steps skipped, in the order they were first skipped.
    pub(crate) fn skipped(&self) -> Vec<LayoutStep> {
        self.skipped.borrow().clone()
    }
}

pub fn layout_kicad_board(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardLayoutConfig,
    router_config: &KiCadBoardRouterConfig,
) -> Result<KiCadBoardLayoutResult, String> {
    if output_directory.exists() {
        return Err(format!(
            "output directory {} already exists",
            output_directory.display()
        ));
    }
    // The search over sizes is the command's (each size a layout of its
    // own): a layout at one size has none.
    if config.free_dimension.is_some() {
        return Err(crate::verdict::input_error(
            "free_dimension: the layout-kicad-board command searches the sizes; a single layout takes none",
        ));
    }
    fs::create_dir_all(output_directory)
        .map_err(|error| format!("failed to create {}: {error}", output_directory.display()))?;
    let layout_started = std::time::Instant::now();
    let elapsed = || layout_started.elapsed().as_secs_f64();
    // The budget counts work, in seconds of an idle machine (placement work
    // and search expansions), so that a busy or a slower machine lays the
    // board out the same way. The wall clock only guards against running
    // far past it.
    let mut work = 0.0;
    // The placement's work counts its longest seed (the budget's measure);
    // its seeds run in a pool, and the clock sees them as the reference
    // machine runs them (`pool_work_seconds`, from the seeds' work alone):
    // the difference counts toward the deadline in work (`work_deadline`),
    // not toward the budgets (video: 193 s of placement work, 630 s of wall
    // clock in waves; its first route ran into the wall-clock deadline at a
    // point that depended on the load).
    let mut pool_excess = 0.0;
    // Requested net classes set their nets' rules (and the spacing).
    let with_classes;
    let router_config = if router_config.net_classes.is_empty() {
        router_config
    } else {
        let board = source_directory.join(format!("{board_id}.kicad_pcb"));
        let text = fs::read_to_string(&board)
            .map_err(|error| format!("failed to read {}: {error}", board.display()))?;
        with_classes = crate::net_classes::with_net_classes(router_config, &parse(&text)?)
            .map_err(crate::verdict::input_error)?;
        &with_classes
    };
    // KiCad's checks inside the layout (the board kept, the ladder's
    // attempts, the neck repair) count as work at a cost modelled from the
    // board alone (`check_work`), the same whether the DRC cache answers
    // a check or not: 10 to 50 s of kicad-cli each when uncached, which the
    // wall-clock guard used to see and the work did not.
    let check_seconds = if router_config.skip_native_verification {
        0.0
    } else {
        match router_config.kicad_check_work {
            Some(seconds) => seconds,
            None => {
                let board = source_directory.join(format!("{board_id}.kicad_pcb"));
                let text = fs::read_to_string(&board)
                    .map_err(|error| format!("failed to read {}: {error}", board.display()))?;
                let inputs = crate::check_work::check_inputs(&parse(&text)?);
                // Modelled on the reference: on a machine the work rates
                // find slower, a check takes longer by as much.
                let seconds =
                    crate::check_work::check_seconds(inputs) * core::router::reference_time_scale();
                eprintln!(
                    "layout: a KiCad check counts {seconds:.1} s of work ({} hatched zones, {:.0} hatch holes, {} Edge.Cuts items)",
                    inputs.hatched_zones, inputs.hatch_holes, inputs.edge_items
                );
                seconds
            }
        }
    };
    let mut checks_charged = crate::check_work::CheckTally::new(check_seconds);
    // Every route of the layout ends by a wall-clock guard past its total
    // budget: on a much busier or slower machine the work would run on
    // past what the agent waits for.
    // (None under `PCB_ROUTER_NO_WALL_GUARD`: the work alone decides.)
    // Stretched where a pinned rate is faster than the machine
    // (`KiCadWorkRate::wall_stretch`, the search's or the placer's).
    let guard_seconds = wall_guard_seconds(config.total_seconds, config.wall_guard)?
        * crate::work_rate::work_rate().wall_stretch(true);
    let with_deadline = KiCadBoardRouterConfig {
        deadline: router_config.deadline.or(core::router::wall_guard()
            .then(|| layout_started + std::time::Duration::from_secs_f64(guard_seconds))),
        kicad_check_work: Some(check_seconds),
        ..router_config.clone()
    };
    let router_config = &with_deadline;
    // The layout's end in work, its nominal end: every router of the
    // layout stops there (`arm`), the polish and the final ladder are
    // planned up to it, so that where a run stops does not depend on the
    // machine's load. The wall-clock deadline `wall_guard` times it is the
    // margin for load (and for what the work does not count); a run that
    // reaches it first says so (`cut`). With both at 1.1 times the budget,
    // the wall clock decided every crowded board under a sweep's load
    // (video: work 1373 s of 1650 when the deadline cut its repair).
    let work_deadline = config.total_seconds.max(0.0);
    let arm = |router: &mut core::router::Router, work: f64| {
        let own = router.work();
        router.config_mut().deadline_work = Some(own + (work_deadline - work).max(0.0));
    };
    let wall_passed = || {
        router_config
            .deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
    };
    let stop = WallStop::new(wall_passed);
    let mut placer_config = resolve_constraints(&config.placer, source_directory)
        .map_err(crate::verdict::input_error)?;
    // Bodies keep at least the largest copper clearance apart.
    if placer_config.copper_clearance_mm.is_none() {
        placer_config.copper_clearance_mm = Some(largest_clearance(router_config));
    }
    if placer_config.hole_clearance_mm.is_none() {
        placer_config.hole_clearance_mm = Some(router_config.hole_clearance_mm);
    }
    if placer_config.solder_mask_to_copper_mm.is_none() {
        placer_config.solder_mask_to_copper_mm = router_config.solder_mask_to_copper_mm;
    }
    placer_config.edge_margin_mm = placer_config
        .edge_margin_mm
        .max(router_config.edge_clearance_mm);
    if placer_config.copper_edge_clearance_mm.is_none() {
        placer_config.copper_edge_clearance_mm = Some(router_config.edge_clearance_mm);
    }
    // Placement gets 40 % of the budget, as the placement race does.
    let placement_budget = 0.4 * config.total_seconds.max(0.0);
    // The placement's budget is its work; its wall-clock guard is the
    // layout's own deadline (at the placement's share of it, the placement
    // stopped on a busy machine where the work would have gone on, and the
    // layout depended on the load).
    placer_config.deadline = router_config.deadline;
    placer_config.work_seconds = Some(placement_budget);
    if placer_config.global_guidance.is_some() {
        placer_config.guidance_router = Some(router_config.clone());
    }
    eprintln!("layout: sizing the outline");
    let outline_sizing = size_outline(
        source_directory,
        board_id,
        output_directory,
        &mut placer_config,
        router_config,
        &mut work,
        work_deadline,
        &mut pool_excess,
    )?;
    let placed_directory = output_directory.join("placed");
    eprintln!("layout: placing ({:.0} s, work {work:.0} s)", elapsed());
    // What sizing the outline left of the placement budget (a little at
    // least: without any, placement would not anneal at all).
    placer_config.work_seconds = Some((placement_budget - work).max(0.1 * placement_budget));
    // A congestion model ranks more of the placer's seeds than the race
    // could probe.
    let congestion_model = match &config.congestion_model {
        Some(path) => {
            Some(pcb_congestion::CongestionModel::load(path).map_err(crate::verdict::input_error)?)
        }
        None => None,
    };
    // Only where the race runs: one router seed, no pin swaps.
    let pre_ranking = config.congestion_score != "none"
        && (config.congestion_score == "rudy"
            || config.congestion_score == "global"
            || congestion_model.is_some())
        && router_config.seeds.unwrap_or(1) <= 1
        && config.pin_swaps.is_none()
        && config.swappable.is_empty();
    if config.congestion_score != "none"
        && config.congestion_score != "rudy"
        && config.congestion_score != "global"
        && congestion_model.is_none()
    {
        return Err(crate::verdict::input_error(format!(
            "congestion_score {:?} needs a congestion_model",
            config.congestion_score
        )));
    }
    if pre_ranking {
        placer_config.placement_seeds = placer_config
            .placement_seeds
            .max(config.congestion_candidates);
    }
    let placement = place_kicad_board(
        source_directory,
        board_id,
        &placed_directory,
        &placer_config,
    )?;
    work += placement.work_seconds;
    pool_excess += (placement.pool_work_seconds - placement.work_seconds).max(0.0);
    let pool_excess = pool_excess;
    // The marks that belong to a movable part were taken from the input;
    // the placed board's lowerings keep them so.
    placer_config.attached_marks = Some(placement.attached_mark_pairs.clone());
    if wall_passed() {
        core::router::note_wall_cut(
            "placement",
            format_args!("the placement ended past the deadline, {work:.0} s of work done"),
        );
    }
    eprintln!(
        "layout: placed ({:.0} s, work {work:.0} s, {} seeds kept of {} run, seed {} kept; {:.0} s more of it in the pool of seeds, {:.0} s of seeds in all)",
        elapsed(),
        placement.seeds,
        placement.seeds_run,
        placement.seed_kept,
        pool_excess,
        placement.seeds_work_seconds
    );
    // The moves keep parts on `"edge": "any"` at the edges they were given.
    if let Some(KiCadConstraintsSource::Inline(constraints)) = &mut placer_config.constraints {
        for entry in &mut constraints.edge {
            if let Some(edge) = placement.edges_chosen.get(&entry.part) {
                entry.edge = edge.clone();
            }
        }
    }
    if !placement.unplaced.is_empty() || !placement.illegal.is_empty() {
        return Err(format!(
            "placement is not legal (unplaced {:?}, illegal {:?}); {}",
            placement.unplaced,
            placement.illegal,
            placement.hints.join("; ")
        ));
    }

    // Pins are assigned for the placement; the placed project (board and
    // schematic) is what the result is built from.
    let pin_swaps = match &config.pin_swaps {
        Some(path) => {
            let spec = read_pin_swap_spec(&source_directory.join(path))?;
            Some(swap_kicad_pins_in_place(
                &placed_directory,
                board_id,
                &spec,
                &config.pin_swap,
            )?)
        }
        None if !config.swappable.is_empty() => {
            let placed = placed_directory.join(format!("{board_id}.kicad_pcb"));
            let text = fs::read_to_string(&placed)
                .map_err(|error| format!("failed to read {}: {error}", placed.display()))?;
            let spec = swappable_spec(
                &crate::canonical_order::parse_board(&text)?,
                &config.swappable,
            )?;
            Some(swap_kicad_pins_in_place(
                &placed_directory,
                board_id,
                &spec,
                &config.pin_swap,
            )?)
        }
        None => None,
    };

    let placed_board = placed_directory.join(format!("{board_id}.kicad_pcb"));
    let source = fs::read_to_string(&placed_board)
        .map_err(|error| format!("failed to read {}: {error}", placed_board.display()))?;
    let mut pcb = crate::canonical_order::parse_board(&source)?;
    // Requested pours (a ground plane) join the placed board.
    let planned = planned_pours(&pcb, router_config)?;
    if !planned.is_empty() {
        if router_config.add_pours.is_empty() {
            eprintln!(
                "automatic planes: {}",
                planned
                    .iter()
                    .map(|pour| format!("{} on {}", pour.net, pour.layers.join(", ")))
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }
        add_pour_zones(&mut pcb, &planned)?;
        fs::write(&placed_board, format!("{}\n", encode(&pcb)))
            .map_err(|error| format!("failed to write {}: {error}", placed_board.display()))?;
    }
    if !router_config.net_classes.is_empty() {
        crate::net_classes::write_net_classes(
            &placed_directory,
            board_id,
            &pcb,
            &router_config.net_classes,
        )?;
    }
    let layers = LayerTable::from_pcb(&pcb)?;
    let has_pours = !pours(&pcb, &layers)?.is_empty();
    let connect = has_pours && router_config.pours != KiCadPourMode::Tracks;
    // The moves that follow keep to the rules the placement needed: on a
    // board placed with relaxed spacing, a move checked against the full
    // spacing is never legal.
    placer_config.tight_bodies = Some(placement.tight_bodies);
    if placer_config.holes_in_courtyards_error.is_none() {
        placer_config.holes_in_courtyards_error =
            Some(crate::board_placer::holes_in_courtyards_error(
                &source_directory.join(format!("{board_id}.kicad_pro")),
            ));
    }
    let mut problem = lower_placement(&pcb, &placer_config)?;
    let first_relaxation = placer::Relaxation {
        spacing: placement.spacing_mm,
        grid: placement.grid_mm,
        halo_scale: placement.halo_scale,
        small_halo_scale: placement.small_halo_scale,
        edge_inset: placement.edge_inset,
        tight: placement.tight_bodies,
        edge_copper: placement.edge_copper,
        edge_rule: placement.edge_rule_mm,
        courtyard_spacing: placement.courtyard_spacing,
        halos: placement.halos.clone(),
        on_marks: placement.on_marks.clone(),
    };
    first_relaxation.apply(&mut problem.problem);
    // The in-place router compares placements: it stops negotiating once
    // conflicts no longer really fall (a move on OpenAirScope negotiated
    // 64 iterations at 15 conflicted nets, 200-350 s a move). The final
    // ladder routes with the full patience.
    let mut core_config = core_config(router_config);
    if router_config.stall_drop.is_none() {
        core_config.stall_drop = 0.1;
    }
    if router_config.stall_at_cap.is_none() {
        core_config.stall_at_cap = core_config.stall_at_cap.min(10);
    }
    if router_config.present_growth.is_none() {
        core_config.present_growth = core_config.present_growth.max(2.0);
    }
    // A trial only ranks placements, and the final ladder routes the kept
    // one with the full patience: below the price cap, eight iterations
    // without fewer conflicted nets end it (OpenAirScope's moves
    // negotiated ~34 iterations, 126-229 s each).
    core_config.stall_patience = core_config.stall_patience.min(8);
    // A new router of the first route stops where the layout's work ends.
    core_config.deadline_work = Some((work_deadline - work - pool_excess).max(0.0));

    let first_started = std::time::Instant::now();
    let mut board = lower(
        &without_copper_texts(&pcb, config.placer.keep_board_marks),
        router_config,
        connect,
    )?
    .board;
    // With seeds, the first route is done several ways at once; the best
    // router state carries the move phase.
    let seeds = router_config.seeds.unwrap_or(1).max(1) as u64;
    let route_once = |board: &core::Board| {
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..seeds)
                .map(|index| {
                    let mut seeded = core_config.clone();
                    // As the race's resume below: at most half of what is
                    // left before the deadline negotiates.
                    let left = seeded.deadline_work.unwrap_or(f64::INFINITY);
                    let half =
                        ((left / 2.0).max(1.0) * core::router::expansions_per_second()) as u64;
                    seeded.negotiation_expansions = if seeded.negotiation_expansions == 0 {
                        half
                    } else {
                        seeded.negotiation_expansions.min(half)
                    };
                    seeded.seed = (index > 0).then_some(index);
                    seeded.verbose = core_config.verbose && index == 0;
                    scope.spawn(move || {
                        // Placements are compared on what stays open; the
                        // polish waits for the one kept.
                        let mut router = core::router::Router::new(board, &seeded);
                        let result = router.run_in_place_polished(false);
                        (router, result)
                    })
                })
                .collect();
            let routed: Vec<_> = handles
                .into_iter()
                .map(|handle| handle.join().expect("routing thread"))
                .collect();
            let expansions: u64 = routed.iter().map(|(router, _)| router.expansions()).sum();
            let best = routed
                .into_iter()
                .min_by(|a, b| {
                    score(&a.1, board)
                        .partial_cmp(&score(&b.1, board))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .expect("at least one seed");
            (
                best.0,
                best.1,
                expansions as f64 / core::router::expansions_per_second(),
            )
        })
    };
    // What the user asked for comes first: only placements that keep the
    // constraints as well as the kept one race it. Seeds that placed alike
    // (every part fixed: the laptop raced one placement against itself for
    // 6 minutes) count once.
    let missed: f64 = placement
        .constraints
        .iter()
        .map(|status| status.violation_mm)
        .sum();
    let alike = |a: &[placer::Pose], b: &[placer::Pose]| {
        a.len() == b.len()
            && a.iter().zip(b).all(|(a, b)| {
                (a.position[0] - b.position[0]).abs() < 1.0e-3
                    && (a.position[1] - b.position[1]).abs() < 1.0e-3
                    && ((a.angle - b.angle).rem_euclid(360.0) + 1.0e-3) % 360.0 < 2.0e-3
            })
    };
    // The placements a congestion model scores (the kept one first).
    let mut pool: Vec<(Vec<placer::Pose>, placer::Relaxation)> = Vec::new();
    let mut others: Vec<(&Vec<placer::Pose>, &placer::Relaxation)> = Vec::new();
    {
        let mut seen = vec![problem.problem.poses.clone()];
        for (poses, relaxation, other_missed) in &placement.alternatives {
            if *other_missed <= missed + 1.0e-6 && !seen.iter().any(|known| alike(known, poses)) {
                seen.push(poses.clone());
                others.push((poses, relaxation));
            }
        }
    }
    // The seeds' placements are ranked by predicted routability (RUDY, or
    // the model), and only the best few are probed (a probe is a real
    // route of up to `probe_seconds`; a score takes milliseconds). The
    // race below then judges those as before, in the ranking's order.
    // Scores and their order depend only on the placements, which the
    // placer makes per seed and sorts by its own key: nothing here
    // depends on thread timing.
    let mut congestion = None;
    if pre_ranking
        && seeds == 1
        && pin_swaps.is_none()
        && others.len() + 1 > config.congestion_probes.max(1)
    {
        pool.push((problem.problem.poses.clone(), first_relaxation));
        pool.extend(
            others
                .iter()
                .map(|(poses, relaxation)| ((*poses).clone(), (*relaxation).clone())),
        );
        let ranked = crate::congestion_rank::rank_placements(
            &without_copper_texts(&pcb, config.placer.keep_board_marks),
            &pool,
            congestion_model.as_ref(),
            router_config,
            &core_config,
            connect,
            &config.congestion_score,
        )?;
        // A placement that needed courtyards touching is a last resort
        // whatever its score: it is probed only when too few others are
        // (laptop power: RUDY picked a touching seed, 18 connections open
        // against 9 for the best of the five that needed no touching).
        let mut order = ranked.order.clone();
        order.sort_by_key(|index| {
            (
                pool[*index].1.courtyard_spacing,
                pool[*index].1.on_marks.len(),
            )
        });
        let keep: Vec<usize> = order
            .iter()
            .copied()
            .take(config.congestion_probes.max(1))
            .collect();
        eprintln!(
            "placement pre-ranking ({}): {} placements scored in {:.2} s (lowering {:.2} s, features {:.2} s, inference {:.2} s); probing {:?} (scores {:?})",
            config.congestion_score,
            pool.len(),
            ranked.seconds,
            ranked.lower_seconds,
            ranked.feature_seconds,
            ranked.inference_seconds,
            keep,
            keep.iter()
                .map(|index| (ranked.scores[*index] * 100.0).round() / 100.0)
                .collect::<Vec<_>>()
        );
        if keep[0] != 0 {
            let (poses, relaxation) = &pool[keep[0]];
            let mut candidate = pcb.clone();
            {
                let mut footprints = footprint_items(&mut candidate)?;
                for (footprint, pose) in footprints.iter_mut().zip(poses) {
                    write_footprint_pose(footprint, *pose)?;
                }
            }
            board = lower(
                &without_copper_texts(&candidate, config.placer.keep_board_marks),
                router_config,
                connect,
            )?
            .board;
            pcb = candidate;
            placer_config.tight_bodies = Some(relaxation.tight);
            problem = lower_placement(&pcb, &placer_config)?;
            relaxation.apply(&mut problem.problem);
        }
        others = keep[1..]
            .iter()
            .map(|index| (&pool[*index].0, &pool[*index].1))
            .collect();
        congestion = Some(KiCadCongestionRanking {
            scores: ranked.scores,
            probed: keep,
        });
    }
    let mut race = Vec::new();
    let (mut router, mut result) = if seeds == 1
        && pin_swaps.is_none()
        && !others.is_empty()
        && stop.allows(LayoutStep::Race)
    {
        // Wirelength is not routability: each placement is probed with the
        // same search, as route mode probes its rungs, and only the one
        // with the fewest nets unfinished routes on (routing the others in
        // full took as long as the first route and left link no time for
        // anything after).
        let probe_seconds = router_config.probe_seconds.unwrap_or(75.0);
        // The other seeds' placements, lowered, in seed order.
        let mut candidates = Vec::new();
        for (poses, relaxation) in others {
            let mut candidate = pcb.clone();
            {
                let mut footprints = footprint_items(&mut candidate)?;
                for (footprint, pose) in footprints.iter_mut().zip(poses) {
                    write_footprint_pose(footprint, *pose)?;
                }
            }
            let candidate_board = lower(
                &without_copper_texts(&candidate, config.placer.keep_board_marks),
                router_config,
                connect,
            )?
            .board;
            candidates.push((candidate, candidate_board, relaxation));
        }
        // The placements are probed side by side, each a router of its own,
        // as many at once as fit half the memory available (the first
        // router tells its size). The outcome is the one of probing them
        // one after the other: the first placement, then the others in
        // seed order while the work stays under 40 % of the budget, the
        // fewest unfinished nets kept, a tie to the earlier one. A probe's
        // budget is work, so what runs beside it does not change it.
        let before_mb = crate::board_router::resident_mb();
        let first = core::router::Router::new(&board, &core_config);
        let router_mb = (crate::board_router::resident_mb() - before_mb).max(1.0);
        let side_by_side = crate::board_router::memory_mb()
            .map_or(1, |(_, available)| {
                (available * 0.5 / (router_mb * 1.5)) as usize
            })
            .clamp(1, candidates.len() + 1);
        // The memory ledger: as many as it has room for, reserved first.
        let side_by_side = crate::ledger::routers_side_by_side(
            0.0,
            router_mb * 1.5,
            side_by_side,
            "the placement race's routers",
        );
        if side_by_side > 1 {
            eprintln!(
                "placement race: {} placements, {side_by_side} side by side",
                candidates.len() + 1
            );
        }
        let mut probed: Vec<(usize, core::router::Router)> = Vec::new();
        let mut first = Some(first);
        let mut next = 0;
        while next <= candidates.len() {
            let wave: Vec<usize> =
                (next..(next + side_by_side).min(candidates.len() + 1)).collect();
            let routers: Vec<(usize, usize, core::router::Router)> = std::thread::scope(|scope| {
                let handles: Vec<_> = wave
                    .iter()
                    .map(|&index| {
                        let built = if index == 0 { first.take() } else { None };
                        let board = if index == 0 {
                            &board
                        } else {
                            &candidates[index - 1].1
                        };
                        let mut quiet = core_config.clone();
                        // One probe's lines in the log; the others' would
                        // interleave with them.
                        quiet.verbose = quiet.verbose && index == wave[0];
                        scope.spawn(move || {
                            let mut router =
                                built.unwrap_or_else(|| core::router::Router::new(board, &quiet));
                            let unfinished = router.probe(probe_seconds);
                            (index, unfinished, router)
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| handle.join().expect("probe thread"))
                    .collect()
            });
            for (index, unfinished, router) in routers {
                probed.push((index, router));
                race.push(unfinished);
            }
            next += wave.len();
        }
        let mut probed = probed.into_iter();
        let (_, mut kept) = probed.next().expect("the first placement's probe");
        let mut best_unfinished = race[0];
        work += kept.expansions() as f64 / core::router::expansions_per_second();
        let mut raced = 1;
        let mut candidates = candidates.into_iter();
        for (index, mut candidate_router) in probed {
            let (candidate, candidate_board, relaxation) =
                candidates.next().expect("a candidate per probe");
            if work > 0.4 * config.total_seconds {
                break;
            }
            let unfinished = race[index];
            raced += 1;
            work += candidate_router.expansions() as f64 / core::router::expansions_per_second();
            eprintln!(
                "placement race: another seed's placement leaves {unfinished} nets unfinished (best {best_unfinished})"
            );
            if unfinished < best_unfinished {
                best_unfinished = unfinished;
                pcb = candidate;
                board = candidate_board;
                candidate_router.config_mut().verbose = core_config.verbose;
                kept = candidate_router;
                placer_config.tight_bodies = Some(relaxation.tight);
                problem = lower_placement(&pcb, &placer_config)?;
                relaxation.apply(&mut problem.problem);
            }
        }
        // The placements past the work limit count for nothing.
        race.truncate(raced);
        arm(&mut kept, work + pool_excess);
        // The negotiation gets at most half of what is left before the
        // deadline: the hard repair after it may take as long as the
        // negotiation did, and a deadline inside the negotiation leaves
        // every net still in conflict open (video: 195 nets in conflict
        // when the wall clock ended its repair after 26 to 98 of them, at
        // a point that depended on the load).
        let left = (work_deadline - work - pool_excess).max(0.0);
        // (The router's negotiation budget in work: its seconds are the
        // wall-clock guard, `GUARD` times that.)
        let budget =
            core_config.negotiation_expansions as f64 / core::router::expansions_per_second();
        let mut seconds = (budget - probe_seconds).max(60.0);
        if seconds > left / 2.0 {
            seconds = (left / 2.0).max(1.0);
            eprintln!(
                "layout: the first route negotiates on for at most {seconds:.0} s of work ({left:.0} s left before the deadline)"
            );
        }
        let before = kept.expansions();
        let result = kept.resume_polished(seconds, false);
        work +=
            kept.expansions().saturating_sub(before) as f64 / core::router::expansions_per_second();
        (kept, result)
    } else {
        let (router, result, first_work) = route_once(&board);
        work += first_work;
        (router, result)
    };
    let mut best = score(&result, &board);
    let first_route_seconds = first_started.elapsed().as_secs_f64();
    let first = (
        best.0,
        result
            .routes
            .iter()
            .map(|route| route.vias.len())
            .sum::<usize>(),
        result
            .routes
            .iter()
            .flat_map(|route| route.segments.iter())
            .map(|segment| distance_squared(segment.start, segment.end).sqrt())
            .sum::<f64>(),
    );

    let move_started = std::time::Instant::now();
    let move_work_started = work;
    let mut moves = Vec::new();
    let mut since_improvement = 0;
    // While connections are open, trials are judged on the open ones, which
    // the polish (an exact cleanup of every net, up to minutes on a big
    // board) does not change: it waits until the end. The first route was
    // not polished either.
    let mut polished = false;
    // Trials since a move last closed an open connection: when moves stop
    // closing them, the final ladder is the better use of the time.
    let mut since_fewer_open = 0;
    let mut recently: Vec<usize> = Vec::new();
    let movable: Vec<usize> = (0..problem.problem.components.len())
        .filter(|index| {
            !problem.problem.components[*index].fixed
                && !problem.problem.components[*index].pins.is_empty()
        })
        .collect();
    // Moves fine-tune a placement that nearly routes; with many
    // connections open the final ladder (pours as tracks or planes, finer
    // pitches) is the better use of the time, and it never ran when the
    // moves took it (Sisu: 108 unconnected in the layout, 20 when route
    // mode routed the same placement).
    // Where the ladder has no other way of connecting the pours to try
    // (no pours, or two layers), the moves are the only lever left (the
    // breadboard fence on two layers: 41 open, no move was ever tried).
    // A bounded round instead of none (at most four parts in a tenth of
    // the budget, tried 2026-10-21) lost time for nothing: DSP-ADAU1452 at
    // a third of its budget, 100 open after the first route, its three
    // trials rerouted 32 nets each and left 224 to 285 open (a reroute in
    // that congestion does not settle), none kept, 89 s of work more for
    // the same board.
    let terminals: usize = board.nets.iter().map(|net| net.terminals.len()).sum();
    let ladder_has_rungs = connect && layers.names.len() >= 4;
    let many_open = ladder_has_rungs && best.0 > ((terminals as f64 * 0.05) as usize).max(10);
    // Where the narrow-signal step applies, a board still far from
    // complete after its first route skips the moves, the resume and the
    // polish too, and the narrow-signal attempt gets the time (below): the
    // step is what nearly completes such boards (jetson: 48 to 6 open in
    // route mode), and the moves and the resume took 388 s of jetson's
    // work for 31 to 25 open and left 218 s, too little for any ladder.
    let narrow_applies = crate::board_router::narrow_step_applies(router_config);
    let far_open =
        narrow_applies && ladder_has_rungs && !many_open && far_from_complete(best.0, terminals);
    if many_open {
        eprintln!(
            "layout: {} open after the first route: no moves, the final ladder gets the time",
            best.0
        );
    } else if far_open {
        eprintln!(
            "layout: {} open after the first route (more than {}), the narrow-signal step applies: no moves, the narrow-signal attempt gets the time",
            best.0,
            far_open_limit(terminals)
        );
    }
    let many_open = many_open || far_open;
    // `global_guidance` with `moves`: the parts by their attribution in
    // the global route of the kept board, recomputed when a move is kept
    // (docs/global-route.md, "Phase 2").
    let guided_moves = placer_config
        .global_guidance
        .as_ref()
        .filter(|guidance| guidance.moves)
        .map(|guidance| guidance.threshold);
    let guidance_index = crate::board_placer::reference_index(&problem.references);
    let mut guided_attribution: Option<Vec<f64>> = None;
    for _ in 0..if many_open { 0 } else { config.moves } {
        if !stop.allows(LayoutStep::Move) {
            break;
        }
        arm(&mut router, work + pool_excess);
        if best.0 == 0 && since_improvement >= config.patience {
            break;
        }
        let budget = if best.0 == 0 {
            config.move_seconds.min(config.polish_seconds)
        } else {
            config.move_seconds
        }
        .min((0.65 * config.total_seconds - move_work_started).max(0.0));
        if since_improvement >= 2 * config.patience
            || (best.0 > 0 && since_fewer_open >= config.patience / 2)
            || work - move_work_started > budget
            || router.past_deadline()
        {
            break;
        }
        if let Some(threshold) = guided_moves
            && guided_attribution.is_none()
        {
            let route = pcb_global::route(&board, &pcb_global::GlobalConfig::default());
            guided_attribution = Some(
                crate::board_placer::attribution(
                    &board,
                    &route,
                    &guidance_index,
                    problem.problem.components.len(),
                    threshold,
                )
                .0,
            );
        }
        // The most congested movable footprint that was not tried lately
        // (with the guidance: the most overflow-attributed one first).
        let mut ranked: Vec<(f64, f64, usize)> = movable
            .iter()
            .filter(|index| !recently.contains(index))
            .map(|index| {
                let component = &problem.problem.components[*index];
                let pose = problem.problem.poses[*index];
                (
                    guided_attribution
                        .as_ref()
                        .map_or(0.0, |attribution| attribution[*index]),
                    body_congestion(
                        &result,
                        component.center(pose),
                        component.half_extent(pose.angle),
                        2.0,
                    ),
                    *index,
                )
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then(b.1.total_cmp(&a.1))
                .then(a.2.cmp(&b.2))
        });
        let Some(&(attributed, congestion, index)) = ranked.first() else {
            break;
        };
        let congestion = congestion.max(attributed);
        if congestion <= 0.0 && best.0 == 0 {
            break;
        }
        recently.push(index);
        if recently.len() > movable.len() / 3 + 1 {
            recently.remove(0);
        }
        let component = &problem.problem.components[index];
        let current = problem.problem.poses[index];

        // Candidate poses: steps along the axes and quarter turns, legal
        // against the placement, ranked by the congestion they land in.
        let mut candidates: Vec<(f64, placer::Pose)> = Vec::new();
        for angle in component.angle_options.iter().copied() {
            for step in config.steps_mm.iter().copied() {
                for (dx, dy) in [(step, 0.0), (-step, 0.0), (0.0, step), (0.0, -step)] {
                    let center = component.center(current);
                    let wanted = [center[0] + dx, center[1] + dy];
                    let position = component.position_for_center(wanted, angle);
                    let pose = placer::Pose {
                        position: placer::constraints::snap_position(
                            &problem.problem,
                            index,
                            position,
                            angle,
                        ),
                        angle,
                    };
                    if pose == current {
                        continue;
                    }
                    if !placer::legal::is_legal(
                        &problem.problem,
                        &problem.problem.poses,
                        index,
                        pose,
                        0..problem.problem.components.len(),
                    ) {
                        continue;
                    }
                    // A nudge must not break a near or relative constraint
                    // further.
                    if !problem.problem.constraints.relations.is_empty() {
                        let before = placer::constraints::relation_penalty(
                            &problem.problem,
                            &problem.problem.poses,
                            Some(index),
                        );
                        let mut trial = problem.problem.poses.clone();
                        trial[index] = pose;
                        if placer::constraints::relation_penalty(
                            &problem.problem,
                            &trial,
                            Some(index),
                        ) > before + 1.0e-9
                        {
                            continue;
                        }
                    }
                    let landing = body_congestion(
                        &result,
                        component.center(pose),
                        component.half_extent(pose.angle),
                        2.0,
                    );
                    candidates.push((landing + 0.01 * (dx.abs() + dy.abs()), pose));
                }
            }
        }
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
        candidates.truncate(3);
        if candidates.is_empty() {
            since_improvement += 1;
            continue;
        }

        let mut improved = false;
        for (_, pose) in candidates {
            // A trial is a reroute and a finish (60-200 s on a big board
            // under load): none starts past the deadline.
            if !stop.allows(LayoutStep::Move) {
                break;
            }
            let trial_started = std::time::Instant::now();
            let saved_router = router.clone();
            let saved_pcb = pcb.clone();
            let saved_pose = problem.problem.poses[index];
            let saved_poses = problem.problem.poses.clone();
            // The part moves, and the parts its row carries with it.
            let mut poses = saved_poses.clone();
            poses[index] = pose;
            problem.problem.sync_followers(&mut poses);
            {
                let mut footprints = footprint_items(&mut pcb)?;
                for (part, (old, new)) in saved_poses.iter().zip(&poses).enumerate() {
                    if part == index || old != new {
                        write_footprint_pose(footprints[part], *new)?;
                    }
                }
            }
            problem.problem.poses = poses;
            let trial_board = lower(
                &without_copper_texts(&pcb, config.placer.keep_board_marks),
                router_config,
                connect,
            )?
            .board;
            arm(&mut router, work + pool_excess);
            let expansions_before = router.expansions();
            // (An error ends the layout: nothing to restore.)
            let rerouted = router.update(&trial_board)?;
            let polish = best.0 == 0;
            let trial = router.reroute(polish);
            work += router.expansions().saturating_sub(expansions_before) as f64
                / core::router::expansions_per_second();
            let trial_score = score(&trial, &trial_board);
            let kept = trial_score < best;
            if kept && trial_score.0 < best.0 {
                since_fewer_open = 0;
            } else {
                since_fewer_open += 1;
            }
            moves.push(KiCadBoardLayoutMove {
                reference: problem.references[index].clone(),
                from: [
                    saved_pose.position[0],
                    saved_pose.position[1],
                    saved_pose.angle,
                ],
                to: [pose.position[0], pose.position[1], pose.angle],
                rerouted_nets: rerouted,
                seconds: trial_started.elapsed().as_secs_f64(),
                kept,
                unconnected_terminals: trial_score.0,
                vias: trial.routes.iter().map(|route| route.vias.len()).sum(),
                length_mm: trial
                    .routes
                    .iter()
                    .flat_map(|route| route.segments.iter())
                    .map(|segment| distance_squared(segment.start, segment.end).sqrt())
                    .sum(),
            });
            if kept {
                best = trial_score;
                result = trial;
                board = trial_board;
                guided_attribution = None;
                polished = polish;
                improved = true;
                break;
            }
            router = saved_router;
            pcb = saved_pcb;
            problem.problem.poses = saved_poses.clone();
        }
        if improved {
            since_improvement = 0;
        } else {
            since_improvement += 1;
        }
    }

    // Connections the first route and the moves left open: before the
    // final ladder starts over, the router goes on from where it is, at
    // the full patience, with a small share of what is left (PolyKybd
    // right: the first route left 33 open after 361 s; the ladder's rungs,
    // started from scratch with the rest, ended at 538 open and the first
    // route stood; the resume took 35 to 32, jetson 26 to 22). A small
    // share: on the laptop motherboard 493 s of it gained nothing, and the
    // ladder, which had completed that board to 2 unconnected, ran out of
    // time. With many open the pour mode is in question: the ladder's.
    arm(&mut router, work + pool_excess);
    if best.0 > 0 && !many_open && !router.past_deadline() && stop.allows(LayoutStep::Resume) {
        let left = (config.total_seconds - work).max(0.0);
        let share = (0.2 * left).min(240.0);
        if share >= 60.0 {
            // A resume that ends worse (MokyaLora: 21 open to 89) must
            // not leave its state behind for the polish: the router is
            // restored with the result.
            let saved_router = router.clone();
            let expansions_before = router.expansions();
            arm(&mut router, work + pool_excess);
            {
                let defaults = core::router::Config::default();
                let full = router.config_mut();
                full.stall_patience = defaults.stall_patience;
                full.stall_at_cap = router_config.stall_at_cap.unwrap_or(defaults.stall_at_cap);
                full.stall_drop = router_config.stall_drop.unwrap_or(defaults.stall_drop);
            }
            let resumed = router.resume_polished(share, false);
            work += router.expansions().saturating_sub(expansions_before) as f64
                / core::router::expansions_per_second();
            let (before, after) = (score(&result, &board), score(&resumed, &board));
            eprintln!(
                "layout: the first route resumed with {share:.0} s: {} open (before {}), {:.0} s",
                after.0,
                before.0,
                elapsed()
            );
            // (`best` is not read past this point.)
            if after <= before {
                result = resumed;
            } else {
                router = saved_router;
            }
        }
    }
    // The polish reroutes everything: past the deadline it would stop at
    // once and throw the moves' best board away (OpenAirScope: 18 open
    // after the moves, 108 of 184 routed after the polish).
    // The polish (clean-up, via reduction, thermal repair) is bounded by
    // the wall clock left before the deadline, and skipped under four
    // minutes of it: PolyKybd right's polish spent 769 s on one via
    // reduction round and was thrown away.
    // What is left before the layout's nominal end, in work (the wall
    // clock would make the polish depend on the machine's load; its
    // deadline still guards the polish inside the router).
    let polish_left = work_deadline - work - pool_excess;
    // A board a move trial already polished polishes again when more than
    // half the layout's budget is left: the trial's via reduction had only
    // its own strict budget (CyberKeeb2040: a round reverted at 60 s, the
    // next not started), the final one gets the run-sized one.
    let polish_again = polished && work < 0.5 * config.total_seconds;
    if polish_again {
        eprintln!(
            "layout: polished by a move trial, {:.0} s of {:.0} s of work left: polishing again",
            config.total_seconds - work,
            config.total_seconds
        );
    }
    arm(&mut router, work + pool_excess);
    if (!polished || polish_again)
        && !many_open
        && !router.past_deadline()
        && polish_left >= 240.0
        && stop.allows(LayoutStep::Polish)
    {
        let saved_router = router.clone();
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
                (config.total_seconds - work).max(0.0) + own,
            ));
            polish.cleanup_seconds = polish.cleanup_seconds.min((polish_left - 120.0) / 4.0);
        }
        let polished_result = router.polish();
        work += router.expansions().saturating_sub(expansions_before) as f64
            / core::router::expansions_per_second();
        // A polish the deadline cut short can leave more open than it
        // found (Sisu: 146 open before it, 255 after): then the board stays
        // as the moves left it.
        let (before, after) = (score(&result, &board), score(&polished_result, &board));
        if after.0 <= before.0 {
            result = polished_result;
        } else {
            eprintln!(
                "layout: the polish left {} open (before it {}): kept the unpolished routes",
                after.0, before.0
            );
            router = saved_router;
        }
    }
    // Reference labels off pads and other silkscreen; the board's own
    // texts and silkscreen graphics stay where the input put them
    // (`keep_board_marks`), and the parts on them are reported.
    let keep_marks = config.placer.keep_board_marks;
    // Fixed parts lay where they are in the input, marks and all (the
    // placement warned of them): only the parts placed are reported.
    let fixed: BTreeSet<String> = problem
        .references
        .iter()
        .zip(&problem.problem.components)
        .filter(|(_, component)| component.fixed)
        .map(|(reference, _)| reference.clone())
        .collect();
    let field_options = crate::silk_fields::FieldOptions {
        rules: crate::silk_fields::SilkRules::from_project(
            &source_directory.join(format!("{board_id}.kicad_pro")),
        ),
        shrink: true,
        hide_when_blocked: config.silkscreen.hide_when_blocked,
    };
    let mut labels = crate::labels::place_labels(
        &mut pcb,
        keep_marks,
        &fixed,
        &placement.attached_marks,
        field_options,
    )?;
    if !labels.stuck.is_empty() {
        eprintln!("labels without a free spot: {:?}", labels.stuck);
    }
    let layer_names = layers.names.clone();
    // Copper texts: kept, or else (decoration on copper) moved off the
    // copper and the parts.
    let copper_texts = crate::labels::place_copper_texts(
        &mut pcb,
        &result.routes,
        &layer_names,
        keep_marks,
        &fixed,
    )?;
    labels
        .overlaps
        .extend(copper_texts.overlaps.iter().cloned());
    for overlap in &labels.overlaps {
        eprintln!("board mark under parts: {overlap}");
    }
    if copper_texts.moved > 0 || !copper_texts.stuck.is_empty() {
        eprintln!(
            "copper texts: {} moved, {} without a free spot",
            copper_texts.moved,
            copper_texts.stuck.len()
        );
    }
    let nets = emit_routes(&mut pcb, &board, &result, &layer_names, false)?;
    let result_directory = output_directory.join("result");
    let mut routed = finish_routed_board(
        &pcb,
        &board,
        &result,
        nets,
        &layer_names,
        &placed_directory,
        board_id,
        &result_directory,
        router_config,
        [
            0.0,
            first_route_seconds + move_started.elapsed().as_secs_f64(),
        ],
    )?;
    routed.pours = if !has_pours {
        "none"
    } else if connect {
        "connect"
    } else {
        "tracks"
    }
    .into();
    // KiCad's check of the board kept counts as work (`check_work`).
    if routed.native.is_some() {
        work += checks_charged.add(1);
    }
    // If something is still open (in KiCad's eyes too), route the final
    // placement with route mode's whole ladder (pours as tracks, finer
    // pitches, more seeds) and keep the better board. If only thermals are
    // starved (pads KiCad does not count as joined to their pour), route
    // the pour nets as tracks.
    let open = |result: &KiCadBoardRouterResult| {
        result.unconnected_terminals
            + result.internal_violations.len()
            + result
                .native
                .as_ref()
                .map_or(0, |native| native.selected_net_unconnected_items)
    };
    let quality = |result: &KiCadBoardRouterResult, directory: &Path| {
        (
            open(result),
            crate::board_router::starved_thermals(directory),
        )
    };
    let current = quality(&routed, &result_directory);
    // Whether KiCad checked a ladder's board (or no check runs at all).
    let checked = |result: &KiCadBoardRouterResult| {
        result.native.is_some() || router_config.skip_native_verification
    };
    let layout_work = work;
    // The ladder below builds routers of its own (probes side by side,
    // seeds side by side): this one would only hold its memory meanwhile.
    drop(router);
    eprintln!("layout: routed ({:.0} s, work {work:.0} s)", elapsed());
    // Past the wall-clock guard the ladder would only lower the board and
    // ask KiCad again (link ended 106 s past it, 44 s before the
    // benchmark's limit).
    // A ladder needs a few minutes of wall clock for a probe, the board
    // and KiCad's check; started with less it runs past the harness's
    // limit (the laptop motherboard: killed at 1800 s without a board).
    let past_guard = work_deadline - work - pool_excess < 240.0 || !stop.allows(LayoutStep::Ladder);
    // A board far from complete where the narrow-signal step applies gets
    // only the step: one attempt with the first route's pours, signals at
    // the project's narrowest width (and its minimum clearance where the
    // project rates clearance findings as warnings), with most of what is
    // left. The class-width ladder before it used to take that work and
    // leave the step too little or nothing, and none of its attempts beat
    // the first route on these boards (x833: module 82 unconnected, DSP
    // 121, vaio_re 138, jetson 25 without a ladder; the step alone 17, 82,
    // 35, 18).
    let direct_narrow = narrow_applies && many_open && current.0 > 0;
    let fallback_config = if past_guard {
        None
    } else if direct_narrow {
        let ladder_left = work_deadline - work - pool_excess;
        let mut narrow = crate::board_router::narrow_step_config(router_config);
        narrow.pours = if connect {
            KiCadPourMode::Connect
        } else {
            KiCadPourMode::Tracks
        };
        narrow.refine_pitches_mm = Some(Vec::new());
        let (negotiation, cleanup) = direct_narrow_budget(ladder_left);
        narrow.negotiation_seconds = Some(negotiation);
        narrow.cleanup_seconds = Some(cleanup);
        eprintln!(
            "layout: {ladder_left:.0} s of work left: the narrow-signal attempt instead of the ladder (pours {}, negotiation {negotiation:.0} s, clean-up {cleanup:.0} s)",
            if connect { "connected" } else { "as tracks" },
        );
        Some(narrow)
    } else if current.0 > 0 {
        Some(router_config.clone())
    } else if current.1 > 0 && connect && router_config.pours == KiCadPourMode::Auto {
        let mut tracks = router_config.clone();
        tracks.pours = KiCadPourMode::Tracks;
        Some(tracks)
    } else {
        None
    };
    let final_ladder_budget_seconds = fallback_config
        .as_ref()
        .map(|_| (work_deadline - work - pool_excess).max(0.0));
    if let Some(fallback_config) = fallback_config {
        let fallback_directory = output_directory.join("result-ladder");
        // The placement file already carries the final poses.
        let placed_pcb = {
            let mut copy = pcb.clone();
            if let Expr::List(items) = &mut copy {
                items.retain(|item| !matches!(item.head(), Some("segment" | "arc" | "via")));
            }
            copy
        };
        fs::write(&placed_board, format!("{}\n", encode(&placed_pcb)))
            .map_err(|error| format!("failed to write {}: {error}", placed_board.display()))?;
        let mut fallback_config = fallback_config;
        let left = (config.total_seconds - work).max(120.0);
        // The work left, or less where the router config says so: until
        // 2026-10-21 at most route mode's default of 1200 s, so a layout
        // of 3000 s (the fence's two layers) ended its ladder with half
        // its budget unused.
        fallback_config.ladder_budget_seconds = Some(
            fallback_config
                .ladder_budget_seconds
                .unwrap_or(left)
                .min(left),
        );
        // The ladder decides whether another attempt fits in work, up to
        // the layout's nominal end.
        fallback_config.deadline_work = Some((work_deadline - work - pool_excess).max(0.0));
        // KiCad checks only the ladder's attempts that could beat the board
        // kept, at most the two best by the router's count, the board kept
        // among them (`CheckGate`).
        fallback_config.kicad_check_top = Some(KICAD_CHECK_TOP);
        // The starved pads go solid once, on the board the layout keeps
        // (below): its attempts are compared as KiCad found them.
        fallback_config.solid_starved_thermals = Some(false);
        fallback_config.kicad_kept = Some((
            (
                crate::board_router::router_open(&routed),
                routed.vias,
                routed.length_mm,
            ),
            current,
        ));
        let fallback = route_kicad_board(
            &placed_directory,
            board_id,
            &fallback_directory,
            &fallback_config,
        )?;
        // (Its checks are in its work already.)
        checks_charged.add(fallback.kicad_checks_charged);
        // The work of the narrow-signal attempt and what follows it counts
        // whether the layout keeps them or not.
        let mut after_work = fallback.work_seconds;
        let fallback_pours = fallback.pours.clone();
        let mut best_quality = current;
        let fallback_quality = quality(&fallback, &fallback_directory);
        // A ladder board KiCad did not check (past the deadline, it could
        // not beat the board kept) is never kept.
        if checked(&fallback) && fallback_quality < current {
            fs::remove_dir_all(&result_directory).map_err(|error| error.to_string())?;
            fs::rename(&fallback_directory, &result_directory)
                .map_err(|error| error.to_string())?;
            routed = fallback;
            best_quality = fallback_quality;
        } else {
            fs::remove_dir_all(&fallback_directory).map_err(|error| error.to_string())?;
        }
        // After the narrow-signal attempt run alone, the work it left (Hub
        // ended at 577 of 1500 s) goes to one more attempt when there is
        // enough of it: the attempt again in the other pour mode (as tracks
        // after connected, and the other way round; the only continuation
        // that gained: docs/changelog.md, "after the narrow attempt").
        if direct_narrow {
            let left = work_deadline - work - pool_excess - after_work;
            let wall = !stop.allows(LayoutStep::Continuation);
            let continuation_config = if narrow_continuation_runs(left, best_quality.0, wall) {
                let mut other = fallback_config.clone();
                other.pours = if fallback_pours == "tracks" {
                    KiCadPourMode::Connect
                } else {
                    KiCadPourMode::Tracks
                };
                let (negotiation, cleanup) = direct_narrow_budget(left);
                other.negotiation_seconds = Some(negotiation);
                other.cleanup_seconds = Some(cleanup);
                eprintln!(
                    "layout: {left:.0} s of work left after the narrow-signal attempt: it again with pours {} (negotiation {negotiation:.0} s, clean-up {cleanup:.0} s)",
                    if other.pours == KiCadPourMode::Tracks {
                        "as tracks"
                    } else {
                        "connected"
                    }
                );
                Some(other)
            } else {
                if best_quality.0 > 0 {
                    let why = if wall {
                        "the wall clock passed the layout's guard".to_string()
                    } else {
                        format!("{left:.0} s of work left, under {NARROW_CONTINUATION_FLOOR:.0} s")
                    };
                    eprintln!("layout: no attempt after the narrow-signal attempt: {why}");
                    routed.skipped.push(format!(
                        "the attempt after the narrow-signal attempt (the other pour mode) did not run: {why}"
                    ));
                }
                None
            };
            if let Some(mut continuation_config) = continuation_config {
                // The work left, as the ladder's (until 2026-10-21 at most
                // 1200 s), or less where the router config says so.
                continuation_config.ladder_budget_seconds = Some(
                    router_config
                        .ladder_budget_seconds
                        .map_or(left, |budget| budget.min(left))
                        .max(0.0),
                );
                continuation_config.deadline_work = Some(left.max(0.0));
                continuation_config.kicad_check_top = Some(KICAD_CHECK_TOP);
                continuation_config.solid_starved_thermals = Some(false);
                continuation_config.kicad_kept = Some((
                    (
                        crate::board_router::router_open(&routed),
                        routed.vias,
                        routed.length_mm,
                    ),
                    best_quality,
                ));
                let continuation_directory = output_directory.join("result-continuation");
                let continuation = route_kicad_board(
                    &placed_directory,
                    board_id,
                    &continuation_directory,
                    &continuation_config,
                )?;
                checks_charged.add(continuation.kicad_checks_charged);
                after_work += continuation.work_seconds;
                let continuation_quality = quality(&continuation, &continuation_directory);
                eprintln!(
                    "layout: the attempt after the narrow-signal attempt: {} open, {} starved, {} vias, work {:.0} s ({} open, {} starved before)",
                    continuation_quality.0,
                    continuation_quality.1,
                    continuation.vias,
                    continuation.work_seconds,
                    best_quality.0,
                    best_quality.1
                );
                if checked(&continuation) && continuation_quality < best_quality {
                    fs::remove_dir_all(&result_directory).map_err(|error| error.to_string())?;
                    fs::rename(&continuation_directory, &result_directory)
                        .map_err(|error| error.to_string())?;
                    routed = continuation;
                } else {
                    fs::remove_dir_all(&continuation_directory)
                        .map_err(|error| error.to_string())?;
                }
            }
        }
        // Work spent is work spent: the ladder's (and the attempts after
        // the narrow-signal attempt) count whether the layout keeps their
        // board or not (koeg in v16: 410 s reported, its unkept ladder ran
        // 1100 s more of wall clock).
        routed.work_seconds = after_work;
    }
    // The starved pads of the board kept, solid (`solid_starved_thermals`,
    // on unless false; kept only when KiCad finds the board better). Once,
    // here: on an attempt it made the layout prefer eternal-keypad's
    // continuation (its starved thermals gone, a short the attempts'
    // comparison does not weigh) to the board it kept before.
    let mut solidified = false;
    if router_config.solid_starved_thermals != Some(false)
        && routed.native.is_some()
        && stop.allows(LayoutStep::NeckRepair)
    {
        let checks = crate::board_router::solidify_starved_thermals(
            &mut routed,
            &output_directory.join("result"),
            board_id,
        )?;
        solidified = checks > 0;
        routed.work_seconds += checks_charged.add(checks);
    }
    // KiCad's connection_width necks on the board kept (a no-op when the
    // route mode ladder above has repaired it already). It improves the
    // board: past the deadline it is skipped, and its checks stay within
    // the layout's work left (`neck_check_bound`; they used to be charged
    // after it, past `total_seconds` by up to eight checks).
    if (routed.native.is_some() || router_config.skip_native_verification)
        && stop.allows(LayoutStep::NeckRepair)
    {
        let left = work_deadline - work - pool_excess - routed.work_seconds;
        let checks = crate::board_router::repair_necks(
            &mut routed,
            &output_directory.join("result"),
            board_id,
            router_config,
            neck_check_bound(left, check_seconds),
        )?;
        // Its checks count as work (after every decision: only the tally).
        routed.work_seconds += checks_charged.add(checks);
    }
    // How whole the pours stayed on the board kept (a report: after every
    // decision, counted as no work).
    routed.pour_quality = crate::board_router::measured_pours(
        &output_directory.join("result"),
        board_id,
        router_config,
    );
    if solidified
        || !routed.pour_quality.is_empty()
        || routed
            .neck_repair
            .as_ref()
            .is_some_and(|repair| repair.kept)
    {
        let report_path = output_directory.join("result").join("board-router.json");
        fs::write(
            &report_path,
            serde_json::to_string_pretty(&routed).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("failed to write {}: {error}", report_path.display()))?;
    }
    // The placement file should show the final poses too.
    {
        let mut placed_pcb = pcb.clone();
        if let Expr::List(items) = &mut placed_pcb {
            items.retain(|item| !matches!(item.head(), Some("segment" | "arc" | "via")));
        }
        fs::write(&placed_board, format!("{}\n", encode(&placed_pcb)))
            .map_err(|error| format!("failed to write {}: {error}", placed_board.display()))?;
    }
    let routed_work = routed.work_seconds;
    let kicad_check_work_seconds = checks_charged.work();
    let layout = KiCadBoardLayoutResult {
        board_id: board_id.into(),
        work_seconds: layout_work,
        kicad_check_seconds: check_seconds,
        kicad_checks_charged: checks_charged.checks,
        kicad_check_work_seconds,
        placement_seconds: placement.seconds,
        first_route_seconds,
        first_unconnected_terminals: first.0,
        first_vias: first.1,
        first_length_mm: first.2,
        final_ladder_budget_seconds,
        moves,
        outline_sizing,
        placement_race: race,
        congestion,
        labels,
        quality: crate::quality::score_kicad_board(&output_directory.join("result"), board_id).ok(),
        pin_swaps,
        constraints: constraint_report(
            &problem.problem,
            &problem.problem.poses,
            &problem.references,
        ),
        routed,
        cut: core::router::wall_cuts(),
    };
    let report_path = output_directory.join("board-layout.json");
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&layout).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", report_path.display()))?;
    let drc_cache = crate::drc_cache_counts();
    eprintln!(
        "layout: done ({:.0} s, work {work:.0} s, {:.0} s in all; work over wall {} at {:.4e} expansions/s, {:.4e} placer units/s, {}); KiCad checks {} ({} from the DRC cache, {} of the neck repair), {} skipped by the router's count; check work {} x {check_seconds:.1} s = {kicad_check_work_seconds:.0} s{}",
        elapsed(),
        work + routed_work,
        crate::verdict::work_wall_ratio(Some(work + routed_work), elapsed())
            .map_or("-".to_string(), |ratio| format!("{ratio:.2}")),
        core::router::expansions_per_second(),
        placer::work_per_second(),
        crate::work_rate::work_rate().source,
        drc_cache.calls,
        drc_cache.hits,
        crate::board_router::neck_checks(),
        crate::board_router::checks_skipped(),
        checks_charged.checks,
        if stop.fired() {
            format!(
                "; past the deadline: {}",
                stop.skipped()
                    .iter()
                    .map(|step| format!("{step:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            String::new()
        }
    );
    Ok(layout)
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    /// Runs a fake layout's steps in order with a guard that fires after
    /// `fires_after` looks at the clock; returns the steps that ran.
    fn run_steps(steps: &[LayoutStep], fires_after: usize) -> (Vec<LayoutStep>, Vec<LayoutStep>) {
        let looks = std::cell::Cell::new(0);
        let stop = WallStop::new(|| {
            looks.set(looks.get() + 1);
            looks.get() > fires_after
        });
        let ran = steps
            .iter()
            .copied()
            .filter(|step| stop.allows(*step))
            .collect();
        (ran, stop.skipped())
    }

    #[test]
    fn past_the_wall_guard_no_improving_step_runs() {
        use LayoutStep::*;
        let steps = [
            Race,
            Move,
            Move,
            Move,
            Resume,
            Polish,
            Ladder,
            Continuation,
            NeckRepair,
        ];
        // The guard never fires: everything runs.
        let (ran, skipped) = run_steps(&steps, usize::MAX);
        assert_eq!(ran, steps.to_vec());
        assert!(skipped.is_empty());
        // It fires during the second move trial: the third trial, the
        // resume, the polish, the ladder, the continuation and the neck
        // repair are skipped.
        let (ran, skipped) = run_steps(&steps, 2);
        assert_eq!(ran, vec![Race, Move]);
        assert_eq!(
            skipped,
            vec![Move, Resume, Polish, Ladder, Continuation, NeckRepair]
        );
        // Fired already in placement: no race either.
        let (ran, _) = run_steps(&steps, 0);
        assert!(ran.is_empty());
    }

    #[test]
    fn once_fired_the_guard_stays_fired() {
        use LayoutStep::*;
        let looks = std::cell::Cell::new(0);
        // A clock that would say "not passed" again after firing.
        let stop = WallStop::new(|| {
            looks.set(looks.get() + 1);
            looks.get() == 2
        });
        assert!(stop.allows(Move));
        assert!(!stop.allows(Move));
        assert!(!stop.allows(Ladder));
        assert!(stop.fired());
        assert!(!stop.allows(NeckRepair));
    }

    #[test]
    fn far_from_complete_is_two_percent_of_the_terminals_at_least_ten() {
        assert_eq!(far_open_limit(100), 10);
        assert_eq!(far_open_limit(653), 13);
        assert!(far_from_complete(30, 653));
        assert!(!far_from_complete(13, 653));
        assert!(!far_from_complete(10, 100));
        assert!(far_from_complete(11, 100));
        assert!(!far_from_complete(0, 0));
    }

    #[test]
    fn the_narrow_attempt_alone_negotiates_most_of_what_is_left() {
        // Jetson: about 600 s left.
        let close = |(a, b): (f64, f64), (c, d): (f64, f64)| {
            (a - c).abs() < 1.0e-9 && (b - d).abs() < 1.0e-9
        };
        assert!(close(direct_narrow_budget(600.0), (405.0, 60.0)));
        // Little left: the floors.
        assert!(close(direct_narrow_budget(200.0), (127.5, 30.0)));
        assert_eq!(direct_narrow_budget(90.0), (60.0, 30.0));
        // Much left: the caps, unchanged up to 1800 s.
        assert_eq!(direct_narrow_budget(1500.0), (900.0, 120.0));
        assert_eq!(direct_narrow_budget(1800.0), (900.0, 120.0));
        // More: they grow with it (half and a twentieth).
        assert!(close(direct_narrow_budget(2000.0), (1000.0, 120.0)));
        assert!(close(direct_narrow_budget(3000.0), (1500.0, 150.0)));
        assert!(close(direct_narrow_budget(6000.0), (3000.0, 300.0)));
        // Negotiation and clean-up never take more than is left.
        for left in [240.0, 400.0, 800.0, 1200.0, 2000.0, 3000.0, 6000.0] {
            let (negotiation, cleanup) = direct_narrow_budget(left);
            assert!(negotiation + cleanup < left);
        }
    }

    #[test]
    fn the_neck_repair_checks_stay_within_the_work_left() {
        assert_eq!(neck_check_bound(100.0, 20.0), Some(5));
        assert_eq!(neck_check_bound(59.0, 20.0), Some(2));
        assert_eq!(neck_check_bound(-10.0, 20.0), Some(0));
        assert_eq!(neck_check_bound(100.0, 0.0), None);
    }

    #[test]
    fn the_continuation_runs_only_with_work_left_and_connections_open() {
        // Hub: about 900 s left, 30 open.
        assert!(narrow_continuation_runs(900.0, 30, false));
        // At the floor it runs, under it not.
        assert!(narrow_continuation_runs(
            NARROW_CONTINUATION_FLOOR,
            30,
            false
        ));
        assert!(!narrow_continuation_runs(
            NARROW_CONTINUATION_FLOOR - 1.0,
            30,
            false
        ));
        // Module and jetson: under 200 s left.
        assert!(!narrow_continuation_runs(170.0, 17, false));
        // Complete, or past the wall-clock guard: nothing.
        assert!(!narrow_continuation_runs(900.0, 0, false));
        assert!(!narrow_continuation_runs(900.0, 30, true));
    }

    #[test]
    fn the_wall_guard_is_a_factor_of_the_budget() {
        // The default: twice the budget, a load margin.
        let config = KiCadBoardLayoutConfig::default();
        assert_eq!(config.wall_guard, 2.0);
        assert_eq!(
            wall_guard_seconds(config.total_seconds, config.wall_guard),
            Ok(3000.0)
        );
        assert_eq!(wall_guard_seconds(40.0, 1.1), Ok(44.0));
        assert_eq!(wall_guard_seconds(40.0, 1.0), Ok(40.0));
        // A negative budget guards nothing beyond its start.
        assert_eq!(wall_guard_seconds(-5.0, 2.0), Ok(0.0));
        // Below 1 the guard would cut every layout before its work ends.
        for bad in [0.9, 0.0, -1.0, f64::NAN, f64::INFINITY] {
            let error = wall_guard_seconds(40.0, bad).unwrap_err();
            assert!(crate::verdict::is_input_error(&error), "{error}");
        }
        // A layout config names it; the default fills it in.
        let config: KiCadBoardLayoutConfig =
            serde_json::from_str(r#"{"total_seconds": 40, "wall_guard": 3}"#).unwrap();
        assert_eq!(config.wall_guard, 3.0);
        let config: KiCadBoardLayoutConfig =
            serde_json::from_str(r#"{"total_seconds": 40}"#).unwrap();
        assert_eq!(config.wall_guard, DEFAULT_WALL_GUARD);
    }
}
