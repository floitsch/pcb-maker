// Copyright (C) 2026 Toit contributors.

//! The router's configuration: what a router config file holds and what
//! the layout sets per attempt.

use super::*;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadBoardRouterConfig {
    /// Wall-clock seconds from the start of the command by which it
    /// returns the best board it has: no attempt starts after it and a
    /// running one stops (an agent waits for the answer). Off by default:
    /// the budgets count work, so a busy machine routes the same way.
    #[serde(default)]
    pub deadline_seconds: Option<f64>,
    /// `deadline_seconds` as a time, set when routing starts (or by a
    /// caller, such as the layout, with a deadline of its own).
    #[serde(skip)]
    pub deadline: Option<std::time::Instant>,
    /// Resolved per-connection geometry, keyed by normalized net name.
    #[serde(default)]
    pub connection_rules: BTreeMap<String, KiCadConnectionRoutingRules>,
    /// Geometry for connections without an explicit entry.
    #[serde(default)]
    pub default_rules: Option<KiCadConnectionRoutingRules>,
    /// Copper-to-edge clearance from the board outline.
    #[serde(default)]
    pub edge_clearance_mm: f64,
    /// Clearance between copper and a hole.
    #[serde(default = "default_hole_clearance")]
    pub hole_clearance_mm: f64,
    /// Clearance between two holes.
    #[serde(default = "default_hole_clearance")]
    pub hole_to_hole_clearance_mm: f64,
    /// Router `via_cost`: a via's cost in millimetres of track (default 8).
    #[serde(default)]
    pub via_cost_mm: Option<f64>,
    /// Router `against_direction`: the cost factor of a step across a
    /// layer's preferred axis (default 1.6).
    #[serde(default)]
    pub against_direction_cost: Option<f64>,
    /// Router `max_iterations`: negotiation iterations at most (default 80).
    #[serde(default)]
    pub maximum_iterations: Option<usize>,
    /// Directory receiving the routing after every negotiation iteration
    /// as `attempt-NN/frame-NNNN.kicad_pcb` (one attempt per ladder rung),
    /// for animations and diagnostics.
    #[serde(default)]
    pub frame_directory: Option<PathBuf>,
    /// Router `pitches`: the candidate lattice pitches, the one aligning
    /// best with the pads wins (default 0.127, 0.125, 0.1).
    #[serde(default)]
    pub grid_pitches_mm: Option<Vec<f64>>,
    /// Above 1, negotiation searches trade optimality for speed; the
    /// cleanup pass always searches exactly.
    #[serde(default)]
    pub heuristic_weight: Option<f64>,
    /// Negotiation's starting price of shared lattice cells.
    #[serde(default)]
    pub present_factor: Option<f64>,
    /// Growth of that price per iteration.
    #[serde(default)]
    pub present_growth: Option<f64>,
    /// Iterations negotiation waits for fewer conflicts once the price is
    /// at its cap.
    #[serde(default)]
    pub stall_at_cap: Option<usize>,
    /// Router `stall_drop`: at the price cap, the share of conflicted nets
    /// that must go before negotiation counts it as progress.
    #[serde(default)]
    pub stall_drop: Option<f64>,
    /// Plan nets on the tile graph first (global routing).
    #[serde(default)]
    pub global_routing: Option<bool>,
    /// On a board with four or more copper layers and no copper pours,
    /// ground becomes a plane on the first inner layer and the supply rail
    /// with the most pads one on the last (default true; ignored when
    /// `add_pours` is given).
    #[serde(default)]
    pub automatic_planes: Option<bool>,
    /// A pad with a mask opening and no copper (a mask-only test point)
    /// opens the mask like a mask graphic: copper of a second net may not
    /// go under it (default true). hackrf-one's two solder mask bridges.
    #[serde(default)]
    pub mask_only_pad_openings: Option<bool>,
    /// Price of sharing when a via reduction round starts.
    #[serde(default)]
    pub via_reduction_present: Option<f64>,
    /// Keep other nets' tracks off inner-layer planes entirely.
    #[serde(default)]
    pub exclusive_planes: Option<bool>,
    /// With exclusive planes: how many of a pour net's inner plane layers
    /// stay exclusive, the ones nearest the outer layers first (the
    /// stackups designers use: one solid ground layer under the top on
    /// four layers, In1 and In4 on six; ESC mini's GND covers In1-In4 and
    /// keeping all four left the signals two layers). The ladder sets it
    /// from the layer count when not given; `None` outside the ladder
    /// keeps every covered inner layer.
    #[serde(default)]
    pub exclusive_layers: Option<usize>,
    /// Route pour-net pads the signals cut off from their pour during the
    /// negotiation (default on; see `pcb_router::Config::pour_islands`).
    #[serde(default)]
    pub pour_islands: Option<bool>,
    /// Thermal spokes as KiCad draws and counts them (default on; see
    /// `pcb_router::Config::kicad_spokes`).
    #[serde(default)]
    pub kicad_spokes: Option<bool>,
    /// Straight tracks from pads short of KiCad's two spokes into their
    /// pour (default on; see `pcb_router::Config::manual_spokes`).
    #[serde(default)]
    pub manual_spokes: Option<bool>,
    /// Reach of the neck width around narrow pads (default 1.5 mm; 0 off).
    #[serde(default)]
    pub neck_reach_mm: Option<f64>,
    /// Cost factor of a track (or via) cutting a plane (default 3).
    #[serde(default)]
    pub plane_cut_cost: Option<f64>,
    /// How much a two-layer pour's integrity counts (default 0: off;
    /// boards of other layer counts are not affected):
    /// the router prices a cut by what it severs of a pour (a corridor
    /// that joins the fill's pieces costs more than a nick in a wide
    /// area; `pcb_router::Config::pour_integrity`), also for pour nets
    /// routed as tracks, and the ladder ranks equally open attempts by
    /// the pours' integrity (`pours`: the share outside the largest piece
    /// and the piece count) before their vias and copper.
    #[serde(default)]
    pub pour_integrity: Option<f64>,
    /// Router `present_cap`: the highest price of sharing (default 1e4).
    #[serde(default)]
    pub present_cap: Option<f64>,
    /// Narrowest track the board allows (neck-downs out of small pads).
    #[serde(default)]
    pub neck_width_mm: Option<f64>,
    /// The board setup's minimum connection width (`rules.min_connection`)
    /// as a floor on every track width the router chooses: class widths,
    /// the narrow-signal width, neck-downs, stubs and spokes. Set from the
    /// project when KiCad's connection_width check reports at error
    /// severity, and at warning severity with
    /// `PCB_MIN_CONNECTION_FLOOR=warning` (`project_rules::
    /// connection_width_floor`, `apply_width_floor`); `None`: no floor.
    #[serde(default)]
    pub min_connection_mm: Option<f64>,
    /// The project's severity of connection_width findings (`error`,
    /// `warning`, `ignore`), for the rules summary.
    #[serde(default)]
    pub connection_width_severity: Option<String>,
    /// Router `cleanup_via_cost`: the via cost while cleaning up
    /// (default 25).
    #[serde(default)]
    pub cleanup_via_cost_mm: Option<f64>,
    /// Router `cleanup_passes`: passes of per-net shortening once the board
    /// is conflict free (default 4).
    #[serde(default)]
    pub cleanup_passes: Option<usize>,
    /// Router `bend_cost`: the cost per 45 degrees of direction change, in
    /// millimetres (default 0.1).
    #[serde(default)]
    pub bend_cost_mm: Option<f64>,
    /// Router `pin_row_cost`: the cost of another net's step between two
    /// adjacent SMD pads of a part's pin row, in millimetres (default
    /// `PCB_ROUTER_PIN_ROW_COST`, else 2; 0: off).
    #[serde(default)]
    pub pin_row_cost_mm: Option<f64>,
    /// Differential pairs (docs/router.md, "Differential pairs"): nets
    /// KiCad pairs by name (`_P`/`_N`, `+`/`-`, `P`/`N`) route coupled at
    /// the pair's width and gap: each link between the pair's parts is a
    /// net of the route at the pair's envelope, negotiated with the other
    /// nets and offset into the two nets' tracks when written; the pads'
    /// stubs and what the coupled runs leave open are ordinary routing.
    /// Route mode only. Default off; `PCB_ROUTER_DIFF_PAIRS=1` turns it on
    /// where the settings name none.
    #[serde(default)]
    pub diff_pairs: Option<bool>,
    /// The project's rules for the nets of differential pairs, keyed by
    /// normalized net name like `connection_rules` (set by the project's
    /// rules, not read from a file).
    #[serde(skip)]
    pub diff_pair_rules: BTreeMap<String, KiCadDiffPairRule>,
    /// Router `via_reduction_rounds`: via reduction rounds after the board
    /// is complete (default 3).
    #[serde(default)]
    pub via_reduction_rounds: Option<usize>,
    /// Router `strict_via_budget` (default on): the via reduction keeps
    /// its budget within a round.
    #[serde(default)]
    pub strict_via_budget: Option<bool>,
    /// The run's work budget and what is left of it as the attempt starts
    /// (router `run_work`; set by the ladder, not read from a file).
    #[serde(skip)]
    pub run_work: Option<(f64, f64)>,
    /// The work (seconds of an idle machine) left before the run's nominal
    /// end, as the ladder starts (set by the layout; none in route mode).
    /// The ladder's decisions of whether another attempt fits are taken in
    /// it, not in the wall clock, so that a busy machine decides the same
    /// way; the wall-clock deadline stays as the last guard.
    #[serde(skip)]
    pub deadline_work: Option<f64>,
    /// With `deadline_work`: the layout routes another attempt after this
    /// ladder (the narrow-signal attempt's continuation in the other pour
    /// mode), so the ladder keeps work for it as it keeps work for its own
    /// narrow-signal step (`narrow_step_reserve`, from its first attempt's
    /// work): its seed rounds see the work left without it (set by the
    /// layout, not read from a file).
    #[serde(skip)]
    pub keep_for_continuation: bool,
    /// The router's open count of a fresh first attempt (the layout's
    /// first route): a ladder routing the remainder of a kept router state
    /// stops after its first attempt when that leaves more open, and the
    /// layout routes from scratch instead (set by the layout, not read
    /// from a file).
    #[serde(skip)]
    pub fresh_open: Option<usize>,
    /// The work at which a router of this attempt stops as at the
    /// deadline (router `deadline_work`, its own work counted from zero;
    /// set by the ladder from `deadline_work`, not read from a file).
    #[serde(skip)]
    pub router_deadline_work: Option<f64>,
    /// The router's work deadline and negotiation limit count all its
    /// work (router `limits_count_all`: its search, pour analyses and
    /// build); set by route mode's ladder for its first attempt and the
    /// rung probes, not read from a file.
    #[serde(skip)]
    pub limits_count_all: bool,
    /// KiCad checks only the attempts among this many best by the
    /// router's count (`CheckGate`; set by the layout for its final
    /// ladder, not read from a file).
    #[serde(skip)]
    pub kicad_check_top: Option<usize>,
    /// The caller's kept board: the router's measures (open count, vias,
    /// copper) and its checked (open count, starved thermals); the
    /// ladder's attempts are ranked against it for KiCad's checks
    /// (`CheckGate`; set by the layout, not read from a file).
    #[serde(skip)]
    pub kicad_kept: Option<(RouterMeasure, (usize, usize))>,
    /// The work one KiCad check of an attempt counts, in seconds of an
    /// idle machine, modelled from the board alone (`check_work`): the
    /// ladder adds it to its work for every check it runs, cached or
    /// not, so that its end in work does not depend on the DRC cache.
    /// Set by the layout, not read from a file; none in route mode.
    #[serde(skip)]
    pub kicad_check_work: Option<f64>,
    /// How many seeds of one attempt route at once (memory; the number of
    /// seeds, and so the result, does not depend on it).
    #[serde(skip)]
    pub seeds_side_by_side: Option<usize>,
    /// A cap on the clean-up's work budget (router `cleanup_seconds`; set
    /// by the layout for a narrow-signal attempt run alone, whose
    /// negotiation needs the work more than its length and vias do).
    #[serde(skip)]
    pub cleanup_seconds: Option<f64>,
    /// Route every attempt this many ways at once (the deterministic order
    /// plus perturbed orders, in parallel) and keep the best: fewest open
    /// connections, then vias, then copper. `None`/1: one way.
    #[serde(default)]
    pub seeds: Option<usize>,
    /// The first of the `seeds` (0: the deterministic order).
    #[serde(default)]
    pub first_seed: Option<u64>,
    /// An attempt that leaves connections open is routed again this many
    /// perturbed ways at once (default 4; 0: never). Many failures depend
    /// on the order: Starling fails unseeded and completes for 5 of 8
    /// seeds.
    #[serde(default)]
    pub retry_seeds: Option<usize>,
    /// Route pour nets first as a fixed tree on their plane layer.
    #[serde(default)]
    pub plane_skeleton: Option<bool>,
    /// Router `skeleton_bias`: the cost factor of a skeleton step off the
    /// plane layer (default 6).
    #[serde(default)]
    pub skeleton_bias: Option<f64>,
    /// Finer lattice pitches tried, in order, when the routing with the
    /// regular pitch leaves connections open. `None` means 0.075 then 0.05.
    #[serde(default)]
    pub refine_pitches_mm: Option<Vec<f64>>,
    /// A finer lattice is only tried when the routing time projected from
    /// the previous attempt stays below this. Default: 240 s, a fifth of
    /// `ladder_budget_seconds` where that is more, and in a layout 0.4 of
    /// the work left before its end where that is more (a layout of 3000 s
    /// tries the finer lattice an attempt of up to about 900 s needs).
    /// Other ways of connecting the pours are tried whatever the time
    /// while connections are open.
    #[serde(default)]
    pub refine_budget_seconds: Option<f64>,
    /// No further attempt of any kind starts after the ladder has run this
    /// long (default 1200 s; in a layout the work left before its end,
    /// at most this where it is given): ColdFire from its schematic spent
    /// 50 min in three pour rungs with their seed retries.
    #[serde(default)]
    pub ladder_budget_seconds: Option<f64>,
    /// With a work deadline (the layout's ladder) and a project whose
    /// final narrow-signal or minimum-clearance step applies, this share
    /// of the ladder's first attempt's work is kept back for that step
    /// from the start (`narrow_step_reserve`; 0: none, the step gets what
    /// the rungs leave).
    #[serde(default)]
    pub narrow_reserve_share: Option<f64>,
    /// A limit on this process's memory in megabytes (resident), beside
    /// `PCB_MAKER_MEMORY_MB` and the limits the process can read itself
    /// (RLIMIT_AS, the cgroup's memory.max): the ladder runs as many
    /// routers side by side as fit the tightest (`MemoryLimits`). For a
    /// harness that kills a run above a resident size it does not set as
    /// a limit (the intake's 3 GiB).
    #[serde(default)]
    pub memory_mb: Option<f64>,
    /// The search expansions a second of work stands for, pinned instead
    /// of measured at the start (`work_rate` in a verdict: for reproducing
    /// a run on another machine with the rate it recorded; docs/router.md,
    /// "Determinism and the wall clock"). `PCB_WORK_RATE` wins over it.
    #[serde(default)]
    pub work_rate: Option<f64>,
    /// With `work_rate`: the placement units a second stands for (default:
    /// the reference's, scaled as `work_rate` scales the reference's search
    /// rate).
    #[serde(default)]
    pub placer_work_rate: Option<f64>,
    /// Seconds one negotiation may run (default 900 s). The ladder sets it
    /// per attempt from what is left of its budget, so that a hopeless
    /// first rung leaves time for the next (video's exclusive-plane rung
    /// ran 1180 s with 222 open, and no other rung ran).
    #[serde(default)]
    pub negotiation_seconds: Option<f64>,
    /// Large multilayer boards: probe every rung briefly and continue the
    /// best (default on).
    #[serde(default)]
    pub probe_ladder: Option<bool>,
    /// Seconds of negotiation per probe (default 75).
    #[serde(default)]
    pub probe_seconds: Option<f64>,
    /// Fixed escape stubs for fine-pitch pad rows, this long in mm (router
    /// `escape_stub_mm`; off by default).
    #[serde(default)]
    pub escape_stub_mm: Option<f64>,
    /// Connect every pour-net pad to its plane by a fixed stub and via
    /// before the signals route (router `fixed_plane_stubs`).
    #[serde(default)]
    pub fixed_plane_stubs: Option<bool>,
    /// The trouble-makers' repair after the hard repair (router
    /// `trouble_repair`; docs/experiments/human-level-routing.md):
    /// unset, the router's default; `PCB_ROUTER_TROUBLE` overrides this.
    #[serde(default)]
    pub trouble_repair: Option<bool>,
    /// Router `thermal_guard_cost`.
    #[serde(default)]
    pub thermal_guard_cost: Option<f64>,
    /// The project's smallest predefined track width (its width menu),
    /// which designers use for signals where the class width is a power
    /// width (ohdsp's DSP board: class 0.5 mm, 0.135 mm in the menu and on
    /// most tracks).
    #[serde(default)]
    pub narrow_signal_mm: Option<f64>,
    /// Route signal nets at `narrow_signal_mm` (the ladder tries it when
    /// connections stay open at the class widths).
    #[serde(default)]
    pub use_narrow_signals: Option<bool>,
    /// The board's minimum clearance when the project rates clearance
    /// findings below errors (a designer who accepted the net classes'
    /// clearances as warnings: the A13 module's DDR fan-out keeps 0.149
    /// against classes of 0.2). The ladder's last step routes at it when
    /// connections stay open.
    #[serde(default)]
    pub relaxed_clearance_mm: Option<f64>,
    /// The project's solder-mask-to-copper clearance (`rules.
    /// solder_mask_to_copper_clearance`, KiCad's default 0): KiCad's DRC
    /// finds a solder mask aperture bridging copper of another net within
    /// it plus the aperture's mask expansion (USB_Cable_Tester: a track
    /// 0.094 mm from a battery holder's F.Mask polygon over a GND fill, at
    /// 0.05 + 0.05 mm).
    #[serde(default)]
    pub solder_mask_to_copper_mm: Option<f64>,
    /// The project's severity of solder_mask_bridge findings (`error`,
    /// `warning`, `ignore`; KiCad's default is an error). Below an error
    /// the designer accepted bridges, and the router keeps its looser
    /// mask rules of before 2026-10-15 (OpenESC 4in1-mini, warnings: the
    /// exact rule took its bridges 29 to 2 and cost 52 to 61 unconnected).
    #[serde(default)]
    pub solder_mask_bridge_severity: Option<String>,
    /// Pads KiCad finds short of thermal spokes (where only one fits, as on
    /// fine-pitch pins, or whose spokes reach only an isolated island of
    /// fill) get a solid zone connection in the result, and the board is
    /// verified again (one more KiCad check, charged as work). On
    /// unless false (since 2026-10-21: four boards of the v18 sweep failed
    /// on starved thermals alone, which this passes; docs/changelog.md).
    /// False keeps the designer's thermal reliefs where KiCad finds them
    /// starved: reliefs are there for soldering.
    #[serde(default)]
    pub solid_starved_thermals: Option<bool>,
    /// Allow vias inside SMD pads (default false): a board whose vias
    /// are filled and capped (via-in-pad plating) may put them there. Off,
    /// every via keeps its annular ring out of every SMD pad but an
    /// exposed pad (a QFN's thermal pad, which holds thermal vias as
    /// designers place them): another net's vias keep their clearance
    /// beyond the pad's copper, the pad's own net's may touch it from
    /// outside. Open vias in a reflowed pad take its solder down the hole
    /// (score-kicad-board's "vias in pads" counts them). Through-hole pads
    /// hold no via either way.
    #[serde(default)]
    pub via_in_pad: Option<bool>,
    /// What route-kicad-board does with the board's own tracks and vias
    /// (`keep`, the default, or `strip`). Kept, they stay exactly as they
    /// are and other nets keep clear of them; pads they join count as
    /// connected, so the router routes only what they leave open and
    /// never rips one up (a designer's or an agent's pre-routed nets).
    /// `strip` (the command's `--strip`) removes them first: the board is
    /// routed from nothing. Copper zones stay either way. A layout strips
    /// them always (its placement moves the parts).
    #[serde(default)]
    pub existing_tracks: KiCadExistingTracks,
    /// Skip the final native KiCad verification (for timing the router).
    #[serde(default)]
    pub skip_native_verification: bool,
    /// Route with widths and clearances this file asks for below the
    /// project's (a net class's, `default_rules`', `connection_rules`',
    /// the neck and narrow-signal widths, `relaxed_clearance_mm`): without
    /// it such a file is refused before any work (exit 2) with each value,
    /// the project's minimum and its source; with it the run goes on and
    /// the verdict warns (`rules.overrides` in board-router.json). KiCad's
    /// DRC still holds the board to the project's rules.
    #[serde(default)]
    pub allow_below_project_minimum: bool,
    /// The routing engine: `lattice` (default, negotiated congestion on a
    /// lattice) or `topological` (TopoR-style: triangulation, crossings,
    /// layer assignment, any-angle tracks with arcs; pour nets as tracks).
    #[serde(default)]
    pub engine: Option<String>,
    /// Rip-up rounds of the topological engine (default 30).
    #[serde(default)]
    pub topological_rounds: Option<usize>,
    /// Weights of the topological engine (defaults in `pcb_topo::Config`).
    #[serde(default)]
    pub topological: KiCadTopologicalConfig,
    /// Pull the lattice router's tracks tight afterwards (any-angle tracks
    /// with arcs, as the topological engine draws them), wherever the
    /// result stays legal.
    #[serde(default)]
    pub tighten: bool,
    /// How nets with copper pours are connected.
    #[serde(default)]
    pub pours: KiCadPourMode,
    /// Copper pours to add before routing, such as a ground plane on the
    /// bottom: each covers the board outline on its layers.
    #[serde(default)]
    pub add_pours: Vec<KiCadPourRequest>,
    /// Net classes to add: track width, clearance and vias for named nets,
    /// ahead of the project's own classes and written into its
    /// `.kicad_pro`.
    #[serde(default)]
    pub net_classes: Vec<crate::net_classes::KiCadNetClassRequest>,
    /// Interchangeable pins (`pin-swaps.json`; relative to the source
    /// directory). The nets on them are permuted before routing, in the
    /// board and the schematic.
    #[serde(default)]
    pub pin_swaps: Option<PathBuf>,
    /// The pin-swap estimate's settings, for `pin_swaps`.
    #[serde(default)]
    pub pin_swap: KiCadPinSwapConfig,
    /// What the project's rules gave the router beyond its classes
    /// (`project_rules`; reported in `rules`, not read from a file).
    #[serde(skip)]
    pub project_rules: Option<crate::project_rules::KiCadProjectRulesReport>,
    /// Router `net_rules` (`project_rules`; not read from a file).
    #[serde(skip)]
    pub net_rules: Option<std::sync::Arc<pcb_router::NetRules>>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadPourMode {
    /// Connect through the pours; if the result is not clean, route the
    /// pour nets as tracks instead and keep the better board.
    #[default]
    Auto,
    /// Pads connect to their pour; only stubs, vias and stitching are added.
    Connect,
    /// Pour nets are routed like any other net; the pour merely fills.
    Tracks,
}

/// What route mode does with a board's own tracks and vias
/// (`existing_tracks`).
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadExistingTracks {
    /// Kept as they are: their connections count as made, and the router
    /// routes only what is open around them.
    #[default]
    Keep,
    /// Removed before routing: the board is routed from nothing.
    Strip,
}

/// Tuning of the topological engine; every field defaults to
/// `pcb_topo::Config`'s.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct KiCadTopologicalConfig {
    /// Millimetres one crossing costs while routing.
    #[serde(default)]
    pub crossing: Option<f64>,
    /// Millimetres a via costs while routing and assigning layers.
    #[serde(default)]
    pub via: Option<f64>,
    /// Millimetres a via costs while improving a legal board.
    #[serde(default)]
    pub improve_via: Option<f64>,
    /// Passes over all wires while improving a legal board.
    #[serde(default)]
    pub improve_passes: Option<usize>,
    /// Spacing of via sites in open areas.
    #[serde(default)]
    pub spacing: Option<f64>,
    /// Wall-clock budget of the rip-up rounds.
    #[serde(default)]
    pub seconds: Option<f64>,
    /// Route the first time layer by layer rather than TopoR's way.
    #[serde(default)]
    pub layered_start: Option<bool>,
    /// Factor on length across a layer's preferred axis (1: none).
    #[serde(default)]
    pub against: Option<f64>,
}

