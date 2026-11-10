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
    LayerTable, add_pour_zones, core_config, emit_routes, finish_routed_board, lower_for_route,
    planned_pours, pours,
};
use crate::pin_swap::{KiCadPinSwapComponent, KiCadPinSwapGroup};
use crate::placement_constraints::{
    KiCadConstraintStatus, constraint_report, default_area_factor, resolve_constraints,
};
use pcb_placer as placer;
use pcb_router as core;

mod candidates;
mod ladder;
mod moves;
mod placement;
mod polish;
mod race;
mod report;
mod setup;
mod write;

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
    /// With a pre-ranking score, the placer makes at least this many
    /// placements to rank (its `placement_seeds` when that is more).
    pub congestion_candidates: usize,
    /// With a pre-ranking score, how many of the best-ranked placements
    /// the race routes (at least one).
    pub congestion_probes: usize,
    /// The placement race's pre-ranking (docs/placer.md, "The race"): the
    /// placer makes `congestion_candidates` placements (seeds), they are
    /// ranked by this score, and the race probes only the best
    /// `congestion_probes`:
    /// - `rudy` (default): RUDY routing demand above the tiles' free
    ///   capacity (quick tier: KiCad unconnected 98 -> 64 against the
    ///   plain race of three seeds);
    /// - `none`: the plain race (the placer's `placement_seeds`, all
    ///   probed);
    /// - `global`: the coarse global route's usage above half the tile
    ///   boundaries' capacity (`pcb-global`, docs/global-route.md; an
    ///   experiment).
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
    /// The move descent after the first route (an experiment, off when
    /// absent; docs/architecture.md, "Off by default, still in the code"): single parts nudged,
    /// turned or swapped where the incremental global route and then a
    /// local reroute say fewer terminals stay open, also on boards far
    /// from complete. `PCB_LAYOUT_MOVE_DESCENT` overrides (`0` off).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub move_descent: Option<crate::move_descent::KiCadMoveDescent>,
    /// The race's route probe (docs/placer.md, "The race's route probe";
    /// off when absent, `{}` for the defaults): the race
    /// probes its `seeds` best-ranked placements for `share` of
    /// `total_seconds` of work each, all of them charged to the layout's
    /// work clock and none dropped for the final route's minimum share
    /// (without it, a probe gets the router's `probe_seconds`, 75 s, and
    /// the race probes fewer placements where the final route would keep
    /// less than `RACE_MIN_FINAL_SHARE`), ranks them by the probe's open
    /// terminals (then its
    /// unfinished nets, then its nets in conflict) and routes on with
    /// the best. The pre-ranking (`congestion_score`) chooses the probed
    /// placements as before, `seeds` of them instead of
    /// `congestion_probes`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub race_probe: Option<KiCadRaceProbe>,
    /// A measurement mode (off by default; docs/benchmarks.md, "The
    /// ladder of placer measurements"): the layout ends after
    /// its first route. Sizing, placement, the race and the first route
    /// run as without it (the same work, the same budget), then the move
    /// trials, the resume, the polish, the final ladder and the neck repair
    /// are skipped and the layout finishes with the first route's board:
    /// its labels, KiCad's check and the report, whose
    /// `first_unconnected_terminals` is the full layout's at the same
    /// `total_seconds`. The report says so (`stopped_after_first_route`).
    pub stop_after_first_route: bool,
    /// What the race ranks its probes by (docs/placer.md, "The race ranks
    /// its probes by the ripped count"; `"ripped"` by default
    /// since the nine seed pairs of 2026-10-28): `"unfinished"` (the nets
    /// each probe left unfinished; with `race_probe`, its standing) or
    /// `"ripped"`: the board's open terminals once each probe's nets in
    /// conflict lose their conflicted branches (the hard repair's rip-up
    /// without its searches, on a copy of the probe's router: the kept
    /// one routes on unripped), ties by the standing. Without
    /// `race_probe` the pick then needs `RACE_RIPPED_MARGIN` over the
    /// first-ranked placement's count. Not with `race_probe.repair`.
    pub race_count: KiCadRaceCount,
}

/// What the race ranks its probes by (`KiCadBoardLayoutConfig::race_count`).
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KiCadRaceCount {
    /// The nets each probe left unfinished (with `race_probe`, its
    /// standing): the race's count until 2026-10-28.
    Unfinished,
    /// The open terminals after the rip-up of the conflicted branches,
    /// ties by the standing.
    #[default]
    Ripped,
}

/// The race's route probe (`KiCadBoardLayoutConfig::race_probe`).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadRaceProbe {
    /// How many placements the race probes (the best-ranked; at least
    /// one, default 3).
    pub seeds: usize,
    /// The work each probe negotiates, as a share of the layout's
    /// `total_seconds` (default 0.08: 24 % for three probes; the probe the
    /// race keeps is the first part of the first route).
    pub share: f64,
    /// Each probe is a whole first route within its share (half of it
    /// negotiating, then the first route's repair and its trouble rounds,
    /// no polish), ranked by the open terminals it leaves; the kept one's
    /// is the layout's first route (default false: the probe negotiates
    /// only, and the kept one routes on).
    pub repair: bool,
}

impl Default for KiCadRaceProbe {
    fn default() -> Self {
        Self {
            seeds: 3,
            share: 0.08,
            repair: false,
        }
    }
}

/// The share of `total_seconds` the race leaves the final route at least
/// (docs/placer.md, "The race"): where the work so far and the other
/// placements' probes would leave less, the race probes fewer of them.
/// At the dense boards' 1500 s, the placement's 40 % and two other
/// probes of 75 s leave 50 %: the race probes all three wherever the
/// placement overran its share by less than 75 s.
pub const RACE_MIN_FINAL_SHARE: f64 = 0.45;

/// The shortest probe the race shortens its probes to for the final
/// route's minimum share, as a share of `probe_seconds` (below it the
/// race probes fewer placements instead).
pub const RACE_SHORTEST_PROBE: f64 = 1.0 / 3.0;

/// How many of `others` placements besides the kept one the race probes,
/// and for how long each: as many as leave the final route
/// `RACE_MIN_FINAL_SHARE` of `total_seconds` after `clock` seconds of
/// work so far (the kept placement's probe is the first part of its
/// first route and does not count). All of them at `probe_seconds`
/// where they fit; else every probe shortened alike to share the room,
/// down to `RACE_SHORTEST_PROBE` of it; else fewer placements (the
/// lowest-ranked dropped first); none where not even one fits.
fn race_probes_that_fit(
    total_seconds: f64,
    clock: f64,
    probe_seconds: f64,
    others: usize,
) -> (usize, f64) {
    let room = (1.0 - RACE_MIN_FINAL_SHARE) * total_seconds - clock;
    if probe_seconds <= 0.0 || others == 0 {
        return (others, probe_seconds);
    }
    for count in (1..=others).rev() {
        let seconds = (room / count as f64).min(probe_seconds);
        if seconds >= RACE_SHORTEST_PROBE * probe_seconds {
            return (count, seconds);
        }
    }
    (0, probe_seconds)
}

/// The margin by which a placement's probe must beat the first-ranked
/// placement's (fewer nets unfinished, as a share of the first-ranked
/// one's) for the race to route on with it (`race_pick`).
pub const RACE_CONFIDENT_MARGIN: f64 = 0.05;

/// `RACE_CONFIDENT_MARGIN` for the ripped count (`race_count: "ripped"`:
/// fewer open terminals after the rip-up, as a share of the first-ranked
/// placement's). Calibrated on the coarse study's 73 placements
/// (docs/placer.md, "The race ranks its probes by the ripped count"
///): of the seed pairs whose counts differ by less than
/// 2 %, the lower count routed better in 8 and worse in 9; from 2 % to
/// 5 % in 23 against 13, from 5 % to 10 % in 38 against 12.
pub const RACE_RIPPED_MARGIN: f64 = 0.02;

/// The race's pick without the route probe: the probes' `best` (by
/// `counts`, unfinished nets or with `ripped` the open terminals after
/// the rip-up, in the ranking's order) where its count is at least the
/// margin (`RACE_CONFIDENT_MARGIN`, `RACE_RIPPED_MARGIN`) below the
/// first-ranked placement's, else the first-ranked one (the pre-route
/// ranking: the placer's key, RUDY and the crossings). Returns the
/// chosen index, whether the probes were confident, and why.
/// Calibrated on the 31 dense races of 2026-10-25 (docs/placer.md, "The
/// race counts every probe it ran"): video's seed 201 (504 against 529,
/// 4.7 %, a far worse route) reverts; 4in1's picks (33 % and 16 %) stand.
fn race_pick(
    counts: &[usize],
    best: usize,
    labels: &[usize],
    ripped: bool,
) -> (usize, bool, String) {
    let (what, required) = if ripped {
        ("open terminals after the rip-up", RACE_RIPPED_MARGIN)
    } else {
        ("nets unfinished", RACE_CONFIDENT_MARGIN)
    };
    if best == 0 {
        return (
            0,
            true,
            format!("the first-ranked placement left the fewest {what}"),
        );
    }
    let first = counts[0] as f64;
    let margin = if first > 0.0 {
        (first - counts[best] as f64) / first
    } else {
        0.0
    };
    if margin >= required {
        (
            best,
            true,
            format!(
                "placement {} left {:.1} % fewer {what} than the first-ranked ({} against {})",
                labels[best],
                margin * 100.0,
                counts[best],
                counts[0]
            ),
        )
    } else {
        (
            0,
            false,
            format!(
                "placement {} left only {:.1} % fewer {what} than the first-ranked ({} against {}, less than {:.0} %): the probes do not separate them, the first-ranked kept",
                labels[best],
                margin * 100.0,
                counts[best],
                counts[0],
                required * 100.0
            ),
        )
    }
}

/// A probe of the race: its index in the race, the nets it left
/// unfinished, its standing, its whole first route (with
/// `race_probe.repair`) and its router.
type RaceProbed = (
    usize,
    usize,
    core::router::ProbeStanding,
    Option<core::RoutingResult>,
    core::router::Router,
);

/// One placement the race probed: its index among the placements the
/// race ranked (0 the placer's kept one, then the other seeds by the
/// placer's key, as `congestion.scores`), the probe's standing and its
/// work, and whether the race before 2026-10-25 would have dropped it
/// (`gated`: judged past 40 % of the budget, the work of the probes it
/// counted before it included).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct KiCadRaceProbeScore {
    pub placement: usize,
    pub open_terminals: usize,
    pub unfinished: usize,
    pub conflicted: usize,
    pub work_seconds: f64,
    #[serde(default)]
    pub gated: bool,
    /// With `race_count: "ripped"`: the board's open terminals once the
    /// probe's nets in conflict lost their conflicted branches (what the
    /// race ranked by) and how many nets were ripped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ripped_open_terminals: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ripped_nets: Option<usize>,
}

/// What the ripped count cost (`race_count: "ripped"`), summed over the
/// probes: the wall clock of copying each probe's router and of its
/// rip-up and count (no search: no work is charged), and the largest
/// copy's resident growth.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct KiCadRaceRippedCost {
    pub copy_seconds: f64,
    pub rip_seconds: f64,
    pub copy_mb: f64,
}