impl KiCadTopologicalConfig {
    pub fn apply(&self, config: &mut pcb_topo::Config) {
        if let Some(layered) = self.layered_start {
            config.layered_start = layered;
        }
        if let Some(against) = self.against {
            config.weights.against = against;
            config.costs.against = against;
        }
        if let Some(crossing) = self.crossing {
            config.weights.crossing = crossing;
        }
        if let Some(via) = self.via {
            config.weights.via = via;
            config.costs.via = via;
        }
        if let Some(via) = self.improve_via {
            config.improve_via = via;
        }
        if let Some(passes) = self.improve_passes {
            config.improve_passes = passes;
        }
        if let Some(spacing) = self.spacing {
            config.spacing = spacing;
        }
        if let Some(seconds) = self.seconds {
            config.seconds = seconds;
        }
    }
}

fn default_hole_clearance() -> f64 {
    0.25
}

/// What the project's rules give one net of a differential pair
/// (`project_rules`): per copper layer in the router's order, the pair's
/// track width and gap there, and the most uncoupled length.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KiCadDiffPairRule {
    /// The complementary net (normalized).
    pub partner: String,
    /// Per copper layer: (width, gap) in millimetres.
    pub layers: Vec<(f64, f64)>,
    /// Per copper layer: whether a custom rule disallows the pair's tracks
    /// there (`disallow track`).
    pub disallowed: Vec<bool>,
    /// Per copper layer: whether a custom rule sets the width (KiCad checks
    /// it: `track_width` on `inDiffPair()`), not the net class's diff pair
    /// width (which only KiCad's interactive router takes).
    pub width_ruled: Vec<bool>,
    /// The custom rules' `diff_pair_uncoupled` maximum, if any.
    pub max_uncoupled: Option<f64>,
    /// Where the width and gap came from, per layer ("rule 'name'", "net
    /// class 'name'", "net class clearance").
    pub sources: Vec<String>,
    /// The custom rules (names) whose diff_pair_gap or diff_pair_uncoupled
    /// set a value here.
    pub rules: Vec<String>,
}