/// What the race did (the verdict's `race`): every probed placement in
/// the ranking's order, the one kept, the one the 40 % gate before
/// 2026-10-25 would have kept (`kept_gated`) and how many placements the
/// final route's minimum share left unprobed (`dropped`).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct KiCadRaceReport {
    pub probe: Vec<KiCadRaceProbeScore>,
    pub kept: usize,
    /// The work each probe was given.
    pub probe_seconds: f64,
    /// Whether each probe was a whole first route (`repair`): its
    /// `open_terminals` are then the route's.
    pub repair: bool,
    #[serde(default)]
    pub kept_gated: usize,
    #[serde(default)]
    pub dropped: usize,
    /// The placement the probes ranked best (`kept` unless the race was
    /// not `confident`).
    #[serde(default)]
    pub best: usize,
    /// Whether the probes separated the placements (`race_pick`); when
    /// not, the race kept the first-ranked placement. `reason` says why.
    #[serde(default)]
    pub confident: bool,
    #[serde(default)]
    pub reason: String,
    /// With `race_count: "ripped"`: what its count cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ripped: Option<KiCadRaceRippedCost>,
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
            congestion_candidates: 16,
            congestion_probes: 3,
            congestion_score: "rudy".into(),
            free_dimension: None,
            silkscreen: KiCadSilkscreenConfig::default(),
            move_descent: None,
            race_probe: None,
            stop_after_first_route: false,
            race_count: KiCadRaceCount::default(),
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
    /// The work up to the end of the first route (sizing, placement and
    /// the first route), and the layout's budget (`total_seconds`): where
    /// the first route left too little for anything after it, no move and
    /// no final ladder ran (the verdict says so).
    pub first_route_work_seconds: f64,
    pub total_seconds: f64,
    /// The wall clock the layout waited in the machine's memory ledger for
    /// its reservations (`ledger::waited_seconds`), part of its wall clock
    /// and none of its work.
    pub ledger_wait_seconds: f64,
    pub first_unconnected_terminals: usize,
    pub first_vias: usize,
    pub first_length_mm: f64,
    /// The layout ended after its first route (`stop_after_first_route`):
    /// the steps after it did not run. Absent otherwise.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stopped_after_first_route: bool,
    /// The digest of what the steps after the placement read from it
    /// (`placement_digest`): two layouts of one binary, task, layout
    /// settings (beside the placer's) and rates with the same digest route
    /// alike (run.py --reuse).
    pub placement_digest: String,
    /// The work the final ladder (or the narrow-signal attempt standing in
    /// for it) had: `total_seconds` less the work so far and the seed
    /// pool's excess; `None` where it did not run. A route of the
    /// designer's placement at this budget gets the routing work this
    /// layout's last router got.
    pub final_ladder_budget_seconds: Option<f64>,
    /// Whether the layout's routes had lattice lines at the pin rows from
    /// the kept placement's probe on, and why, in one line (`pin_lines`
    /// unset: the race's decision, `race_lines_probe`); absent where they
    /// did not (the final ladder's own decision is in `routed`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_lines: Option<String>,
    pub moves: Vec<KiCadBoardLayoutMove>,
    /// What the move descent did (`move_descent`; absent when it did not
    /// run).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub move_descent: Option<crate::move_descent::KiCadMoveDescentReport>,
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
    /// The race: every placement it probed, the one kept (absent when the
    /// race did not run).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub race: Option<KiCadRaceReport>,
    /// Pads without a way out (`pad_exits`, absent when off) of the
    /// placement routed (the race's pick; placed/board-placer.json's are
    /// the placer's first-ranked one's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad_exits: Option<crate::board_placer::KiCadPadExitsReport>,
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
    /// Each placement's ratsnest crossings, where they rank the seeds
    /// beside the score (`ratsnest_crossings.rank_seeds`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crossings: Option<Vec<u64>>,
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
            let (board, plan_work) = lower_for_route(
                &without_copper_texts(&pcb, placer_config.keep_board_marks),
                router_config,
                connect,
            )?;
            *work += plan_work;
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
        let first = grid.index_of(axis, low).ceil().max(0.0) as usize;
        let last =
            (grid.index_of(axis, high).floor().max(0.0) as usize).min(size.saturating_sub(1));
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

/// The narrow-signal attempt run alone with `work_left`: its negotiation
/// and clean-up, and whether it keeps the attempt after it (the other
/// pour mode) its floor (`NARROW_CONTINUATION_FLOOR`): with at least
/// twice the floor left the attempt is sized from what is left without
/// it, and its ladder keeps it from the start (Glasgow: the attempt's
/// first route took 664 s of 701 s and the continuation never ran).
pub(crate) fn narrow_with_continuation(work_left: f64) -> (f64, f64, bool) {
    if work_left >= 2.0 * NARROW_CONTINUATION_FLOOR {
        let (negotiation, cleanup) = direct_narrow_budget(work_left - NARROW_CONTINUATION_FLOOR);
        (negotiation, cleanup, true)
    } else {
        let (negotiation, cleanup) = direct_narrow_budget(work_left);
        (negotiation, cleanup, false)
    }
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
    /// Set by `end_after_first_route` (`stop_after_first_route`): every
    /// step but the race is skipped from then on, by the option, not by
    /// the wall clock (no cut is noted).
    ended: std::cell::Cell<bool>,
    ended_skipped: std::cell::RefCell<Vec<LayoutStep>>,
}

impl<F: Fn() -> bool> WallStop<F> {
    pub(crate) fn new(passed: F) -> Self {
        Self {
            passed,
            fired: std::cell::Cell::new(false),
            skipped: std::cell::RefCell::new(Vec::new()),
            ended: std::cell::Cell::new(false),
            ended_skipped: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// The layout ends after its first route (`stop_after_first_route`):
    /// from now on `allows` refuses every step but the race.
    pub(crate) fn end_after_first_route(&self) {
        self.ended.set(true);
    }

    /// The steps the end after the first route skipped, in order.
    pub(crate) fn ended_skipped(&self) -> Vec<LayoutStep> {
        self.ended_skipped.borrow().clone()
    }

    /// Whether `step` runs: until the wall clock has passed the deadline.
    pub(crate) fn allows(&self, step: LayoutStep) -> bool {
        if self.ended.get() && step != LayoutStep::Race {
            let mut skipped = self.ended_skipped.borrow_mut();
            if !skipped.contains(&step) {
                skipped.push(step);
                eprintln!("layout: stop_after_first_route: {} skipped", step.name());
            }
            return false;
        }
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

/// The digest of what a layout's steps after its placement read from the
/// placement phase (`layout: placement digest` in the log,
/// `placement_digest` in the report; docs/benchmarks.md, "The ladder of
/// placer measurements"): the placed board (with its pours
/// and pin swaps), the lowered placement problem (its parts, nets, poses
/// and the placer settings it carries, the first placement's relaxation
/// applied), the other placements the race may probe and their
/// relaxations, the placer settings the later steps read beside the
/// problem (the crossings' seed ranking, the guided moves, the board's
/// marks), the pin swaps, and the work the placement booked (the later
/// steps' budgets are what is left of the layout's). Two layouts of one
/// binary, task, layout settings and rates whose digests are equal route
/// alike: everything after the placement is a function of these and of
/// the settings outside the placer's.
fn placement_digest(
    board_text: &str,
    problem: &crate::board_placer::LoweredPlacement,
    others: &[(&Vec<placer::Pose>, &placer::Relaxation)],
    placer_config: &KiCadBoardPlacerConfig,
    pin_swaps: Option<&crate::pin_swap::KiCadPinSwapResult>,
    work: f64,
    pool_excess: f64,
) -> String {
    use sha2::{Digest, Sha256};
    fn part(hash: &mut Sha256, label: &str, text: &str) {
        hash.update(label.as_bytes());
        hash.update((text.len() as u64).to_le_bytes());
        hash.update(text.as_bytes());
    }
    let mut hash = Sha256::new();
    part(&mut hash, "board", board_text);
    part(&mut hash, "problem", &format!("{:?}", problem.problem));
    part(
        &mut hash,
        "lowered",
        &format!(
            "{:?} {:?} {:?} {:?} {:?} {:?} {:?}",
            problem.net_names,
            problem.references,
            problem.source_at,
            problem.routing_layers,
            problem.poured,
            problem.attached_marks,
            problem.attached_report
        ),
    );
    for (poses, relaxation) in others {
        part(&mut hash, "other", &format!("{poses:?} {relaxation:?}"));
    }
    part(
        &mut hash,
        "placer",
        &format!(
            "{:?} {:?} {:?} {:?} {:?}",
            placer_config
                .ratsnest_crossings
                .as_ref()
                .map(|crossings| crossings.rank_seeds),
            placer_config
                .global_guidance
                .as_ref()
                .map(|guidance| (guidance.moves, guidance.threshold)),
            placer_config.keep_board_marks,
            placer_config.tight_bodies,
            placer_config.attached_marks,
        ),
    );
    part(
        &mut hash,
        "pin_swaps",
        &pin_swaps
            .map(|swaps| serde_json::to_string(swaps).unwrap_or_default())
            .unwrap_or_default(),
    );
    part(
        &mut hash,
        "work",
        &format!("{} {}", work.to_bits(), pool_excess.to_bits()),
    );
    format!("{:x}", hash.finalize())
}

/// The placement's share of a layout's budget: 40 % of `total_seconds`
/// (the race after it leaves the final route `RACE_MIN_FINAL_SHARE` at
/// least).
pub const PLACEMENT_SHARE: f64 = 0.4;

pub fn placement_budget(config: &KiCadBoardLayoutConfig) -> f64 {
    PLACEMENT_SHARE * config.total_seconds.max(0.0)
}

/// The placer's configuration as a layout gives it, before the outline is
/// sized: the layout config's `placer` with its constraints resolved, the
/// router's clearances, edge margin and wall-clock deadline, the
/// placement's share of the budget and the router configuration the
/// escape demand's measurements and the global guidance lower with.
/// `place-kicad-board` places with the same (`placement_check_config`),
/// so that its yes is the layout's (the OpenESC panel: the check placed
/// every part on the escape demand's estimate where the layout, which
/// measured it, left 8 without a place).
pub fn layout_placer_config(
    config: &KiCadBoardLayoutConfig,
    router_config: &KiCadBoardRouterConfig,
    source_directory: &Path,
) -> Result<KiCadBoardPlacerConfig, String> {
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
    // The placement's budget is its work; its wall-clock guard is the
    // layout's own deadline (at the placement's share of it, the placement
    // stopped on a busy machine where the work would have gone on, and the
    // layout depended on the load).
    placer_config.deadline = router_config.deadline;
    placer_config.work_seconds = Some(placement_budget(config));
    if placer_config.global_guidance.is_some()
        || placer_config.escape_demand.is_some()
        || crate::board_placer::pad_exits_settings(&placer_config).is_some()
    {
        placer_config.guidance_router = Some(router_config.clone());
    }
    // The router's rules the placer keeps (the far side under vias in
    // pads, length maxima and minima), only where one applies.
    if (placer_config.via_in_pad_far_side
        && (crate::board_router::admits_vias_in_pads(router_config)
            || crate::board_router::bga_fanout_on(router_config)))
        || (placer_config.length_maxima
            && crate::board_router::length_bounded_nets(router_config).is_some())
        || (placer_config.length_minima
            && crate::board_router::length_floor_nets(router_config).is_some())
        || placer_config.bga_room
    {
        placer_config.router_rules = Some(std::sync::Arc::new(router_config.clone()));
    }
    // The pair rows as the placer config sets them, else following the
    // router's pairs where `PAIR_ROWS_FOLLOW_DIFF_PAIRS` (off since its
    // measurement: docs/placer.md, "Following diff_pairs"), with the
    // pairs' widths and gaps for their pitch.
    placer_config.pair_rows = Some(placer_config.pair_rows_for(
        crate::board_router::pairs_planned(router_config),
        crate::board_placer::PAIR_ROWS_FOLLOW_DIFF_PAIRS,
    ));
    if placer_config.pair_rows_on() && placer_config.pair_geometry.is_empty() {
        placer_config.pair_geometry = crate::board_placer::pair_geometry(router_config);
    }
    Ok(placer_config)
}

/// The router's settings for the race, the move trials and the first
/// route (`core_config` with a shorter patience): the in-place router
/// compares placements, so it stops negotiating once conflicts no longer
/// really fall (a move on OpenAirScope negotiated 64 iterations at 15
/// conflicted nets, 200-350 s a move). The final ladder routes the kept
/// placement with the full patience.
pub(crate) fn race_core_config(router_config: &KiCadBoardRouterConfig) -> core::Config {
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
    // A trial only ranks placements: below the price cap, eight
    // iterations without fewer conflicted nets end it (OpenAirScope's
    // moves negotiated ~34 iterations, 126-229 s each).
    core_config.stall_patience = core_config.stall_patience.min(8);
    core_config
}

/// Whether the layout's race pre-ranks the placer's seeds (by RUDY or the
/// global route): only with one router seed and no pin swaps.
fn pre_ranks(config: &KiCadBoardLayoutConfig, router_config: &KiCadBoardRouterConfig) -> bool {
    (config.congestion_score == "rudy" || config.congestion_score == "global")
        && router_config.seeds.unwrap_or(1) <= 1
        && config.pin_swaps.is_none()
        && config.swappable.is_empty()
}

/// The placer's seeds where the race pre-ranks them: as many as it ranks
/// (`congestion_candidates`) when that is more.
fn race_placement_seeds(
    config: &KiCadBoardLayoutConfig,
    placer_config: &KiCadBoardPlacerConfig,
) -> usize {
    placer_config
        .placement_seeds
        .max(config.congestion_candidates)
}

/// The placer's configuration of `place-kicad-board`: what the layout of
/// that layout config and router configuration places with
/// (`layout_placer_config`, the net classes' rules, the race's seeds),
/// for a board whose outline is given (a layout sizing an automatic
/// outline places within each size it tries).
pub fn placement_check_config(
    source_directory: &Path,
    board_id: &str,
    config: &KiCadBoardLayoutConfig,
    router_config: &KiCadBoardRouterConfig,
) -> Result<KiCadBoardPlacerConfig, String> {
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
    let mut placer_config = layout_placer_config(config, router_config, source_directory)?;
    if pre_ranks(config, router_config) {
        placer_config.placement_seeds = race_placement_seeds(config, &placer_config);
    }
    Ok(placer_config)
}

/// Whether the layout's race decides the lines at the pin rows: the
/// router config's `pin_lines` unset and no `PCB_ROUTER_PIN_LINES` (true
/// puts them on every route of the layout, as before; false on none).
pub(crate) fn race_lines_automatic(config: &KiCadBoardRouterConfig) -> bool {
    config.pin_lines.is_none() && core::pin_lines::lines_override().is_none()
}

/// The lines at the pin rows decided at the kept placement's probe
/// (docs/router.md, "A layout's rows decided by its probe's open pads"):
/// where the race's probe of the placement it keeps left nets unfinished
/// (`unfinished`) at pads of pin rows whose straight exit the regular
/// lattice misses (`pin_lines::missed_exit_pads`, against
/// `Router::unfinished_pads` within `OPEN_PAD_NEAR`), and a probe's work
/// (`probe_seconds`) is at most `LINES_PROBE_SHARE` of the layout's work
/// left (`left`), the first route starts over on the lattice with the
/// lines: a router built with them probes as the race did (its count in
/// the log beside the probe's without), and the first route resumes it;
/// every route of the layout after it has the lines. Unlike route mode's
/// probes, the count of that probe does not decide: on RP2040_base's
/// layout the probe with lines left 82 nets unfinished against 75
/// without, and the first route with them ended at 11 open terminals
/// against 40 (`PCB_LAYOUT_LINES_PROBE=lead` keeps them only where the
/// probe with them leaves a tenth fewer, `lines_lead`). Returns the work
/// charged (that probe's search, as the race charges its probes; none
/// where it did not run) and, where the lines are taken, the router and
/// why. A placement whose probe leaves no pad at such a row routes as
/// without it: nothing is built and nothing charged (the pads' lists read
/// the board and the router's state).
fn race_lines_probe(
    board: &core::Board,
    config: &core::Config,
    kept: &core::router::Router,
    unfinished: usize,
    probe_seconds: f64,
    left: f64,
    router_mb: f64,
) -> (f64, Option<(core::router::Router, String)>) {
    if unfinished == 0 {
        return (0.0, None);
    }
    let missed = core::pin_lines::missed_exit_pads(board, &config.pitches);
    if missed.is_empty() {
        return (0.0, None);
    }
    let open = kept.unfinished_pads(crate::board_router::OPEN_PAD_NEAR);
    let at = crate::board_router::pads_at_rows(&open, &missed);
    let verdict = crate::board_router::lines_probe_runs(
        true,
        unfinished,
        at.len(),
        probe_seconds,
        Some(left),
    );
    eprintln!(
        "placement race: the kept placement's probe left {unfinished} unfinished, {} pad(s) of them at pin rows the regular lattice misses ({} missed in {}){}",
        at.len(),
        missed.len(),
        crate::board_router::parts_of(&missed).join(", "),
        match verdict {
            Ok(()) => format!(
                ": the first route starts over with lines at the pin rows ({probe_seconds:.0} s of work probed again, {left:.0} s left)"
            ),
            Err(why) => format!(": no lines ({why})"),
        }
    );
    if verdict.is_err() {
        return (0.0, None);
    }
    let mut lines_config = config.clone();
    lines_config.pin_lines = core::pin_lines::Lines::Exits;
    crate::ledger::reserve_at_least(
        crate::board_router::resident_mb() + router_mb,
        "the race's probe with lines at the pin rows",
    );
    let mut router = core::router::Router::new(board, &lines_config);
    let lines_unfinished = router.probe(probe_seconds);
    let work = router.expansions() as f64 / core::router::expansions_per_second();
    let lead_only = crate::board_router::layout_lines_probes().lead;
    let taken = !lead_only || crate::board_router::lines_lead(unfinished, lines_unfinished);
    let (without, with) = (kept.probe_standing(), router.probe_standing());
    eprintln!(
        "placement race: the kept placement's probe with lines at the pin rows: {lines_unfinished} nets unfinished, {unfinished} without (work {work:.0} s; open terminals {} against {}, in conflict {} against {}): {}",
        with.open_terminals,
        without.open_terminals,
        with.conflicted,
        without.conflicted,
        if taken {
            "the first route and every route after it with the lines".to_string()
        } else {
            format!(
                "the probe without lines leads (PCB_LAYOUT_LINES_PROBE=lead: the lines only at {:.0} % fewer)",
                crate::board_router::LINES_PROBE_MARGIN * 100.0
            )
        }
    );
    let parts =
        crate::board_router::parts_of(&at.iter().map(|pad| (*pad).clone()).collect::<Vec<_>>());
    (
        work,
        taken.then(|| {
            (
                router,
                format!(
                    "decided at the kept placement's probe: it left {unfinished} nets unfinished with {} pad(s) at pin rows the regular lattice misses ({}), so the first route started over with lines at those rows (its probe {lines_unfinished} nets unfinished) and every route after it had them",
                    at.len(),
                    parts.join(", ")
                ),
            )
        }),
    )
}

/// Lays the board out in `output_directory`: places it, routes it once
/// (the placement race over the seeds' placements, or the kept placement
/// alone), moves footprints where nets fought, finishes the kept routes,
/// writes and checks the board, runs the final ladder while connections
/// stay open, and writes `board-layout.json`. Each stage is a function of
/// its own (`setup`, `placement`, `candidates`, `race`, `moves`, `polish`,
/// `write`, `ladder`, `report`); this is their order, and what one hands
/// the next.
pub fn layout_kicad_board(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardLayoutConfig,
    router_config: &KiCadBoardRouterConfig,
) -> Result<KiCadBoardLayoutResult, String> {
    let (mut ctx, mut ledger) = setup::prepare(
        source_directory,
        board_id,
        output_directory,
        config,
        router_config,
    )?;
    let placement::Placed {
        placement,
        mut placer_config,
        outline_sizing,
    } = placement::place(&ctx, &mut ledger)?;
    let pin_swaps = placement::swap_pins(&ctx)?;
    let no_pin_swaps = pin_swaps.is_none();
    let placement::PlacedBoard {
        pcb,
        problem,
        first_relaxation,
        copper,
    } = placement::prepare_placed_board(&ctx, &placement, &mut placer_config)?;
    let (core_config, board, first_started) =
        race::lower_first_route(&ctx, &mut ledger, &pcb, &copper)?;
    let mut kept = candidates::Kept {
        pcb,
        board,
        problem,
        placer_config,
    };
    let mut candidates = candidates::race_candidates(&placement, &kept.problem.problem.poses);
    let placement_digest = candidates::digest(&kept, &candidates, pin_swaps.as_ref(), &ledger);
    candidates::pre_rank(
        &ctx,
        &mut ledger,
        &mut kept,
        &mut candidates,
        first_relaxation,
        &core_config,
        &copper,
        no_pin_swaps,
    )?;
    candidates::limit_race(&ctx, &ledger, &mut candidates, no_pin_swaps);
    let (router, result, race) = race::race_and_route(
        &ctx,
        &mut ledger,
        &mut kept,
        candidates,
        &core_config,
        &copper,
        no_pin_swaps,
    )?;
    // Decided at the race: every route after it (the moves' in place, the
    // final ladder's attempts) has the lines.
    if race.lines_why.is_some() {
        ctx.router_config.pin_lines = Some(true);
    }
    let best = score(&result, &kept.board);
    let first_route = report::FirstRoute::measure(&result, best.0, first_started, &ledger);
    let mut routing = moves::Routing {
        router,
        result,
        best,
    };

    if config.stop_after_first_route {
        eprintln!(
            "layout: {} open after the first route; stop_after_first_route: the layout finishes with the first route's board",
            routing.best.0
        );
        ctx.stop.end_after_first_route();
    }
    crate::mem_profile::phase("moves");
    let move_started = std::time::Instant::now();
    let move_work_started = ledger.work;
    let movable = moves::movable_parts(&kept.problem);
    let judgement = moves::judge_first_route(&ctx, &kept.board, &copper, routing.best.0);
    let move_descent = moves::descend(&ctx, &mut ledger, &mut kept, &mut routing, &copper)?;
    let trials = moves::try_moves(
        &ctx,
        &mut ledger,
        &mut kept,
        &mut routing,
        &copper,
        &movable,
        judgement.many_open,
        move_work_started,
    )?;
    polish::resume_first_route(
        &ctx,
        &mut ledger,
        &mut routing,
        &kept.board,
        judgement.many_open,
    );
    polish::polish_routes(
        &ctx,
        &mut ledger,
        &mut routing,
        &kept.board,
        trials.polished,
        judgement.many_open,
    );
    let (routed, labels) = write::write_kept_board(
        &ctx,
        &mut ledger,
        &mut kept,
        &mut routing.result,
        &copper,
        &placement.attached_marks,
        first_route.seconds,
        move_started,
    )?;
    let current = ladder::quality(&routed, &ctx.output_directory.join("result"));
    let layout_work = ledger.work;
    // The ladder below builds routers of its own (probes side by side,
    // seeds side by side): this one would only hold its memory meanwhile.
    drop(routing.router);
    crate::mem_profile::phase("layout ladder");
    eprintln!(
        "layout: routed ({:.0} s, work {:.0} s)",
        ctx.elapsed(),
        ledger.work
    );
    let (routed, final_ladder_budget_seconds) = ladder::final_steps(
        &ctx,
        &mut ledger,
        &kept.pcb,
        routed,
        current,
        &judgement,
        &copper,
    )?;
    // The placement file should show the final poses too.
    placement::write_placement_without_copper(&ctx, &kept.pcb)?;
    report::finish_report(
        &ctx,
        &ledger,
        report::ReportParts {
            placement: &placement,
            kept: &kept,
            outline_sizing,
            pin_swaps,
            placement_digest,
            race,
            first_route,
            moves: trials.moves,
            move_descent,
            labels,
            layout_work,
            final_ladder_budget_seconds,
            routed,
        },
    )
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
    fn after_the_first_route_only_the_race_still_runs() {
        use LayoutStep::*;
        // The guard never fires: the end after the first route alone
        // skips the improving steps, without a wall cut.
        let stop = WallStop::new(|| false);
        assert!(stop.allows(Race));
        assert!(stop.allows(Move));
        stop.end_after_first_route();
        for step in [Move, Resume, Polish, Ladder, Continuation, NeckRepair, Move] {
            assert!(!stop.allows(step));
        }
        assert!(stop.allows(Race));
        assert!(!stop.fired());
        assert!(stop.skipped().is_empty());
        assert_eq!(
            stop.ended_skipped(),
            vec![Move, Resume, Polish, Ladder, Continuation, NeckRepair]
        );
    }

    #[test]
    fn the_layout_config_reads_stop_after_first_route() {
        let config: KiCadBoardLayoutConfig =
            serde_json::from_str(r#"{"stop_after_first_route": true}"#).unwrap();
        assert!(config.stop_after_first_route);
        // Off by default.
        assert!(!KiCadBoardLayoutConfig::default().stop_after_first_route);
        let config: KiCadBoardLayoutConfig = serde_json::from_str("{}").unwrap();
        assert!(!config.stop_after_first_route);
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
    fn the_narrow_attempt_alone_leaves_the_continuation_its_floor() {
        let (negotiation, cleanup, keeps) = narrow_with_continuation(701.0);
        assert!(keeps);
        assert_eq!((negotiation, cleanup), direct_narrow_budget(701.0 - 240.0));
        assert!(negotiation + cleanup < 701.0 - 240.0);
        let (negotiation, cleanup, keeps) = narrow_with_continuation(400.0);
        assert!(!keeps);
        assert_eq!((negotiation, cleanup), direct_narrow_budget(400.0));
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

    #[test]
    fn the_race_probes_fewer_placements_only_where_the_final_route_would_keep_less_than_its_share()
    {
        // The dense boards' budget: placement at its 40 %, two others fit.
        assert_eq!(race_probes_that_fit(1500.0, 600.0, 75.0, 2), (2, 75.0));
        // Overrun by 76 s: 825 - 676 = 149 s, two probes of 74.5 s.
        let (count, seconds) = race_probes_that_fit(1500.0, 676.0, 75.0, 2);
        assert_eq!(count, 2);
        assert!((seconds - 74.5).abs() < 1.0e-9, "{seconds}");
        // The short tier's 500 s, placement at 200 s: two of 37.5 s.
        let (count, seconds) = race_probes_that_fit(500.0, 200.0, 75.0, 2);
        assert_eq!(count, 2);
        assert!((seconds - 37.5).abs() < 1.0e-9, "{seconds}");
        // Shorter than a third: fewer placements (40 s for one).
        let (count, seconds) = race_probes_that_fit(500.0, 235.0, 75.0, 2);
        assert_eq!(count, 1);
        assert!((seconds - 40.0).abs() < 1.0e-9, "{seconds}");
        // Not even one of 25 s: none.
        assert_eq!(race_probes_that_fit(1500.0, 810.0, 75.0, 2).0, 0);
        // Never more than there are.
        assert_eq!(race_probes_that_fit(1500.0, 0.0, 75.0, 2), (2, 75.0));
        assert_eq!(race_probes_that_fit(1500.0, 0.0, 0.0, 2), (2, 0.0));
    }

    #[test]
    fn the_race_keeps_the_first_ranked_placement_unless_a_probe_beats_it_by_the_margin() {
        let labels = [0, 1, 2];
        // video seed 201: 504 against 529 (4.7 %).
        let (chosen, confident, _) = race_pick(&[529, 504], 1, &labels, false);
        assert_eq!((chosen, confident), (0, false));
        // 4in1 seed 201: 185 against 220 (16 %).
        let (chosen, confident, _) = race_pick(&[220, 189, 185], 2, &labels, false);
        assert_eq!((chosen, confident), (2, true));
        // The first-ranked best.
        let (chosen, confident, _) = race_pick(&[164, 199], 0, &labels, false);
        assert_eq!((chosen, confident), (0, true));
        // Exactly the margin counts.
        assert_eq!(race_pick(&[100, 95], 1, &labels, false).0, 1);
        // The ripped count's margin: 2 %.
        assert_eq!(race_pick(&[1000, 981], 1, &labels, true).0, 0);
        assert_eq!(race_pick(&[1000, 980], 1, &labels, true).0, 1);
        let (chosen, confident, reason) = race_pick(&[1532, 1453], 1, &labels, true);
        assert_eq!((chosen, confident), (1, true));
        assert!(reason.contains("after the rip-up"), "{reason}");
    }

    #[test]
    fn the_race_probe_is_off_unless_named_and_an_empty_object_means_its_defaults() {
        let config: KiCadBoardLayoutConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.race_probe, None);
        let config: KiCadBoardLayoutConfig = serde_json::from_str(r#"{"race_probe": {}}"#).unwrap();
        assert_eq!(
            config.race_probe,
            Some(KiCadRaceProbe {
                seeds: 3,
                share: 0.08,
                repair: false
            })
        );
        let config: KiCadBoardLayoutConfig =
            serde_json::from_str(r#"{"race_probe": {"seeds": 2, "share": 0.1}}"#).unwrap();
        assert_eq!(
            config.race_probe,
            Some(KiCadRaceProbe {
                seeds: 2,
                share: 0.1,
                repair: false
            })
        );
        assert!(
            serde_json::from_str::<KiCadBoardLayoutConfig>(r#"{"race_probe": {"k": 2}}"#).is_err()
        );
        // Off, the config serializes as before.
        let text = serde_json::to_string(&KiCadBoardLayoutConfig::default()).unwrap();
        assert!(!text.contains("race_probe"), "{text}");
        assert!(text.contains(r#""race_count":"ripped""#), "{text}");
    }

    #[test]
    fn the_race_counts_ripped_terminals_unless_the_unfinished_nets_are_named() {
        let config: KiCadBoardLayoutConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.race_count, KiCadRaceCount::Ripped);
        let config: KiCadBoardLayoutConfig =
            serde_json::from_str(r#"{"race_count": "unfinished"}"#).unwrap();
        assert_eq!(config.race_count, KiCadRaceCount::Unfinished);
        assert!(
            serde_json::to_string(&config)
                .unwrap()
                .contains(r#""race_count":"unfinished""#)
        );
        assert!(
            serde_json::from_str::<KiCadBoardLayoutConfig>(r#"{"race_count": "open"}"#).is_err()
        );
    }

    /// A USB pair from J1 to U2 (diff_pairs.rs's test board without U1).
    const PAIR_BOARD: &str = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (general (thickness 1.6)) (paper "A4")
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
  (gr_rect (start 0 0) (end 40 20) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))
  (footprint "test" (layer "F.Cu") (at 3 10)
    (property "Reference" "J1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 34 10)
    (property "Reference" "U2" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D-"))))
"#;

    /// Lays `PAIR_BOARD` out (its parts fixed, so that the placement is
    /// the input's) with `diff_pairs` as given, the first route alone (no
    /// move, no final ladder: the board kept is the layout's own).
    fn pair_layout(diff_pairs: Option<bool>) -> KiCadBoardLayoutResult {
        pair_layout_with(diff_pairs, true)
    }

    /// `pair_layout`, its parts fixed or free (then the placer's three
    /// seeds race).
    fn pair_layout_with(diff_pairs: Option<bool>, fixed: bool) -> KiCadBoardLayoutResult {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "pcb-maker-layout-pairs-{}-{nonce}-{diff_pairs:?}",
            std::process::id()
        ));
        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("board.kicad_pcb"), PAIR_BOARD).unwrap();
        let config = KiCadBoardLayoutConfig {
            total_seconds: 30.0,
            stop_after_first_route: true,
            placer: KiCadBoardPlacerConfig {
                fixed_patterns: if fixed { vec!["*".into()] } else { Vec::new() },
                placement_seeds: if fixed { 1 } else { 3 },
                ..KiCadBoardPlacerConfig::default()
            },
            congestion_score: "none".into(),
            // Free parts: the route probe races the seeds' placements.
            race_probe: (!fixed).then_some(KiCadRaceProbe {
                seeds: 3,
                share: 0.1,
                repair: false,
            }),
            ..KiCadBoardLayoutConfig::default()
        };
        let router = KiCadBoardRouterConfig {
            default_rules: Some(crate::KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            diff_pairs,
            skip_native_verification: true,
            ..KiCadBoardRouterConfig::default()
        };
        let result = layout_kicad_board(&source, "board", &root.join("out"), &config, &router);
        let written = fs::read_to_string(root.join("out/result/board.kicad_pcb"));
        fs::remove_dir_all(&root).ok();
        let result = result.unwrap();
        // The board written has the pair's copper: tracks of both nets.
        let written = written.unwrap();
        for net in ["/D+", "/D-"] {
            assert!(
                written.contains(&format!("(net \"{net}\")")) || written.contains(net),
                "{net}"
            );
        }
        result
    }

    /// With `diff_pairs` the layout's own first route plans the pairs and
    /// the board it writes finishes them (`finish_pairs`): the pair is
    /// coupled without any final ladder, and the report (the verdict's
    /// `diff_pairs`) says so with the setting; off, as before, nothing.
    #[test]
    fn a_layouts_own_first_route_couples_the_pairs() {
        let on = pair_layout(Some(true));
        assert!(on.final_ladder_budget_seconds.is_none());
        assert!(on.moves.is_empty());
        let pairs = on.routed.diff_pairs.as_ref().expect("the pairs' report");
        assert!(pairs.on);
        assert_eq!(pairs.default, "explicit");
        assert_eq!(pairs.found, 1);
        assert_eq!(pairs.links, 1);
        assert_eq!((pairs.coupled, pairs.fallen_back), (1, 0), "{pairs:?}");
        assert!(pairs.coupled_mm > 20.0, "{pairs:?}");
        assert_eq!(on.routed.unconnected_terminals, 0);
        let off = pair_layout(None);
        assert!(off.routed.diff_pairs.is_none());
        assert_eq!(off.routed.unconnected_terminals, 0);
    }

    /// The placement race's probes route boards with the pairs' plan too:
    /// whichever placement it keeps, the board written finishes its pairs.
    #[test]
    fn the_placement_races_probes_plan_the_pairs() {
        let raced = pair_layout_with(Some(true), false);
        let race = raced.race.as_ref().expect("the race ran");
        assert!(race.probe.len() >= 2, "{race:?}");
        let pairs = raced.routed.diff_pairs.as_ref().expect("the pairs' report");
        assert_eq!((pairs.found, pairs.links), (1, 1), "{pairs:?}");
    }

    /// `pair_rows` unset: following the router's pairs (on where the router
    /// plans them, `diff_pairs`, off otherwise) where the switch says so,
    /// and the layout's switch is off (measured 2026-11-04): unset it is
    /// off with the pairs on too; the placer config's true or false wins
    /// either way.
    #[test]
    fn pair_rows_follow_the_routers_diff_pairs_only_where_the_switch_says() {
        let unset = KiCadBoardPlacerConfig::default();
        assert!(unset.pair_rows_for(true, true));
        assert!(!unset.pair_rows_for(false, true));
        assert!(!unset.pair_rows_for(true, false));
        let set = |on: bool| KiCadBoardPlacerConfig {
            pair_rows: Some(on),
            ..KiCadBoardPlacerConfig::default()
        };
        for (planned, follows) in [(false, false), (true, false), (false, true), (true, true)] {
            assert!(set(true).pair_rows_for(planned, follows));
            assert!(!set(false).pair_rows_for(planned, follows));
        }
        let f1 = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/f1");
        let router = crate::project_rules::resolve_project_rules(
            &f1.join("f1.kicad_pro"),
            &f1.join("f1.kicad_pcb"),
        )
        .unwrap();
        let paired = KiCadBoardRouterConfig {
            diff_pairs: Some(true),
            ..router.clone()
        };
        let layout = |pair_rows: Option<bool>| KiCadBoardLayoutConfig {
            total_seconds: 30.0,
            placer: KiCadBoardPlacerConfig {
                pair_rows,
                ..KiCadBoardPlacerConfig::default()
            },
            ..Default::default()
        };
        let rows = |pair_rows: Option<bool>, router: &KiCadBoardRouterConfig| {
            layout_placer_config(&layout(pair_rows), router, &f1)
                .unwrap()
                .pair_rows
        };
        // The layout's: off unset, with the pairs on or off.
        assert_eq!(rows(None, &router), Some(false));
        assert_eq!(rows(None, &paired), Some(false));
        assert_eq!(rows(Some(false), &paired), Some(false));
        assert_eq!(rows(Some(true), &router), Some(true));
        assert_eq!(rows(Some(true), &paired), Some(true));
        // The place check places as the layout does.
        let check = placement_check_config(&f1, "f1", &layout(None), &paired).unwrap();
        assert_eq!(check.pair_rows, Some(false));
        let check = placement_check_config(&f1, "f1", &layout(Some(true)), &paired).unwrap();
        assert_eq!(check.pair_rows, Some(true));
        // Unset, it reads and writes as before.
        let config: KiCadBoardPlacerConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.pair_rows, None);
        assert!(!config.pair_rows_on());
        let config: KiCadBoardPlacerConfig =
            serde_json::from_str(r#"{"pair_rows": true}"#).unwrap();
        assert_eq!(config.pair_rows, Some(true));
    }

    #[test]
    fn the_place_check_places_with_the_layouts_placer_config() {
        // The OpenESC panel's place check placed every part on the escape
        // demand's estimate and three seeds where the layout measured it
        // with its router and ranked sixteen.
        let f1 = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/f1");
        let router = crate::project_rules::resolve_project_rules(
            &f1.join("f1.kicad_pro"),
            &f1.join("f1.kicad_pcb"),
        )
        .unwrap();
        let layout = KiCadBoardLayoutConfig {
            total_seconds: 30.0,
            ..Default::default()
        };
        let check = placement_check_config(&f1, "f1", &layout, &router).unwrap();
        assert_eq!(check.work_seconds, Some(12.0));
        assert_eq!(check.placement_seeds, layout.congestion_candidates);
        assert!(check.guidance_router.is_some());
        assert_eq!(check.copper_clearance_mm, Some(largest_clearance(&router)));
        assert_eq!(
            check.copper_edge_clearance_mm,
            Some(router.edge_clearance_mm)
        );
        let own = layout_placer_config(&layout, &router, &f1).unwrap();
        assert_eq!(own.work_seconds, check.work_seconds);
        // The plain race places the placer's own seeds.
        let plain = KiCadBoardLayoutConfig {
            congestion_score: "none".into(),
            ..layout.clone()
        };
        let check = placement_check_config(&f1, "f1", &plain, &router).unwrap();
        assert_eq!(check.placement_seeds, plain.placer.placement_seeds);
    }
}