impl KiCadBoardRouterConfig {
    /// Whether differential pairs route coupled (`diff_pairs`, else
    /// `PCB_ROUTER_DIFF_PAIRS`).
    pub fn diff_pairs_on(&self) -> bool {
        self.diff_pairs.unwrap_or_else(|| {
            std::env::var("PCB_ROUTER_DIFF_PAIRS")
                .is_ok_and(|value| !value.is_empty() && value != "0")
        })
    }

    /// The narrowest track width allowed by `min_connection_mm` (0: none).
    pub fn width_floor(&self) -> f64 {
        self.min_connection_mm
            .filter(|floor| *floor > 0.0)
            .unwrap_or(0.0)
    }

    /// Raises every track width of the configuration to `width_floor`:
    /// the classes, the narrow-signal width and the neck width.
    pub fn apply_width_floor(&mut self) {
        let floor = self.width_floor();
        if floor <= 0.0 {
            return;
        }
        for rules in self
            .connection_rules
            .values_mut()
            .chain(self.default_rules.iter_mut())
        {
            rules.trace_width_mm = rules.trace_width_mm.max(floor);
        }
        self.narrow_signal_mm = self.narrow_signal_mm.map(|width| width.max(floor));
        self.neck_width_mm = self.neck_width_mm.map(|width| width.max(floor));
    }

    /// Accepts either this module's schema or an adaptive-routing config,
    /// from which only the resolved physical rules are taken.
    pub fn from_json(value: serde_json::Value) -> Result<Self, String> {
        if let Some(entry) = value
            .get("sequential")
            .and_then(|sequential| sequential.get("routing_portfolio"))
            .and_then(|portfolio| portfolio.get(0))
        {
            let number = |key: &str| entry.get(key).and_then(serde_json::Value::as_f64);
            let default_rules = match (
                number("trace_width_mm"),
                number("clearance_mm"),
                number("via_size_mm"),
                number("via_drill_mm"),
            ) {
                (
                    Some(trace_width_mm),
                    Some(clearance_mm),
                    Some(via_size_mm),
                    Some(via_drill_mm),
                ) => Some(KiCadConnectionRoutingRules {
                    trace_width_mm,
                    clearance_mm,
                    via_size_mm,
                    via_drill_mm,
                }),
                _ => None,
            };
            return Ok(Self {
                connection_rules: entry
                    .get("connection_rules")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|error| format!("invalid connection_rules: {error}"))?
                    .unwrap_or_default(),
                default_rules,
                edge_clearance_mm: number("edge_clearance_mm").unwrap_or(0.0),
                hole_clearance_mm: default_hole_clearance(),
                hole_to_hole_clearance_mm: number("hole_to_hole_clearance_mm")
                    .unwrap_or_else(default_hole_clearance),
                ..Self::default()
            });
        }
        serde_json::from_value(value)
            .map_err(|error| format!("invalid board-router config: {error}"))
    }
}
