// Copyright (C) 2026 Toit contributors.

//! Whole-board routing through the in-memory `pcb-router` core.
//!
//! The board is parsed once, lowered once, routed entirely in memory, checked
//! by the core's exact verifier, and written once. Native KiCad verification
//! is a single final gate, never part of the routing loop.

use super::*;
use pcb_router as core;

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
    #[serde(default)]
    pub edge_clearance_mm: f64,
    #[serde(default = "default_hole_clearance")]
    pub hole_clearance_mm: f64,
    #[serde(default = "default_hole_clearance")]
    pub hole_to_hole_clearance_mm: f64,
    #[serde(default)]
    pub via_cost_mm: Option<f64>,
    #[serde(default)]
    pub against_direction_cost: Option<f64>,
    #[serde(default)]
    pub maximum_iterations: Option<usize>,
    /// Directory receiving the routing after every negotiation iteration
    /// as `attempt-NN/frame-NNNN.kicad_pcb` (one attempt per ladder rung),
    /// for animations and diagnostics.
    #[serde(default)]
    pub frame_directory: Option<PathBuf>,
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
    /// with the most pads one on the second (default true; ignored when
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
    #[serde(default)]
    pub cleanup_via_cost_mm: Option<f64>,
    #[serde(default)]
    pub cleanup_passes: Option<usize>,
    #[serde(default)]
    pub bend_cost_mm: Option<f64>,
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
    /// The work at which a router of this attempt stops as at the
    /// deadline (router `deadline_work`, its own work counted from zero;
    /// set by the ladder from `deadline_work`, not read from a file).
    #[serde(skip)]
    pub router_deadline_work: Option<f64>,
    /// How many seeds of one attempt route at once (memory; the number of
    /// seeds, and so the result, does not depend on it).
    #[serde(skip)]
    pub seeds_side_by_side: Option<usize>,
    /// A cap on the clean-up's work budget (router `cleanup_seconds`; set
    /// by the layout for a narrow-signal attempt run alone, whose
    /// negotiation needs the work more than its length and vias do).
    #[serde(skip)]
    pub cleanup_seconds: Option<f64>,
    #[serde(default)]
    pub jacobi_batch: Option<usize>,
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
    #[serde(default)]
    pub skeleton_bias: Option<f64>,
    /// Finer lattice pitches tried, in order, when the routing with the
    /// regular pitch leaves connections open. `None` means 0.075 then 0.05.
    #[serde(default)]
    pub refine_pitches_mm: Option<Vec<f64>>,
    /// A finer lattice is only tried when the routing time projected from
    /// the previous attempt stays below this (default 240 s). Other ways
    /// of connecting the pours are tried whatever the time while
    /// connections are open.
    #[serde(default)]
    pub refine_budget_seconds: Option<f64>,
    /// No further attempt of any kind starts after the ladder has run this
    /// long (default 1200 s): ColdFire from its schematic spent 50 min in
    /// three pour rungs with their seed retries.
    #[serde(default)]
    pub ladder_budget_seconds: Option<f64>,
    /// With a work deadline (the layout's ladder) and a project whose
    /// final narrow-signal or minimum-clearance step applies, this share
    /// of the ladder's first attempt's work is kept back for that step
    /// from the start (`narrow_step_reserve`; 0: none, the step gets what
    /// the rungs leave).
    #[serde(default)]
    pub narrow_reserve_share: Option<f64>,
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
    /// Pads KiCad finds short of thermal spokes (where only one fits, as on
    /// fine-pitch pins) get a solid zone connection in the result, and the
    /// board is verified again. Off by default: thermal reliefs are there
    /// for soldering, so this is the designer's (or the agent's) call.
    #[serde(default)]
    pub solid_starved_thermals: Option<bool>,
    /// Skip the final native KiCad verification (for timing the router).
    #[serde(default)]
    pub skip_native_verification: bool,
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
    #[serde(default)]
    pub pin_swap: KiCadPinSwapConfig,
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
    #[serde(default)]
    pub improve_passes: Option<usize>,
    /// Spacing of via sites in open areas.
    #[serde(default)]
    pub spacing: Option<f64>,
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

impl KiCadBoardRouterConfig {
    /// The narrowest track width allowed by `min_connection_mm` (0: none).
    pub fn width_floor(&self) -> f64 {
        self.min_connection_mm.filter(|floor| *floor > 0.0).unwrap_or(0.0)
    }

    /// Raises every track width of the configuration to `width_floor`:
    /// the classes, the narrow-signal width and the neck width.
    pub fn apply_width_floor(&mut self) {
        let floor = self.width_floor();
        if floor <= 0.0 {
            return;
        }
        for rules in self.connection_rules.values_mut().chain(self.default_rules.iter_mut()) {
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

#[derive(Clone, Debug, Serialize)]
pub struct KiCadBoardRouterNet {
    pub connection: String,
    pub terminals: usize,
    pub status: String,
    pub unconnected_terminals: usize,
    pub vias: usize,
    pub length_mm: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadBoardRouterViolation {
    pub connection: String,
    pub other: String,
    pub layer: String,
    pub at: [f64; 2],
    pub required_mm: f64,
    pub actual_mm: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadBoardRouterResult {
    pub board_id: String,
    pub routable_connections: usize,
    pub routed_connections: usize,
    pub unconnected_terminals: usize,
    pub vias: usize,
    pub length_mm: f64,
    pub grid_pitch_mm: f64,
    pub grid_nodes: [usize; 2],
    pub iterations: usize,
    pub searches: u64,
    pub expansions: u64,
    pub lowering_seconds: f64,
    pub routing_seconds: f64,
    pub internal_verification_seconds: f64,
    pub native_verification_seconds: f64,
    pub internal_violations: Vec<KiCadBoardRouterViolation>,
    pub native: Option<VerificationReport>,
    /// `connect`, `tracks`, or `none` when the board has no pours.
    pub pours: String,
    pub nets: Vec<KiCadBoardRouterNet>,
    /// Congestion history on the routing lattice (row major), for placement
    /// feedback. Not serialized.
    #[serde(skip)]
    pub congestion: Vec<f32>,
    #[serde(skip)]
    pub grid_origin: [f64; 2],
    /// The pin assignment chosen before routing, if pins were swappable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_swaps: Option<KiCadPinSwapResult>,
    /// What stood in the way, when something did.
    pub diagnostics: KiCadRoutingDiagnostics,
    /// What the wall clock cut short (`pcb_router::router::WALL_CUTS`),
    /// rather than the work: a result that depends on the machine's load.
    /// Empty when the work decided everything, and the same input and
    /// thread count then give the same board.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cut: Vec<String>,
    /// The routing work of this run (the attempt ladder's), in seconds of
    /// an idle machine: what the budgets count.
    #[serde(default)]
    pub work_seconds: f64,
    /// Steps of the attempt ladder that did not run while connections
    /// were still open, each with why (`narrow-signal step did not run:
    /// the ladder's budget was used`). Empty on a complete board.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<String>,
    /// KiCad's connection_width necks and the tracks laid over them
    /// (`necks`), when its DRC reported any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub neck_repair: Option<crate::necks::NeckRepair>,
    /// The width rules the router routed with.
    pub rules: KiCadRulesSummary,
}

/// The track width rules of a routing, for whoever reads the report.
#[derive(Clone, Debug, Default, Serialize)]
pub struct KiCadRulesSummary {
    /// The minimum connection width applied as a floor on every track
    /// width (`KiCadBoardRouterConfig::min_connection_mm`); absent when
    /// none was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_floor_mm: Option<f64>,
    /// The project's severity of KiCad's connection_width findings, when
    /// its board setup has a minimum connection width.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_width_severity: Option<String>,
    /// The narrowest track width of the nets' classes as routed.
    pub narrowest_track_mm: f64,
    /// The neck-down width out of small pads.
    pub neck_width_mm: f64,
}

impl KiCadRulesSummary {
    fn of(board: &core::Board, config: &KiCadBoardRouterConfig) -> Self {
        let narrowest = board
            .nets
            .iter()
            .map(|net| board.classes[net.class].trace_width)
            .fold(f64::INFINITY, f64::min);
        Self {
            width_floor_mm: config.min_connection_mm.filter(|floor| *floor > 0.0),
            connection_width_severity: config.connection_width_severity.clone(),
            narrowest_track_mm: if narrowest.is_finite() { narrowest } else { 0.0 },
            neck_width_mm: board.neck_width,
        }
    }
}

/// Why a routing is not complete, for whoever has to fix the board.
#[derive(Clone, Debug, Default, Serialize)]
pub struct KiCadRoutingDiagnostics {
    /// Pads no track of their net's class can enter: too narrow a gap to
    /// their neighbours for the class's width and clearance, or covered by
    /// another object. `near` lists the objects around the pad.
    pub dead_pads: Vec<KiCadDeadPad>,
    /// Nets still competing for room when negotiation stopped.
    pub conflicted_nets: Vec<String>,
    /// Where nets fought longest (16 x 16-node tiles), most contested
    /// first: the places to give more room.
    pub hot_spots: Vec<KiCadHotSpot>,
    /// Pads KiCad finds short of thermal spokes (its starved_thermal),
    /// with what to do about each.
    pub starved_thermals: Vec<KiCadStarvedThermal>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadStarvedThermal {
    /// KiCad's name for the pad (`Pad 4 [GND] of U201`).
    pub pad: String,
    pub layer: String,
    pub at: [f64; 2],
    /// Spokes reaching the fill and spokes the zone needs, where KiCad
    /// says.
    pub spokes: Option<usize>,
    pub needed: Option<usize>,
    pub advice: String,
}

/// The starved thermals of a native DRC report, each with advice.
fn starved_thermal_diagnostics(directory: &Path) -> Vec<KiCadStarvedThermal> {
    let Ok(text) = fs::read_to_string(directory.join("drc.json")) else {
        return Vec::new();
    };
    let Ok(report) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let number_after = |text: &str, key: &str| -> Option<usize> {
        let rest = &text[text.find(key)? + key.len()..];
        rest.trim_start().split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
    };
    let mut found = Vec::new();
    for violation in report["violations"].as_array().into_iter().flatten() {
        if violation["type"].as_str() != Some("starved_thermal") {
            continue;
        }
        let description = violation["description"].as_str().unwrap_or("");
        let Some(pad) = violation["items"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|item| item["description"].as_str().is_some_and(|text| text.contains("ad ")))
        else {
            continue;
        };
        let layer = description
            .split("layer ")
            .nth(1)
            .and_then(|rest| rest.split([';', ')']).next())
            .unwrap_or("")
            .to_string();
        let spokes = number_after(description, "actual");
        let needed = number_after(description, "spoke count");
        let advice = if description.contains("isolated island") {
            "its spokes reach a piece of fill not joined to the rest of the pour: join that piece (a via or a track) or connect the pad by track".to_string()
        } else {
            format!(
                "only {} of {} spokes reach the fill: keep other copper off the pad's sides, or give the pad a solid zone connection (its zone_connect 2)",
                spokes.map_or("some".to_string(), |count| count.to_string()),
                needed.map_or("the needed".to_string(), |count| count.to_string())
            )
        };
        found.push(KiCadStarvedThermal {
            pad: pad["description"].as_str().unwrap_or("").to_string(),
            layer,
            at: [pad["pos"]["x"].as_f64().unwrap_or(0.0), pad["pos"]["y"].as_f64().unwrap_or(0.0)],
            spokes,
            needed,
            advice,
        });
    }
    found
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadDeadPad {
    pub pad: String,
    pub net: String,
    pub at: [f64; 2],
    pub near: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadHotSpot {
    pub layer: String,
    pub at: [f64; 2],
    pub history: f32,
}

fn diagnostics(board: &core::Board, result: &core::RoutingResult, layer_names: &[String]) -> KiCadRoutingDiagnostics {
    let round = |point: [f64; 2]| [(point[0] * 1000.0).round() / 1000.0, (point[1] * 1000.0).round() / 1000.0];
    KiCadRoutingDiagnostics {
        dead_pads: result
            .diagnostics
            .dead_pads
            .iter()
            .map(|dead| KiCadDeadPad {
                pad: dead.label.clone(),
                net: board.nets[dead.net as usize].name.clone(),
                at: round(dead.anchor),
                near: dead.near.clone(),
            })
            .collect(),
        conflicted_nets: result
            .diagnostics
            .conflicted
            .iter()
            .map(|net| board.nets[*net as usize].name.clone())
            .collect(),
        hot_spots: result
            .diagnostics
            .hot_spots
            .iter()
            .map(|spot| KiCadHotSpot {
                layer: layer_names.get(spot.layer).cloned().unwrap_or_default(),
                at: round(spot.center),
                history: spot.history,
            })
            .collect(),
        starved_thermals: Vec::new(),
    }
}

fn shape(geometry: &ObstacleGeometry) -> core::Shape {
    match geometry {
        ObstacleGeometry::Circle { center, radius } => core::Shape::Circle {
            center: *center,
            radius: *radius,
        },
        ObstacleGeometry::Segment { start, end, radius } => core::Shape::Capsule {
            start: *start,
            end: *end,
            radius: *radius,
        },
        ObstacleGeometry::Rectangle {
            center,
            half_size,
            angle_degrees,
        } => core::Shape::rectangle(*center, *half_size, *angle_degrees),
        ObstacleGeometry::Polygon { points } => core::Shape::Polygon {
            points: points.clone(),
        },
        ObstacleGeometry::Union { parts } => core::Shape::Union {
            parts: parts.iter().map(shape).collect(),
        },
    }
}

/// The board's copper layers in stack order: front, inner 1..n, back.
pub(super) struct LayerTable {
    pub names: Vec<String>,
}

impl LayerTable {
    pub fn from_pcb(pcb: &Expr) -> Result<Self, String> {
        let mut names: Vec<String> = pcb
            .child("layers")
            .map(Expr::children)
            .unwrap_or_default()
            .iter()
            .filter_map(|entry| entry.children().get(1).and_then(Expr::atom))
            .filter(|name| name.ends_with(".Cu"))
            .map(str::to_owned)
            .collect();
        let rank = |name: &str| -> (u8, u32) {
            match name {
                "F.Cu" => (0, 0),
                "B.Cu" => (2, 0),
                _ => (
                    1,
                    name.trim_start_matches("In")
                        .trim_end_matches(".Cu")
                        .parse()
                        .unwrap_or(u32::MAX),
                ),
            }
        };
        names.sort_by_key(|name| rank(name));
        if names.first().map(String::as_str) != Some("F.Cu")
            || names.last().map(String::as_str) != Some("B.Cu")
            || names.len() > 32
        {
            return Err(format!(
                "unsupported copper layer stack {names:?}: expected F.Cu, inner layers, B.Cu"
            ));
        }
        Ok(Self { names })
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|layer| layer == name)
    }

    pub fn all(&self) -> core::LayerMask {
        (1u32 << self.len()) - 1
    }

    /// Mask for a KiCad layer list such as `("F.Cu" "In1.Cu")`, `*.Cu`
    /// (all copper) or `F&B.Cu` (both outer layers).
    pub fn mask_of<'a>(
        &self,
        names: impl Iterator<Item = &'a str>,
    ) -> Result<core::LayerMask, String> {
        let mut mask = 0;
        for name in names {
            match name {
                "*.Cu" => mask |= self.all(),
                "F&B.Cu" => mask |= 1 | 1 << (self.len() - 1),
                // A copper layer the stackup does not have (a footprint
                // made for more layers) carries nothing, as in KiCad.
                name if name.ends_with(".Cu") => {
                    if let Some(index) = self.index(name) {
                        mask |= 1 << index;
                    }
                }
                _ => {}
            }
        }
        Ok(mask)
    }

    /// Mask of an item with either a `layer` or a `layers` form.
    pub fn mask_of_item(&self, item: &Expr) -> Result<core::LayerMask, String> {
        if let Some(layer) = form_atom(item, "layer", 1) {
            return self.mask_of(std::iter::once(layer));
        }
        let names: Vec<&str> = item
            .child("layers")
            .map(Expr::children)
            .unwrap_or_default()
            .iter()
            .skip(1)
            .filter_map(Expr::atom)
            .collect();
        self.mask_of(names.into_iter())
    }
}

thread_local! {
    /// `unconnected-(...)` nets that nevertheless join two or more pads: a
    /// reversible footprint's duplicate pad numbers on both sides. KiCad
    /// wants them connected and reports them unconnected otherwise.
    static JUMPER_NETS: std::cell::RefCell<std::collections::HashSet<String>> = Default::default();
}

/// Records the `unconnected-(...)` nets of `pcb` that hold two or more
/// pads, for `routable_net` (on this thread, for the lowering that follows).
fn note_jumper_nets(pcb: &Expr) {
    let mut pads: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for footprint in pcb.children().iter().filter(|item| item.head() == Some("footprint")) {
        for pad in footprint.children().iter().filter(|item| item.head() == Some("pad")) {
            if let Some(net) = node_net(pad)
                && net.rsplit('/').next().unwrap_or(net).starts_with("unconnected-(")
            {
                *pads.entry(net).or_default() += 1;
            }
        }
    }
    JUMPER_NETS.with(|jumpers| {
        *jumpers.borrow_mut() = pads.into_iter().filter(|(_, count)| *count >= 2).map(|(net, _)| net.to_string()).collect();
    });
}

/// Whether a net (by its raw name, sheet path included) is routed. Bare
/// numbers are placeholders for single pins; `/1` is a labelled net; an
/// `unconnected-(...)` net is a single pin unless two pads share it.
pub(super) fn routable_net(raw: &str) -> bool {
    !raw.is_empty()
        && !raw.bytes().all(|byte| byte.is_ascii_digit())
        && (!raw.rsplit('/').next().unwrap_or(raw).starts_with("unconnected-(")
            || JUMPER_NETS.with(|jumpers| jumpers.borrow().contains(raw)))
}

/// The drilled hole of a pad: a circle, or a capsule for a slot
/// (`(drill oval 1 7)`), centred on the pad.
fn drill_shape(pad: &Expr, center: [f64; 2]) -> Result<Option<core::Shape>, String> {
    let Some(drill) = pad.child("drill") else {
        return Ok(None);
    };
    let values: Vec<f64> = drill
        .children()
        .iter()
        .skip(1)
        .filter_map(Expr::atom)
        .filter_map(|value| value.parse::<f64>().ok())
        .collect();
    let Some(&diameter) = values.first().filter(|diameter| **diameter > 0.0) else {
        return Ok(None);
    };
    let (sin, cos) = (-form_at(pad)?[2]).to_radians().sin_cos();
    Ok(Some(match values.get(1) {
        Some(&height) if (height - diameter).abs() > 1.0e-9 => {
            let (long, short) = (diameter.max(height), diameter.min(height));
            let axis = if diameter >= height { [1.0, 0.0] } else { [0.0, 1.0] };
            let half = (long - short) / 2.0;
            let offset = [
                half * (axis[0] * cos - axis[1] * sin),
                half * (axis[0] * sin + axis[1] * cos),
            ];
            core::Shape::Capsule {
                start: [center[0] - offset[0], center[1] - offset[1]],
                end: [center[0] + offset[0], center[1] + offset[1]],
                radius: short / 2.0,
            }
        }
        _ => core::Shape::Circle {
            center,
            radius: diameter / 2.0,
        },
    }))
}

pub(super) struct Lowered {
    pub board: core::Board,
}

static LOWER_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static LOWER_NANOS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Calls of `lower` in this process and their seconds (summed over
/// threads), for the board check's line.
pub(super) fn lowering_totals() -> (u64, f64) {
    use std::sync::atomic::Ordering::Relaxed;
    (LOWER_CALLS.load(Relaxed), LOWER_NANOS.load(Relaxed) as f64 / 1.0e9)
}

pub(super) fn lower(
    pcb: &Expr,
    config: &KiCadBoardRouterConfig,
    connect_pours: bool,
) -> Result<Lowered, String> {
    use std::sync::atomic::Ordering::Relaxed;
    let started = std::time::Instant::now();
    let lowered = lower_board(pcb, config, connect_pours);
    LOWER_CALLS.fetch_add(1, Relaxed);
    LOWER_NANOS.fetch_add(started.elapsed().as_nanos() as u64, Relaxed);
    lowered
}

fn lower_board(
    pcb: &Expr,
    config: &KiCadBoardRouterConfig,
    connect_pours: bool,
) -> Result<Lowered, String> {
    note_jumper_nets(pcb);
    crate::note_net_names(pcb);
    let loops = outline::board_loops(pcb)?;
    let layers = LayerTable::from_pcb(pcb)?;
    let mut classes: Vec<core::RuleClass> = Vec::new();
    let mut nets: Vec<core::Net> = Vec::new();
    let mut net_ids = BTreeMap::<String, core::NetId>::new();
    let mut obstacles = Vec::new();
    // Rule areas that keep zone fills out: (layers, polygon).
    let mut pour_keepouts: Vec<(u32, Vec<[f64; 2]>)> = Vec::new();
    let mut solid_pads: Vec<String> = Vec::new();
    // Openings in the solder mask (graphics on a mask layer), placed once
    // the pads under them are known.
    // The margin is how far the opening reaches past its shapes (a
    // mask-only pad's mask expansion); the last value is the aperture's
    // mask expansion as KiCad's bridge test takes it (a graphic's own
    // `solder_mask_margin`, else the board's `pad_to_mask_clearance`).
    let mut mask_openings: Vec<(Vec<core::Shape>, usize, String, f64, f64)> = Vec::new();
    // Board copper graphics, with the net they carry, placed once the nets
    // of the pads are known.
    let mut copper_graphics: Vec<(Vec<core::Shape>, core::LayerMask, Option<String>, String)> = Vec::new();
    let mut isolated_pads: Vec<String> = Vec::new();
    let mut pad_spokes: std::collections::HashMap<usize, core::PadSpokes> = std::collections::HashMap::new();
    // With `use_narrow_signals`, signal nets (not poured, not named as a
    // ground or a rail) route at the project's smallest predefined width.
    let poured: std::collections::HashSet<String> = if config.use_narrow_signals == Some(true) {
        pours(pcb, &layers)?.into_iter().map(|pour| pour.net).collect()
    } else {
        Default::default()
    };

    let mut net_id = |name: &str,
                      nets: &mut Vec<core::Net>,
                      classes: &mut Vec<core::RuleClass>|
     -> Result<core::NetId, String> {
        if let Some(id) = net_ids.get(name) {
            return Ok(*id);
        }
        let rules = config
            .connection_rules
            .get(name)
            .or(config.default_rules.as_ref())
            .ok_or_else(|| format!("no routing rules for connection {name:?}"))?;
        let narrow = match (config.use_narrow_signals, config.narrow_signal_mm) {
            (Some(true), Some(width))
                if width < rules.trace_width_mm
                    && !poured.contains(name)
                    && !crate::quality::ground(name)
                    && !crate::quality::rail_name(name) =>
            {
                Some(width)
            }
            _ => None,
        };
        let class = core::RuleClass {
            trace_width: narrow.unwrap_or(rules.trace_width_mm).max(config.width_floor()),
            clearance: rules.clearance_mm,
            via_diameter: rules.via_size_mm,
            via_drill: rules.via_drill_mm,
        };
        let class_index = classes
            .iter()
            .position(|existing| *existing == class)
            .unwrap_or_else(|| {
                classes.push(class);
                classes.len() - 1
            });
        let id = nets.len() as core::NetId;
        nets.push(core::Net {
            name: name.to_string(),
            class: class_index,
            terminals: Vec::new(),
        });
        net_ids.insert(name.to_string(), id);
        Ok(id)
    };

    // A pad's solder mask aperture (the pad grown by its mask margin) bares
    // the copper around it: another net's copper within the mask web of it
    // bridges. Only margins that reach past the copper clearance matter.
    let board_mask_margin = pcb.child("setup").and_then(|setup| form_f64(setup, "solder_mask_margin", 1).ok()).unwrap_or(0.0);
    // KiCad's own name for the board's mask expansion. (The copper pads'
    // aperture rule above reads `solder_mask_margin`, which KiCad's setup
    // does not write; left as it is, measured with it.)
    let board_pad_to_mask = pcb.child("setup").and_then(|setup| form_f64(setup, "pad_to_mask_clearance", 1).ok()).unwrap_or(0.0);
    let pad_mask_reach = pcb
        .child("setup")
        .and_then(|setup| form_f64(setup, "solder_mask_min_width", 1).ok())
        .map_or(0.0, |width| width / 2.0 + 0.01);
    let smallest_clearance = config
        .connection_rules
        .values()
        .chain(config.default_rules.iter())
        .map(|rules| rules.clearance_mm)
        .fold(f64::INFINITY, f64::min);
    let outer_layers: core::LayerMask = 1 | (1 << layers.len().saturating_sub(1));
    for item in pcb.children() {
        match item.head() {
            Some("footprint") => {
                let footprint_at = form_at(item)?;
                let reference = footprint_reference(item).unwrap_or_default();
                // Rule areas inside the footprint (an antenna keepout) keep
                // tracks and vias out like board-level ones.
                for zone in item
                    .children()
                    .iter()
                    .filter(|child| child.head() == Some("zone") && is_rule_area(child))
                {
                    if let Some(obstacle) = rule_area_obstacle(zone, &layers, &format!("{reference} keepout"))? {
                        obstacles.push(obstacle);
                    }
                    if let Some(keepout) = pour_keepout(zone, &layers)? {
                        pour_keepouts.push(keepout);
                    }
                }
                // Copper drawn inside the footprint (a logo, a net tie's
                // bridge) belongs to no net.
                for graphic in item.children().iter().filter(|child| {
                    matches!(child.head(), Some("fp_line" | "fp_arc" | "fp_rect" | "fp_circle" | "fp_poly"))
                }) {
                    // Mask graphics open the mask like the board's own
                    // (Sisu's 190 B.Mask polygons, link's connector).
                    if let Some(side @ ("F.Mask" | "B.Mask")) = form_atom(graphic, "layer", 1) {
                        let shapes: Vec<core::Shape> = copper_graphic_shapes(graphic)?
                            .into_iter()
                            .map(|shape| place_shape(shape, footprint_at))
                            .collect();
                        if !shapes.is_empty() {
                            let layer = if side == "F.Mask" { 0 } else { layers.len() - 1 };
                            let expansion = form_f64(graphic, "solder_mask_margin", 1).ok().unwrap_or(board_pad_to_mask).max(0.0);
                            mask_openings.push((shapes, layer, graphic.head().unwrap_or("graphic").to_string(), 0.0, expansion));
                        }
                        continue;
                    }
                    let Some(layer) = form_atom(graphic, "layer", 1).and_then(|name| layers.index(name)) else {
                        continue;
                    };
                    for shape in copper_graphic_shapes(graphic)? {
                        obstacles.push(core::Obstacle {
                            shape: place_shape(shape, footprint_at),
                            layers: 1 << layer,
                            kind: core::ObstacleKind::Copper,
                            net: None,
                            clearance: 0.0,
                            clearance_override: None,
                            blocks_tracks: true,
                            blocks_vias: true,
                            label: format!("{reference} copper {}", graphic.head().unwrap_or("graphic")),
                        });
                    }
                }
                for pad in item
                    .children()
                    .iter()
                    .filter(|child| child.head() == Some("pad"))
                {
                    let pad_name = pad.children().get(1).and_then(Expr::atom).unwrap_or("");
                    let label = format!("{reference}.{}", pad_name.trim_matches('"'));
                    // KiCad's zone connection: the pad's own, else its
                    // footprint's (0 none, 1 thermal, 2 solid).
                    match form_atom(pad, "zone_connect", 1).or_else(|| form_atom(item, "zone_connect", 1)) {
                        Some("0") => isolated_pads.push(label.clone()),
                        Some("2") => solid_pads.push(label.clone()),
                        _ => {}
                    }
                    let lowered = lower_pad(pad, footprint_at)?;
                    let pad_type = pad.children().get(2).and_then(Expr::atom).unwrap_or("");
                    // Plated holes of pads without a net are just holes to
                    // everyone; the board's hole clearance applies. Holes of
                    // mechanical (unplated) pads always are.
                    let hole = (pad_type == "thru_hole" && !node_net(pad).is_some_and(routable_net))
                        || pad_type == "np_thru_hole";
                    if hole && let Some(shape) = drill_shape(pad, lowered.center)? {
                        obstacles.push(core::Obstacle {
                            shape,
                            layers: layers.all(),
                            kind: core::ObstacleKind::Hole,
                            net: None,
                            clearance: local_clearance::pad_clearance(pad, item)?,
                            clearance_override: None,
                            blocks_tracks: true,
                            blocks_vias: true,
                            label: if pad_type == "np_thru_hole" {
                                format!("{label} hole")
                            } else {
                                format!("{label} plated hole")
                            },
                        });
                    }
                    let pad_layers = layers.mask_of_item(pad)?;
                    if pad_layers == 0 {
                        // A pad with a mask opening and no copper (hackrf's
                        // mask-only test points over a track) bares the
                        // copper under it: KiCad reports a solder mask
                        // bridge when copper of a second net lies there. It
                        // is an opening like a mask graphic, grown by the
                        // pad's mask expansion.
                        if config.mask_only_pad_openings != Some(false) {
                            let names: Vec<&str> = pad
                                .child("layers")
                                .map(Expr::children)
                                .unwrap_or_default()
                                .iter()
                                .skip(1)
                                .filter_map(Expr::atom)
                                .collect();
                            let front = names.iter().any(|name| matches!(*name, "F.Mask" | "*.Mask" | "F&B.Mask"));
                            let back = names.iter().any(|name| matches!(*name, "B.Mask" | "*.Mask" | "F&B.Mask"));
                            let margin = form_f64(pad, "solder_mask_margin", 1)
                                .ok()
                                .or_else(|| form_f64(item, "solder_mask_margin", 1).ok())
                                .unwrap_or(board_pad_to_mask)
                                .max(0.0);
                            let kind = format!("pad {label}");
                            for (open, side) in [(front, 0), (back, layers.len() - 1)] {
                                if open {
                                    mask_openings.push((vec![shape(&lowered.geometry)], side, kind.clone(), margin, margin));
                                }
                            }
                        }
                        continue;
                    }
                    let net_name = node_net(pad).map(normalize_net);
                    let net = node_net(pad)
                        .filter(|raw| routable_net(raw))
                        .map(normalize_net)
                        .map(|name| net_id(name, &mut nets, &mut classes))
                        .transpose()?;
                    // A positive pad or footprint override replaces the
                    // net class's clearance in KiCad, also when smaller.
                    let clearance_override =
                        Some(local_clearance::pad_clearance(pad, item)?).filter(|value| *value > 0.0);
                    let mut clearance = 0.0;
                    if net.is_none() {
                        // A pad the router never connects (single-pin net,
                        // unconnected pin, no net) still keeps its net
                        // class's clearance in KiCad's DRC.
                        let rules = net_name
                            .and_then(|name| config.connection_rules.get(name))
                            .or(config.default_rules.as_ref());
                        if let Some(rules) = rules {
                            clearance = rules.clearance_mm;
                        }
                    }
                    let pad_obstacle = obstacles.len();
                    // A custom pad's box holds its primitives: its
                    // spokes stay along the axes of its copper's box.
                    if pad.children().get(3).and_then(Expr::atom) != Some("custom")
                        && let Ok(frame) = pad_spokes_frame(pad, lowered.center)
                    {
                        pad_spokes.insert(pad_obstacle, frame);
                    }
                    obstacles.push(core::Obstacle {
                        shape: shape(&lowered.geometry),
                        layers: pad_layers,
                        kind: core::ObstacleKind::Copper,
                        net,
                        clearance,
                        clearance_override,
                        blocks_tracks: true,
                        blocks_vias: true,
                        label: label.clone(),
                    });
                    // The pad's mask aperture, where it reaches past the
                    // copper clearance (a "Hole, 3mm" footprint: a ring of
                    // small pads with 0.1 mm margins whose apertures merge
                    // into a mask-free annulus; a track keeping the copper
                    // clearance from each small pad ran inside it and
                    // bridged). A mask opening of the pad's net.
                    let mask_margin = form_f64(pad, "solder_mask_margin", 1)
                        .ok()
                        .or_else(|| form_f64(item, "solder_mask_margin", 1).ok())
                        .unwrap_or(board_mask_margin);
                    if mask_margin > 0.0
                        && mask_margin + pad_mask_reach > smallest_clearance.min(clearance_override.unwrap_or(f64::INFINITY)) - 1.0e-9
                        && pad_layers & outer_layers != 0
                    {
                        obstacles.push(core::Obstacle {
                            shape: shape(&lowered.geometry),
                            layers: pad_layers & outer_layers,
                            kind: core::ObstacleKind::Keepout,
                            net,
                            clearance: mask_margin + pad_mask_reach,
                            clearance_override: None,
                            blocks_tracks: true,
                            blocks_vias: true,
                            label: format!("{label} mask aperture"),
                        });
                    }
                    // Other nets keep the hole clearance from a plated hole
                    // too, which reaches past a thin annular ring (a thermal
                    // via pad: 0.3 drill in 0.5 copper, eurorack-pmod's U1).
                    // Their vias as well (jetson's J2: 0.55 drill in 0.75
                    // copper, clearance 0.125, hole clearance 0.25); the
                    // pad's own net is exempt (`StaticMaps::via_owner`).
                    if pad_type == "thru_hole"
                        && net.is_some()
                        && let Some(shape) = drill_shape(pad, lowered.center)?
                    {
                        obstacles.push(core::Obstacle {
                            shape,
                            layers: layers.all(),
                            kind: core::ObstacleKind::Hole,
                            net,
                            clearance: 0.0,
                            clearance_override: None,
                            blocks_tracks: true,
                            blocks_vias: true,
                            label: format!("{label} plated hole"),
                        });
                    }
                    if let Some(net) = net {
                        // Without a hole, a pad on several layers (an
                        // edge-mount connector's top and bottom pads) does
                        // not join them: it is reached on its footprint's
                        // side, and layer changes need a via.
                        let mut terminal_layers = pad_layers;
                        if pad.child("drill").is_none() && pad_layers.count_ones() > 1 {
                            let side = form_atom(item, "layer", 1)
                                .and_then(|name| layers.index(name))
                                .map(|layer| 1 << layer)
                                .filter(|mask: &core::LayerMask| mask & pad_layers != 0);
                            terminal_layers = side.unwrap_or(pad_layers & pad_layers.wrapping_neg());
                        }
                        // A custom pad connects through its anchor only.
                        let contact = match (pad.children().get(3).and_then(Expr::atom), &lowered.geometry) {
                            (Some("custom"), ObstacleGeometry::Union { parts }) if parts.len() > 1 => Some(shape(&parts[0])),
                            _ => None,
                        };
                        nets[net as usize].terminals.push(core::Terminal {
                            anchor: lowered.center,
                            layers: terminal_layers,
                            pad: pad_obstacle,
                            contact,
                            label,
                        });
                    }
                }
            }
            Some("segment") => {
                let Some(layer) = form_atom(item, "layer", 1).and_then(|name| layers.index(name))
                else {
                    continue;
                };
                let net = node_net(item)
                    .filter(|raw| routable_net(raw))
                    .map(normalize_net)
                    .map(|name| net_id(name, &mut nets, &mut classes))
                    .transpose()?;
                obstacles.push(core::Obstacle {
                    shape: core::Shape::Capsule {
                        start: form_xy(item, "start")?,
                        end: form_xy(item, "end")?,
                        radius: form_f64(item, "width", 1)? / 2.0,
                    },
                    layers: 1 << layer,
                    kind: core::ObstacleKind::Copper,
                    net,
                    clearance: 0.0,
                    clearance_override: None,
                    blocks_tracks: true,
                    blocks_vias: true,
                    label: "existing track".into(),
                });
            }
            Some("via") => {
                let net = node_net(item)
                    .filter(|raw| routable_net(raw))
                    .map(normalize_net)
                    .map(|name| net_id(name, &mut nets, &mut classes))
                    .transpose()?;
                obstacles.push(core::Obstacle {
                    shape: core::Shape::Circle {
                        center: form_xy(item, "at")?,
                        radius: form_f64(item, "size", 1)? / 2.0,
                    },
                    layers: layers.all(),
                    kind: core::ObstacleKind::Copper,
                    net,
                    clearance: 0.0,
                    clearance_override: None,
                    blocks_tracks: true,
                    blocks_vias: true,
                    label: "existing via".into(),
                });
            }
            Some("arc") => {
                return Err("the board router does not yet lower existing arc tracks".into());
            }
            // Board graphics: copper on the copper layers they name (KiCad
            // 9 draws them on several layers at once, with a net: Sisu's
            // GND rectangles on F.Cu and F.Mask), openings on the mask
            // layers. Graphics on a mask layer open the mask: copper of two
            // nets under one opening is a solder bridge (KiCad's
            // solder_mask_bridge), so routed copper stays out from under
            // them on that side.
            Some("gr_text" | "gr_line" | "gr_rect" | "gr_arc" | "gr_circle" | "gr_poly") => {
                let copper = layers.mask_of_item(item)?;
                let names: Vec<&str> = match form_atom(item, "layer", 1) {
                    Some(name) => vec![name],
                    None => item
                        .child("layers")
                        .map(Expr::children)
                        .unwrap_or_default()
                        .iter()
                        .skip(1)
                        .filter_map(Expr::atom)
                        .collect(),
                };
                let sides: Vec<usize> = names
                    .iter()
                    .filter_map(|name| match *name {
                        "F.Mask" => Some(0),
                        "B.Mask" => Some(layers.len() - 1),
                        _ => None,
                    })
                    .collect();
                if copper == 0 && sides.is_empty() {
                    continue;
                }
                let shapes = copper_graphic_shapes(item)?;
                let kind = item.head().unwrap_or("graphic").to_string();
                if copper != 0 {
                    let net = node_net(item).filter(|raw| routable_net(raw)).map(|raw| normalize_net(raw).to_string());
                    copper_graphics.push((shapes.clone(), copper, net, kind.clone()));
                }
                // KiCad's bridge test grows a shape by its mask expansion,
                // a text by nothing.
                let expansion = if item.head() == Some("gr_text") {
                    0.0
                } else {
                    form_f64(item, "solder_mask_margin", 1).ok().unwrap_or(board_pad_to_mask).max(0.0)
                };
                for side in sides {
                    mask_openings.push((shapes.clone(), side, kind.clone(), 0.0, expansion));
                }
            }
            Some("zone") if is_rule_area(item) => {
                if let Some(obstacle) = rule_area_obstacle(item, &layers, "unnamed rule area")? {
                    obstacles.push(obstacle);
                }
                if let Some(keepout) = pour_keepout(item, &layers)? {
                    pour_keepouts.push(keepout);
                }
            }
            _ => {}
        }
    }
    // Pads of one number that overlap are one pad to KiCad (a solder pad
    // with holes in it, an exposed pad's thermal vias): the largest is the
    // terminal, the others just its copper. OpenRX's panel hangs half of
    // each wire pad's holes over the edge, unreachable as terminals.
    let mut merged_pads: Vec<core::MergedPad> = Vec::new();
    for (net_id, net) in nets.iter_mut().enumerate() {
        let area = |terminal: &core::Terminal| {
            let bounds = obstacles[terminal.pad].shape.aabb();
            (bounds.maximum[0] - bounds.minimum[0]) * (bounds.maximum[1] - bounds.minimum[1])
        };
        let mut order: Vec<usize> = (0..net.terminals.len()).collect();
        order.sort_by(|a, b| area(&net.terminals[*b]).total_cmp(&area(&net.terminals[*a])).then(a.cmp(b)));
        let mut keep = vec![true; net.terminals.len()];
        let mut host = vec![usize::MAX; net.terminals.len()];
        for (position, &large) in order.iter().enumerate() {
            if !keep[large] {
                continue;
            }
            for &small in &order[position + 1..] {
                if keep[small]
                    && net.terminals[small].label == net.terminals[large].label
                    && obstacles[net.terminals[large].pad].shape.contains(net.terminals[small].anchor)
                {
                    keep[small] = false;
                    host[small] = large;
                }
            }
        }
        let mut renumbered = vec![usize::MAX; net.terminals.len()];
        let mut next = 0;
        for (index, kept) in keep.iter().enumerate() {
            if *kept {
                renumbered[index] = next;
                next += 1;
            }
        }
        for (index, terminal) in net.terminals.iter().enumerate() {
            if !keep[index] {
                merged_pads.push(core::MergedPad {
                    net: net_id as core::NetId,
                    host: renumbered[host[index]],
                    pad: terminal.pad,
                    anchor: terminal.anchor,
                    layers: obstacles[terminal.pad].layers,
                });
            }
        }
        let mut index = 0;
        net.terminals.retain(|_| {
            index += 1;
            keep[index - 1]
        });
    }
    // Copper graphics belong to their net (other nets keep clear) or, without
    // one, to none.
    for (index, (shapes, copper, net, kind)) in copper_graphics.into_iter().enumerate() {
        let net = net.and_then(|name| net_ids_lookup(&nets, &name));
        // A filled graphic with a net is copper KiCad wants connected like
        // a pad (Sisu's antenna feed, a rectangle on /RF/ANT, stayed an
        // island): it is a terminal of its net too.
        // Its fill is the one shape that is not a stroke's capsule.
        let fills: Vec<usize> =
            (0..shapes.len()).filter(|index| !matches!(shapes[*index], core::Shape::Capsule { .. })).collect();
        if let (Some(id), [fill]) = (net, fills.as_slice()) {
            let shape = &shapes[*fill];
            let bounds = shape.aabb();
            let center = [(bounds.minimum[0] + bounds.maximum[0]) / 2.0, (bounds.minimum[1] + bounds.maximum[1]) / 2.0];
            if shape.contains(center) {
                nets[id as usize].terminals.push(core::Terminal {
                    anchor: center,
                    layers: copper,
                    pad: obstacles.len() + fill,
                    contact: None,
                    label: format!("{kind} {}", index + 1),
                });
            }
        }
        obstacles.extend(shapes.into_iter().map(|shape| core::Obstacle {
            shape,
            layers: copper,
            kind: core::ObstacleKind::Copper,
            net,
            clearance: 0.0,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: format!("copper {kind}"),
        }));
    }
    // A mask opening exposes the copper under it: pads, or else the fill
    // of a pour. Over one net (a pad or a pour exposed on purpose) it is
    // that net's: it may route there, others would bridge to it. Over
    // several (J_LCD1's opening over its eight pads in Sisu) or none,
    // nothing new goes under it; pads inside leave it by their stubs. The
    // net is the whole graphic's.
    let pours = pours(pcb, &layers)?;
    // KiCad's bridge test drops mask slivers under the minimum web width:
    // copper of another net within half of it of an opening is exposed
    // (link: a track 0.025 mm from an F.Mask polygon bridged it to GND with
    // a 0.1 mm minimum; at 0.07 mm it did not).
    let mask_reach = pcb
        .child("setup")
        .and_then(|setup| form_f64(setup, "solder_mask_min_width", 1).ok())
        .map_or(0.0, |width| width / 2.0 + 0.01);
    // KiCad's bridge test (DRC_TEST_PROVIDER_SOLDER_MASK, KiCad 10)
    // exposes copper within the project's solder-mask-to-copper clearance
    // plus the aperture's expansion, and never less than the web rule
    // above.
    let mask_to_copper = config.solder_mask_to_copper_mm.unwrap_or(0.0).max(0.0);
    for (shapes, layer, kind, margin, expansion) in mask_openings {
        let reach = (mask_reach + margin).max(mask_to_copper + expansion);
        let bounds = shapes.iter().skip(1).fold(shapes[0].aabb(), |bounds, shape| bounds.union(shape.aabb())).inflated(margin);
        let mut exposed: std::collections::BTreeSet<core::NetId> = obstacles
            .iter()
            .filter(|obstacle| {
                obstacle.kind == core::ObstacleKind::Copper
                    && obstacle.layers & (1 << layer) != 0
                    && obstacle.shape.aabb().intersects(bounds)
            })
            .filter_map(|obstacle| obstacle.net)
            .collect();
        let center = [(bounds.minimum[0] + bounds.maximum[0]) / 2.0, (bounds.minimum[1] + bounds.maximum[1]) / 2.0];
        let under: Vec<&Pour> = pours
            .iter()
            .filter(|pour| pour.layers & (1 << layer) != 0 && core::geometry::point_in_polygon(center, &pour.polygon))
            .collect();
        // A pour's fill keeps its clearance from pads: it shows through
        // only where the opening has no pad of its own.
        if exposed.is_empty()
            && let Some(top) = under.iter().map(|pour| pour.priority).max()
        {
            exposed.extend(
                under
                    .iter()
                    .filter(|pour| pour.priority == top)
                    .filter_map(|pour| net_ids_lookup(&nets, &pour.net)),
            );
        }
        if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
            eprintln!("mask opening ({kind}) on layer {layer}: {:?}, nets {exposed:?}", bounds);
        }
        // Over bare board an opening is no net's: copper of two nets under
        // one text bridges (jetson-nano's B.Mask title over three nets'
        // vias and tracks).
        let net = if exposed.len() == 1 { exposed.first().copied() } else { None };
        obstacles.extend(shapes.into_iter().map(|shape| core::Obstacle {
            shape,
            layers: 1 << layer,
            kind: core::ObstacleKind::Keepout,
            net,
            clearance: reach,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: format!("solder mask opening ({kind})"),
        }));
    }
    for cutout in &loops.cutouts {
        // A cutout is board edge to KiCad: the edge clearance applies, not
        // the hole clearance.
        obstacles.push(core::Obstacle {
            shape: core::Shape::Polygon {
                points: cutout.clone(),
            },
            layers: layers.all(),
            kind: core::ObstacleKind::Hole,
            net: None,
            clearance: config.edge_clearance_mm + outline::ARC_TOLERANCE,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: "board cutout".into(),
        });
    }
    let polygon_area = |points: &[[f64; 2]]| {
        (0..points.len())
            .map(|index| {
                let (a, b) = (points[index], points[(index + 1) % points.len()]);
                a[0] * b[1] - a[1] * b[0]
            })
            .sum::<f64>()
            .abs()
            / 2.0
    };
    let board_area = polygon_area(&loops.outline);
    let mut planes = Vec::new();
    for pour in &pours {
        // A pour without pads has nothing to connect.
        let Some(net) = net_ids_lookup(&nets, &pour.net) else {
            continue;
        };
        let net_class = classes[nets[net as usize].class];
        let brush = core::RuleClass {
            trace_width: pour.min_thickness.max(0.05),
            clearance: pour.clearance.max(net_class.clearance),
            ..net_class
        };
        let class = classes
            .iter()
            .position(|existing| *existing == brush)
            .unwrap_or_else(|| {
                classes.push(brush);
                classes.len() - 1
            });
        for layer in 0..layers.len() {
            if pour.layers & (1 << layer) == 0 {
                continue;
            }
            planes.push(core::Plane {
                net,
                class,
                layer,
                polygon: pour.polygon.clone(),
                excluded: pours
                    .iter()
                    .filter(|other| {
                        other.net != pour.net
                            && other.layers & (1 << layer) != 0
                            && other.priority > pour.priority
                    })
                    .map(|other| other.polygon.clone())
                    .chain(
                        pour_keepouts
                            .iter()
                            .filter(|(mask, _)| mask & (1 << layer) != 0)
                            .map(|(_, polygon)| polygon.clone()),
                    )
                    .collect(),
                // KiCad cuts a same-net zone of a strictly higher priority
                // out of this one's fill (an equal one overlaps it).
                above: pours
                    .iter()
                    .filter(|other| other.net == pour.net && other.layers & (1 << layer) != 0 && other.priority > pour.priority)
                    .map(|other| other.polygon.clone())
                    .collect(),
                connect: connect_pours,
                thermal_reach: pour.thermal_reach,
                thermal_gap: pour.thermal_gap,
                // An inner layer poured over nearly the whole board is a
                // plane: it carries no signals.
                exclusive: layers.names[layer].starts_with("In") && polygon_area(&pour.polygon) >= 0.9 * board_area,
                solid: pour.solid,
                hatch: pour.hatch,
            });
        }
    }
    if let Some(limit) = config.exclusive_layers {
        let mut by_net: BTreeMap<core::NetId, Vec<usize>> = BTreeMap::new();
        for (index, plane) in planes.iter().enumerate() {
            if plane.exclusive {
                by_net.entry(plane.net).or_default().push(index);
            }
        }
        let last = layers.len().saturating_sub(1);
        for (_, mut indices) in by_net {
            indices.sort_by_key(|index| (planes[*index].layer.min(last - planes[*index].layer), planes[*index].layer));
            for index in indices.into_iter().skip(limit) {
                planes[index].exclusive = false;
            }
        }
    }
    if classes.is_empty() {
        return Err("the board has no routable connections".into());
    }
    Ok(Lowered {
        board: core::Board {
            layer_count: layers.len(),
            neck_width: config
                .neck_width_mm
                .unwrap_or_else(|| {
                    classes
                        .iter()
                        .map(|class| class.trace_width)
                        .fold(f64::INFINITY, f64::min)
                })
                .max(config.width_floor()),
            outline: loops.outline.clone(),
            // Arcs are flattened to chords inside the true curve; the
            // chord error is added so copper stays clear of the real edge.
            edge_clearance: config.edge_clearance_mm + outline::ARC_TOLERANCE,
            hole_clearance: config.hole_clearance_mm,
            hole_to_hole: config.hole_to_hole_clearance_mm,
            classes,
            obstacles,
            nets,
            planes,
            solid_pads: {
                solid_pads.sort();
                solid_pads
            },
            isolated_pads: {
                isolated_pads.sort();
                isolated_pads
            },
            pad_spokes,
            merged_pads,
        },
    })
}

/// KiCad's frame for a pad's thermal spokes (`core::PadSpokes`): its shape
/// position, size at orientation 0, absolute orientation and spoke angle
/// (the pad's `thermal_bridge_angle`, else 45 degrees for a circle and 90
/// for any other shape, a custom pad's by its anchor).
fn pad_spokes_frame(pad: &Expr, center: [f64; 2]) -> Result<core::PadSpokes, String> {
    let pad_at = form_at(pad)?;
    let size = form_xy(pad, "size")?;
    let shape = pad.children().get(3).and_then(Expr::atom).unwrap_or("rect");
    let anchor = if shape == "custom" {
        pad.child("options").and_then(|options| form_atom(options, "anchor", 1)).unwrap_or("circle")
    } else {
        shape
    };
    let offset = match pad.child("drill").filter(|drill| drill.child("offset").is_some()) {
        Some(drill) => {
            let (x, y) = (form_f64(drill, "offset", 1).unwrap_or(0.0), form_f64(drill, "offset", 2).unwrap_or(0.0));
            let (sine, cosine) = (-pad_at[2]).to_radians().sin_cos();
            [x * cosine - y * sine, x * sine + y * cosine]
        }
        None => [0.0, 0.0],
    };
    Ok(core::PadSpokes {
        center: [center[0] + offset[0], center[1] + offset[1]],
        half: [size[0] / 2.0, size[1] / 2.0],
        orientation: pad_at[2],
        spoke_angle: form_f64(pad, "thermal_bridge_angle", 1).unwrap_or(if anchor == "circle" { 45.0 } else { 90.0 }),
        circular: shape == "circle" || (shape == "oval" && (size[0] - size[1]).abs() < 1.0e-9),
    })
}

/// Lowers the board as it is, its tracks and vias kept as fixed copper,
/// with the pours connected, and writes the router's pour map before any
/// routing (`Router::dump_pours`) into `output_directory`, with
/// `layers.json` naming the layers. For comparing the pour model with
/// KiCad's fill on the same copper (experiments/pour-fidelity).
pub fn dump_kicad_pour_map(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardRouterConfig,
) -> Result<(), String> {
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    let source = fs::read_to_string(&source_board)
        .map_err(|error| format!("failed to read {}: {error}", source_board.display()))?;
    let pcb = parse(&source)?;
    let Lowered { board } = lower(&pcb, config, true)?;
    let layer_names = LayerTable::from_pcb(&pcb)?.names;
    let mut router = core::router::Router::new(&board, &core_config(config));
    router.adopt_existing_copper();
    if std::env::var_os("PCB_ROUTER_DUMP_MANUAL_SPOKES").is_some() {
        eprintln!("manual spokes: {} added", router.add_manual_spokes());
    }
    router.dump_pours(output_directory).map_err(|error| error.to_string())?;
    let names: Vec<String> = layer_names.iter().map(|name| format!("\"{name}\"")).collect();
    fs::write(output_directory.join("layers.json"), format!("[{}]\n", names.join(", "))).map_err(|error| error.to_string())
}

/// The search benchmark: the board lowered with its pours connected, a
/// router built, and `iterations` negotiation iterations run with an
/// unbounded work budget (the run stops at the iteration count). Returns
/// the work done and a fingerprint of the routes, which must not change
/// when only the speed of the search does.
pub fn bench_kicad_search(
    source_directory: &Path,
    board_id: &str,
    iterations: usize,
    config: &KiCadBoardRouterConfig,
) -> Result<String, String> {
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    let source = fs::read_to_string(&source_board)
        .map_err(|error| format!("failed to read {}: {error}", source_board.display()))?;
    let pcb = parse(&source)?;
    let Lowered { board } = lower(&pcb, config, true)?;
    let mut router_config = core_config(config);
    router_config.max_iterations = iterations;
    let mut router = core::router::Router::new(&board, &router_config);
    let started = std::time::Instant::now();
    let unfinished = router.probe(1.0e7);
    let seconds = started.elapsed().as_secs_f64();
    let snapshot = router.snapshot();
    let vias: usize = snapshot.routes.iter().map(|route| route.vias.len()).sum();
    let length: f64 = snapshot
        .routes
        .iter()
        .flat_map(|route| route.segments.iter())
        .map(|segment| distance_squared(segment.start, segment.end).sqrt())
        .sum();
    Ok(format!(
        "bench: {} expansions, {unfinished} unfinished, {vias} vias, {length:.4} mm, {seconds:.1} s",
        router.expansions()
    ))
}

fn net_ids_lookup(nets: &[core::Net], name: &str) -> Option<core::NetId> {
    nets.iter()
        .position(|net| net.name == name)
        .map(|index| index as core::NetId)
}

/// Copies `source` to `destination` without tracks, vias and stale pour
/// fills. Unlike the cold-board strip, copper pours themselves are kept:
/// where copper is poured is a design decision like the placement, and the
/// router connects to pours instead of replacing them by tracks.
pub fn write_kicad_board_without_tracks(source: &Path, destination: &Path) -> Result<(), String> {
    let text = fs::read_to_string(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?;
    let mut pcb = parse(&text)?;
    let board_area = polygon_area(&outline::board_loops(&pcb)?.outline);
    let Expr::List(items) = &mut pcb else {
        return Err("PCB root is not a list".into());
    };
    items.retain(|item| !matches!(item.head(), Some("segment" | "arc" | "via")));
    // Small filled zones are hand-drawn copper, i.e. routing; only pours
    // covering a good part of the board are kept.
    items.retain(|item| {
        item.head() != Some("zone")
            || is_rule_area(item)
            || rule_area_polygon_points(item)
                .is_ok_and(|points| polygon_area(&points) >= POUR_SHARE * board_area)
    });
    for zone in items.iter_mut().filter(|item| item.head() == Some("zone")) {
        if let Expr::List(children) = zone {
            children
                .retain(|child| !matches!(child.head(), Some("filled_polygon" | "fill_segments")));
        }
    }
    // KiCad refuses a board with items on undefined layers; the stripped
    // board loads like the routed one will (koeg-board's Rescue layer).
    move_undefined_layers(&mut pcb);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    fs::write(destination, format!("{}\n", encode(&pcb)))
        .map_err(|error| format!("failed to write {}: {error}", destination.display()))
}

/// A shape in a footprint's frame, placed on the board.
fn place_shape(shape: core::Shape, at: [f64; 3]) -> core::Shape {
    let place = |point: [f64; 2]| {
        let offset = rotate_vector(point, -at[2]);
        [at[0] + offset[0], at[1] + offset[1]]
    };
    match shape {
        core::Shape::Circle { center, radius } => core::Shape::Circle {
            center: place(center),
            radius,
        },
        core::Shape::Capsule { start, end, radius } => core::Shape::Capsule {
            start: place(start),
            end: place(end),
            radius,
        },
        core::Shape::Polygon { points } => core::Shape::Polygon {
            points: points.into_iter().map(place).collect(),
        },
        core::Shape::Union { parts } => core::Shape::Union {
            parts: parts.into_iter().map(|part| place_shape(part, at)).collect(),
        },
    }
}

/// Copper-layer graphics as obstacle shapes. Text becomes its (generous)
/// bounding box. Footprint graphics (`fp_*`) come out in the footprint's
/// frame; `place_shape` puts them on the board.
pub(super) fn copper_graphic_shapes(item: &Expr) -> Result<Vec<core::Shape>, String> {
    let width = item
        .child("stroke")
        .and_then(|stroke| form_f64(stroke, "width", 1).ok())
        .or_else(|| form_f64(item, "width", 1).ok())
        .unwrap_or(0.1);
    let capsule = |start: [f64; 2], end: [f64; 2]| core::Shape::Capsule {
        start,
        end,
        radius: width / 2.0,
    };
    let shapes = match item.head() {
        Some("gr_line" | "fp_line") => vec![capsule(form_xy(item, "start")?, form_xy(item, "end")?)],
        Some("gr_arc" | "fp_arc") => outline::arc_points(
            form_xy(item, "start")?,
            form_xy(item, "mid")?,
            form_xy(item, "end")?,
        )
        .windows(2)
        .map(|pair| capsule(pair[0], pair[1]))
        .collect(),
        Some("gr_rect" | "fp_rect") => {
            let (a, b) = (form_xy(item, "start")?, form_xy(item, "end")?);
            let corners = [a, [b[0], a[1]], b, [a[0], b[1]]];
            let filled = !matches!(form_atom(item, "fill", 1), None | Some("none" | "no"));
            // A filled rectangle's edges matter only with a stroke reaching
            // beyond the fill.
            let mut shapes: Vec<_> = if filled && width <= 0.0 {
                Vec::new()
            } else {
                (0..4).map(|index| capsule(corners[index], corners[(index + 1) % 4])).collect()
            };
            if filled {
                shapes.push(core::Shape::Polygon {
                    points: corners.to_vec(),
                });
            }
            shapes
        }
        Some("gr_circle" | "fp_circle") => {
            let center = form_xy(item, "center")?;
            let radius = distance_squared(center, form_xy(item, "end")?).sqrt();
            let filled = !matches!(form_atom(item, "fill", 1), None | Some("no" | "none"));
            if filled || width <= 0.0 {
                vec![core::Shape::Circle {
                    center,
                    radius: radius + width / 2.0,
                }]
            } else {
                // Unfilled: the ring only.
                outline::circle_points(center, radius)
                    .windows(2)
                    .map(|pair| core::Shape::Capsule {
                        start: pair[0],
                        end: pair[1],
                        radius: width / 2.0 + 2.0 * outline::ARC_TOLERANCE,
                    })
                    .collect()
            }
        }
        Some("gr_poly" | "fp_poly") => {
            let points = rule_area_like_points(item)?;
            // Old polygons have no fill form and are filled; the stroke
            // reaches half its width beyond the outline (SNSP's J2 mask
            // opening: a via 0.006 mm off the bare outline bridged to it).
            let filled = !matches!(form_atom(item, "fill", 1), Some("no" | "none"));
            let mut shapes: Vec<_> = if width > 0.0 || !filled {
                (0..points.len()).map(|index| capsule(points[index], points[(index + 1) % points.len()])).collect()
            } else {
                Vec::new()
            };
            if filled {
                shapes.push(core::Shape::Polygon { points });
            }
            shapes
        }
        // Text in a TrueType face carries its glyphs as polygons.
        Some("gr_text")
            if item
                .child("render_cache")
                .is_some_and(|cache| cache.children().iter().any(|child| child.head() == Some("polygon"))) =>
        {
            let cache = item.child("render_cache").unwrap();
            cache
                .children()
                .iter()
                .filter(|child| child.head() == Some("polygon"))
                .map(|polygon| Ok(core::Shape::Polygon { points: rule_area_like_points(polygon)? }))
                .collect::<Result<_, String>>()?
        }
        Some("gr_text") => {
            let at = form_at(item)?;
            let text = item.children().get(1).and_then(Expr::atom).unwrap_or("");
            let font = item
                .child("effects")
                .and_then(|effects| effects.child("font"));
            let size = font
                .and_then(|font| form_xy(font, "size").ok())
                .unwrap_or([1.0, 1.0]);
            let thickness = font
                .and_then(|font| form_f64(font, "thickness", 1).ok())
                .unwrap_or(0.15);
            // KiCad's stroke font, measured on its own plots: glyph
            // advances in units of the font width (`i` 0.48 ... `m` 1.33);
            // capitals and digits are one font height tall, descenders
            // reach 0.3 below, and lines are 1.6 heights apart. The sum
            // gets 8 % on top so the box stays outside every measured text.
            let advance = |character: char| match character {
                'i' | 'j' | 'I' | '!' | '.' | ',' | ':' | ';' | '\'' | '`' => 0.48,
                'l' => 0.52,
                't' | 'f' => 0.57,
                'r' => 0.62,
                '(' | ')' | '[' | ']' | '{' | '}' => 0.67,
                'J' | '_' | ' ' | '"' => 0.76,
                'A' => 0.86,
                'N' | 'w' => 1.05,
                'M' | 'W' | '%' => 1.14,
                '-' => 1.24,
                '@' => 1.29,
                'm' => 1.33,
                _ => 1.0,
            };
            let lines = text.split("\\n").count().max(1) as f64;
            let widest = text
                .split("\\n")
                .map(|line| 1.08 * line.chars().map(advance).sum::<f64>())
                .fold(0.0, f64::max);
            let descends = text.chars().any(|c| matches!(c, 'g' | 'j' | 'p' | 'q' | 'y' | ',' | ';'));
            let half = [
                (widest * size[1] + thickness) / 2.0,
                (lines - 1.0) * 1.62 * size[0] / 2.0 + (if descends { 0.8 } else { 0.5 }) * size[0] + thickness / 2.0,
            ];
            let justify: Vec<&str> = item
                .child("effects")
                .and_then(|effects| effects.child("justify"))
                .map(|justify| {
                    justify
                        .children()
                        .iter()
                        .skip(1)
                        .filter_map(Expr::atom)
                        .collect()
                })
                .unwrap_or_default();
            let shift = if justify.contains(&"left") {
                half[0]
            } else if justify.contains(&"right") {
                -half[0]
            } else {
                0.0
            };
            let shift = if justify.contains(&"mirror") {
                -shift
            } else {
                shift
            };
            // Top and bottom put the anchor on that side of the box.
            let rise = if justify.contains(&"top") {
                half[1]
            } else if justify.contains(&"bottom") {
                -half[1]
            } else {
                0.0
            };
            let offset = rotate_vector([shift, rise], -at[2]);
            // KiCad's glyphs reach a little past this model across the
            // line (SNSP's bold "SNSP" on B.Cu 0.17 mm past the box of a
            // 2.5 mm text, jetson-nano's title 0.03 mm): some room on both
            // sides.
            let half = [half[0], half[1] + 0.08 * size[0]];
            vec![core::Shape::rectangle(
                [at[0] + offset[0], at[1] + offset[1]],
                half,
                -at[2],
            )]
        }
        _ => Vec::new(),
    };
    // A knockout text is a copper plate with the glyphs cut out: its
    // bounding box, grown by the stroke, is the copper (Quanta75's 7 mm
    // "0bsilab" on F.Cu: tracks and pads landed inside the plate, shorts
    // and mask bridges).
    let knockout = matches!(item.head(), Some("gr_text" | "fp_text"))
        && item.child("layer").is_some_and(|layer| layer.children().iter().skip(2).any(|child| child.atom() == Some("knockout")));
    if knockout && !shapes.is_empty() {
        let thickness = item
            .child("effects")
            .and_then(|effects| effects.child("font"))
            .and_then(|font| form_f64(font, "thickness", 1).ok())
            .unwrap_or(0.15);
        let bounds = shapes.iter().skip(1).fold(shapes[0].aabb(), |bounds, shape| bounds.union(shape.aabb()));
        let (low, high) = (bounds.minimum, bounds.maximum);
        return Ok(vec![core::Shape::Polygon {
            points: vec![
                [low[0] - thickness, low[1] - thickness],
                [high[0] + thickness, low[1] - thickness],
                [high[0] + thickness, high[1] + thickness],
                [low[0] - thickness, high[1] + thickness],
            ],
        }]);
    }
    Ok(shapes)
}

/// A rule area that keeps zone fills out (`(copperpour not_allowed)`): its
/// layers and polygon. KiCad's fill does not enter it, so neither does a
/// pour's on those layers (Sisu: a 30 x 18 mm area on In3 and three on
/// In4 and B.Cu, which the pour model counted as fill).
fn pour_keepout(zone: &Expr, layers: &LayerTable) -> Result<Option<(u32, Vec<[f64; 2]>)>, String> {
    let keeps_fill_out = zone
        .child("keepout")
        .and_then(|keepout| keepout.child("copperpour"))
        .and_then(|form| form.children().get(1))
        .and_then(Expr::atom)
        == Some("not_allowed");
    if !keeps_fill_out {
        return Ok(None);
    }
    // Only copper layers matter, and an outline that is not a polygon is
    // skipped unless it is a drawn circle (TelemetryOnboard: circles on
    // F.SilkS, outlines of one arc each, refused the whole board).
    let mask = layers.mask_of_item(zone)?;
    if mask == 0 {
        return Ok(None);
    }
    if let Ok(points) = rule_area_polygon_points(zone) {
        return Ok(Some((mask, points)));
    }
    Ok(rule_area_circle(zone).map(|points| (mask, points)))
}

/// A rule area outlined by one full-circle arc (start and end equal, the
/// middle across), as a 32-gon around it; `None` for any other outline.
fn rule_area_circle(zone: &Expr) -> Option<Vec<[f64; 2]>> {
    let pts = zone.child("polygon")?.child("pts")?;
    let arcs: Vec<&Expr> = pts.children().iter().filter(|item| item.head() == Some("arc")).collect();
    let [arc] = arcs.as_slice() else {
        return None;
    };
    let point = |name: &str| -> Option<[f64; 2]> {
        let form = arc.child(name)?;
        Some([form.children().get(1)?.atom()?.parse().ok()?, form.children().get(2)?.atom()?.parse().ok()?])
    };
    let (start, middle, end) = (point("start")?, point("mid")?, point("end")?);
    if (start[0] - end[0]).abs() > 1.0e-6 || (start[1] - end[1]).abs() > 1.0e-6 {
        return None;
    }
    let center = [(start[0] + middle[0]) / 2.0, (start[1] + middle[1]) / 2.0];
    let radius = ((start[0] - middle[0]).powi(2) + (start[1] - middle[1]).powi(2)).sqrt() / 2.0;
    (radius > 0.0).then(|| {
        (0..32)
            .map(|index| {
                let angle = index as f64 * std::f64::consts::TAU / 32.0;
                [center[0] + radius * angle.cos(), center[1] + radius * angle.sin()]
            })
            .collect()
    })
}

/// A rule area that keeps tracks or vias out, as an obstacle. Zones inside
/// footprints are stored in board coordinates too.
fn rule_area_obstacle(zone: &Expr, layers: &LayerTable, fallback_label: &str) -> Result<Option<core::Obstacle>, String> {
    let allowed = |kind: &str| {
        zone.child("keepout")
            .and_then(|keepout| keepout.child(kind))
            .and_then(|form| form.children().get(1))
            .and_then(Expr::atom)
            != Some("not_allowed")
    };
    let (blocks_tracks, blocks_vias) = (!allowed("tracks"), !allowed("vias"));
    if !(blocks_tracks || blocks_vias) {
        return Ok(None);
    }
    Ok(Some(core::Obstacle {
        shape: core::Shape::Polygon {
            points: rule_area_polygon_points(zone)?,
        },
        layers: layers.mask_of_item(zone)?,
        kind: core::ObstacleKind::Keepout,
        net: None,
        clearance: 0.0,
        clearance_override: None,
        blocks_tracks,
        blocks_vias,
        label: form_atom(zone, "name", 1).unwrap_or(fallback_label).to_string(),
    }))
}

fn rule_area_like_points(item: &Expr) -> Result<Vec<[f64; 2]>, String> {
    let points = outline::pts_points(item)?;
    if points.len() < 3 {
        return Err("copper polygon has fewer than three points".into());
    }
    Ok(points)
}

/// A zone is a pour when it covers at least this share of the board.
const POUR_SHARE: f64 = 0.1;

fn polygon_area(points: &[[f64; 2]]) -> f64 {
    (0..points.len())
        .map(|index| {
            let (a, b) = (points[index], points[(index + 1) % points.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        .abs()
        / 2.0
}

pub(super) struct Pour {
    pub(crate) net: String,
    pub(crate) layers: core::LayerMask,
    priority: i64,
    clearance: f64,
    min_thickness: f64,
    thermal_reach: f64,
    thermal_gap: f64,
    /// `(connect_pads yes ...)`: pads join wherever the fill touches.
    solid: bool,
    /// `(fill (mode hatch) ...)`.
    hatch: Option<core::Hatch>,
    polygon: Vec<[f64; 2]>,
}

pub(super) fn pours(pcb: &Expr, layers: &LayerTable) -> Result<Vec<Pour>, String> {
    let mut result = Vec::new();
    for zone in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone") && !is_rule_area(item))
    {
        let Some(net) = form_atom(zone, "net_name", 1)
            .or_else(|| node_net(zone))
            .filter(|raw| routable_net(raw))
            .map(normalize_net)
        else {
            continue;
        };
        if form_atom(zone, "fill", 1) == Some("no") {
            continue;
        }
        let mask = layers.mask_of_item(zone)?;
        if mask == 0 {
            continue;
        }
        result.push(Pour {
            net: net.to_string(),
            layers: mask,
            priority: form_atom(zone, "priority", 1)
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            clearance: zone
                .child("connect_pads")
                .and_then(|form| form_f64(form, "clearance", 1).ok())
                .unwrap_or(0.0),
            min_thickness: form_f64(zone, "min_thickness", 1).unwrap_or(0.25),
            solid: zone
                .child("connect_pads")
                .and_then(|form| form.children().get(1))
                .and_then(Expr::atom)
                == Some("yes"),
            thermal_gap: zone
                .child("fill")
                .map(|fill| form_f64(fill, "thermal_gap", 1).unwrap_or(0.5))
                .unwrap_or(0.5),
            thermal_reach: zone
                .child("fill")
                .map(|fill| {
                    form_f64(fill, "thermal_gap", 1).unwrap_or(0.5)
                        + form_f64(fill, "thermal_bridge_width", 1).unwrap_or(0.5)
                })
                .unwrap_or(1.0),
            hatch: zone.child("fill").filter(|fill| form_atom(fill, "mode", 1) == Some("hatch")).map(|fill| core::Hatch {
                thickness: form_f64(fill, "hatch_thickness", 1).unwrap_or(1.0),
                gap: form_f64(fill, "hatch_gap", 1).unwrap_or(1.5),
                orientation: form_f64(fill, "hatch_orientation", 1).unwrap_or(0.0),
                smoothing_level: form_f64(fill, "hatch_smoothing_level", 1).unwrap_or(0.0).max(0.0) as u32,
                smoothing_value: form_f64(fill, "hatch_smoothing_value", 1).unwrap_or(0.0),
                min_hole_area: form_f64(fill, "hatch_min_hole_area", 1).unwrap_or(0.3),
            }),
            polygon: rule_area_polygon_points(zone)?,
        });
    }
    Ok(result)
}

fn nanometres(point: [f64; 2]) -> [f64; 2] {
    point.map(|value| (value * 1.0e6).round() / 1.0e6)
}

/// The process's resident size now, in megabytes (Linux; elsewhere 0).
pub(crate) fn resident_mb() -> f64 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("VmRSS:"))?
                .split_whitespace()
                .nth(1)?
                .parse::<f64>()
                .ok()
        })
        .map_or(0.0, |kb| kb / 1024.0)
}

/// The process's peak resident size and the machine's available memory,
/// in megabytes (Linux; elsewhere unknown).
/// The machine's memory, in megabytes: a fixed share of it decides what
/// the ladder may attempt, so that the attempts do not depend on what else
/// runs (the memory available decides only how many run at once).
pub(crate) fn memory_total_mb() -> Option<f64> {
    fs::read_to_string("/proc/meminfo")
        .ok()?
        .lines()
        .find(|line| line.starts_with("MemTotal:"))?
        .split_whitespace()
        .nth(1)?
        .parse::<f64>()
        .ok()
        .map(|kb| kb / 1024.0)
}

pub(crate) fn memory_mb() -> Option<(f64, f64)> {
    let field = |text: &str, name: &str| -> Option<f64> {
        text.lines()
            .find(|line| line.starts_with(name))?
            .split_whitespace()
            .nth(1)?
            .parse::<f64>()
            .ok()
            .map(|kb| kb / 1024.0)
    };
    let peak = field(&fs::read_to_string("/proc/self/status").ok()?, "VmHWM:")?;
    let available = field(&fs::read_to_string("/proc/meminfo").ok()?, "MemAvailable:")?;
    Some((peak, available))
}

/// Whether the ladder's final step applies to this project: signals at
/// the project's narrowest predefined width (narrower than some class), or
/// the board's minimum clearance where the project rates clearance
/// findings as warnings (smaller than some class's clearance).
pub(crate) fn narrow_step_applies(config: &KiCadBoardRouterConfig) -> bool {
    let rules = || config.connection_rules.values().chain(config.default_rules.iter());
    let narrow = config.use_narrow_signals.is_none()
        && config
            .narrow_signal_mm
            .is_some_and(|narrow| rules().any(|rules| rules.trace_width_mm > narrow + 1.0e-9));
    let relaxed = config
        .relaxed_clearance_mm
        .is_some_and(|relaxed| rules().any(|rules| rules.clearance_mm > relaxed + 1.0e-9));
    narrow || relaxed
}

/// Work kept back for the final narrow-signal step, from the ladder's
/// first attempt's work: a share of it, at least 120 s (an attempt's
/// floor elsewhere in the ladder) and at most `cap`.
pub(crate) fn narrow_step_reserve(first_attempt_work: f64, share: f64, cap: f64) -> f64 {
    if share <= 0.0 {
        return 0.0;
    }
    (first_attempt_work * share).max(120.0).min(cap)
}

/// The least work kept for the narrow-signal step when the reserve is a
/// floor: the step's own minimum (`narrow_step_minimum`, 180 s) and its
/// clean-up at its negotiation's floor (60 s). TelemetryOnboard (v16)
/// kept 120 s, had 153 s left, and the step that completes it never ran.
pub(crate) const NARROW_RESERVE_FLOOR: f64 = 240.0;

/// `narrow_step_reserve` as a floor: at least `NARROW_RESERVE_FLOOR`, at
/// most `cap`, and none at all when the cap leaves less than the step
/// needs to run (`narrow_step_minimum`): a reserve the step can never use
/// would only take work from the rungs.
pub(crate) fn narrow_step_reserve_floored(first_attempt_work: f64, share: f64, cap: f64) -> f64 {
    if share <= 0.0 {
        return 0.0;
    }
    let kept = (first_attempt_work * share).max(NARROW_RESERVE_FLOOR).min(cap);
    if kept < narrow_step_minimum(kept) { 0.0 } else { kept }
}

/// The work the narrow-signal step needs left to run, with `kept` kept
/// for it: half of that, and at least three times its negotiation's floor
/// of 60 s (repair and polish take up to twice the negotiation again).
pub(crate) fn narrow_step_minimum(kept: f64) -> f64 {
    (0.5 * kept).max(180.0)
}

/// The narrow-signal attempt's negotiation in work seconds when the ladder
/// kept work for it: a third of what is left (repair and polish take up
/// to twice the negotiation again), within 60 to 900 s, and never more
/// than the attempt it varies had.
pub(crate) fn narrow_step_negotiation(work_left: f64, own: Option<f64>) -> f64 {
    let share = (work_left / 3.0).clamp(60.0, 900.0);
    own.map_or(share, |own| own.min(share))
}

/// `narrow_step_negotiation`, at least what the ladder's first attempt
/// negotiated (`first`), bounded by the work kept for the step and by
/// half of what is left (the hard repair after the negotiation needs as
/// much: a deadline inside the negotiation leaves every net in conflict
/// open). DSP-ADAU1452's step, varying a finer rung with a smaller
/// budget, got less negotiation than the first attempt had.
pub(crate) fn narrow_step_negotiation_floored(work_left: f64, own: Option<f64>, first: f64, kept: f64) -> f64 {
    narrow_step_negotiation(work_left, own).max(first.min(kept).min(0.5 * work_left))
}

/// Whether an attempt projected to take `cost` of work may start with
/// `work_left` left and `reserve` kept for the narrow-signal step: only if
/// the reserve stays whole after it (an attempt counts at least 120 s).
pub(crate) fn fits_before_reserve(work_left: f64, reserve: f64, cost: f64) -> bool {
    work_left - reserve >= cost.max(120.0)
}

/// The work an attempt on a lattice of `pitch` mm is projected to take
/// from one of `work` on `previous` mm: a lattice grows with the square of
/// the pitch ratio.
pub(crate) fn projected_attempt_work(work: f64, previous: f64, pitch: f64) -> f64 {
    if pitch > 0.0 && pitch < previous { work * (previous / pitch).powi(2) } else { work }
}

/// The work deadline of each of `routers` routers whose work counts
/// together (seeds side by side) when the work kept for the narrow-signal
/// step may not be touched: their share of what is left beyond it.
pub(crate) fn deadline_before_reserve(work_left: f64, reserve: f64, routers: usize) -> f64 {
    ((work_left - reserve) / routers.max(1) as f64).max(0.0)
}

/// `config` as the narrow-signal step routes it: signal nets at the
/// project's narrowest predefined width when that is narrower than a
/// class, and the board's minimum clearance when the project rates
/// clearance findings as warnings. Neither step applies to the result
/// again.
pub(crate) fn narrow_step_config(config: &KiCadBoardRouterConfig) -> KiCadBoardRouterConfig {
    let mut step = config.clone();
    let wider = |narrow: f64| config.connection_rules.values().chain(config.default_rules.iter()).any(|rules| rules.trace_width_mm > narrow + 1.0e-9);
    if config.use_narrow_signals.is_none() {
        step.use_narrow_signals = Some(config.narrow_signal_mm.is_some_and(wider));
    }
    if let Some(relaxed) = config.relaxed_clearance_mm {
        step.relaxed_clearance_mm = None;
        for rules in step.connection_rules.values_mut().chain(step.default_rules.iter_mut()) {
            rules.clearance_mm = rules.clearance_mm.min(relaxed);
        }
    }
    step
}

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
                deadline: Some(std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds.max(0.0))),
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
        let text = fs::read_to_string(&board).map_err(|error| format!("failed to read {}: {error}", board.display()))?;
        with_classes = crate::net_classes::with_net_classes(config, &parse(&text).map_err(crate::verdict::input_error)?)
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
        let text = fs::read_to_string(&board).map_err(|error| format!("failed to read {}: {error}", board.display()))?;
        planned_pours(&parse(&text).map_err(crate::verdict::input_error)?, config).map_err(crate::verdict::input_error)?
    };
    if config.add_pours.is_empty() && !planned.is_empty() {
        eprintln!(
            "automatic planes: {}",
            planned.iter().map(|pour| format!("{} on {}", pour.net, pour.layers.join(", "))).collect::<Vec<_>>().join("; ")
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
        let text = fs::read_to_string(&board).map_err(|error| format!("failed to read {}: {error}", board.display()))?;
        let mut pcb = parse(&text)?;
        add_pour_zones(&mut pcb, &planned).map_err(crate::verdict::input_error)?;
        fs::write(&board, format!("{}\n", encode(&pcb))).map_err(|error| format!("failed to write {}: {error}", board.display()))?;
        if !config.net_classes.is_empty() {
            crate::net_classes::write_net_classes(&poured_directory, board_id, &pcb, &config.net_classes)?;
        }
        poured_directory.as_path()
    };
    let swapped_directory = output_directory.with_extension("swapped");
    let (source_directory, pin_swaps) = match &config.pin_swaps {
        Some(path) => {
            let spec = read_pin_swap_spec(&source_directory.join(path)).map_err(crate::verdict::input_error)?;
            let result =
                swap_kicad_pins(source_directory, board_id, &swapped_directory, &spec, &config.pin_swap)?;
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
        let mut report = route_kicad_board_once(source_directory, board_id, output_directory, config, false)?;
        report.pin_swaps = pin_swaps;
        report.pours = "topological (pour nets as tracks)".into();
        return Ok(report);
    }
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    let source = fs::read_to_string(&source_board)
        .map_err(|error| format!("failed to read {}: {error}", source_board.display()))?;
    let parsed = parse(&source)?;
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
        (true, KiCadPourMode::Connect) => vec![(true, skeleton, exclusive.unwrap_or(false), stubs.unwrap_or(false))],
        (true, KiCadPourMode::Auto) if skeleton => vec![(true, true, false, false), (false, false, false, false)],
        (true, KiCadPourMode::Auto) if two_layers => {
            vec![(true, false, false, false), (true, true, false, false), (false, false, false, false)]
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
    // One router may take most of the memory available; routers side by
    // side (the seeds) half of it, since other work on the machine grows
    // too.
    let fits = |megabytes: f64, share: f64| -> bool {
        memory_mb().is_none_or(|(_, available)| megabytes < available * share)
    };
    let mut extra_rung_tried = false;
    // Steps that did not run, with why: the report's `skipped` when the
    // board stays incomplete.
    let mut skipped: Vec<String> = Vec::new();
    let ladder_budget = config.ladder_budget_seconds.unwrap_or(1200.0);
    let ladder_started = std::time::Instant::now();
    // The ladder's budget is spent in work (search expansions in seconds of
    // an idle machine), so that a busy machine routes the same way; the
    // wall clock counts only beyond `GUARD` times that.
    let mut ladder_work = 0.0f64;
    // `ladder_work` as the checks below read it.
    let ladder_work_now = std::cell::Cell::new(0.0f64);
    let work_of = |expansions: u64| expansions as f64 / core::router::EXPANSIONS_PER_SECOND;
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
    let past_deadline = || {
        let past = config.deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline);
        if past {
            core::router::note_wall_cut("deadline", format_args!("the ladder at {:.0} s of its work", ladder_work_now.get()));
        }
        past
    };
    // What the last attempt took, in work: no attempt starts with less
    // than that left before the run's nominal end (an attempt checks the
    // deadline only in its negotiation; MokyaLora's last rung started at
    // 1560 s and ran to 1890 s, past the harness's limit).
    let last_attempt_work = std::cell::Cell::new(120.0f64);
    // Its lattice pitch (0: none yet), for projecting a finer rung.
    let last_attempt_pitch = std::cell::Cell::new(0.0f64);
    let work_left = || config.deadline_work.map_or(f64::INFINITY, |left| left - ladder_work_now.get());
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
    let reserve_cap = config.deadline_work.map_or(0.0, |left| (0.4 * left).min(600.0));
    let mut reserve_set = false;
    let no_time_for_another = || work_left() - reserve.get() < last_attempt_work.get().max(120.0);
    // The reserve is a floor (2026-10-15): once kept, no rung, finer pitch
    // or seed round starts unless its projected work leaves the whole
    // reserve, and none may run into it (their work deadlines end where
    // it begins); the step negotiates at least what the first attempt did.
    // It used to be checked against the last attempt's work only, so a
    // finer pitch (four times a rung at half the pitch) or a rung run to
    // the pitch's deadline could eat it, and the step needed half of it.
    // `PCB_LADDER_OLD_RESERVE` restores that.
    let reserve_floor = std::env::var_os("PCB_LADDER_OLD_RESERVE").is_none();
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
    if config.probe_ladder.unwrap_or(true) && modes.len() >= 3 && config.negotiation_seconds.is_none() {
        let probe_seconds = config.probe_seconds.unwrap_or(75.0);
        let layer_names = LayerTable::from_pcb(&parsed)?.names;
        let mut leader: Option<(usize, usize, core::router::Router, core::Board, KiCadBoardRouterConfig)> = None;
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
            let connections: usize = board.nets.iter().map(|net| net.terminals.len().saturating_sub(1)).sum();
            if mode == 0 && (board.layer_count < 4 || connections < 120) {
                break;
            }
            rungs.push((mode, board, attempt));
        }
        if !rungs.is_empty() && (past_deadline() || work_left() < probe_seconds * 3.0 + 180.0) {
            eprintln!("deadline too near for the ladder: no attempts");
            skipped.push("the rung probes did not run: too little work left before the deadline".into());
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
                let router = core::router::Router::new(&rungs[next].1, &core_config(&rungs[next].2));
                let megabytes = (resident_mb() - before).max(1.0);
                first = Some((router, started.elapsed().as_secs_f64()));
                router_mb_estimate = Some(megabytes);
            }
            // The search scratches and the via reduction's copy come on
            // top of the maps a new router holds.
            let side_by_side = memory_mb()
                .zip(router_mb_estimate)
                .map_or(1, |((_, available), megabytes)| (available * 0.5 / (megabytes * 1.5)) as usize)
                .clamp(1, rungs.len() - next)
                // A cap for experiments (1: one after the other).
                .min(std::env::var("PCB_PROBES_SIDE_BY_SIDE").ok().and_then(|value| value.parse().ok()).unwrap_or(usize::MAX));
            let wave = &rungs[next..next + side_by_side];
            let first_router = first.take();
            let probed: Vec<(usize, usize, core::router::Router, f64)> = std::thread::scope(|scope| {
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
                            let unfinished = router.probe(probe_seconds);
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
                handles.into_iter().map(|handle| handle.join().expect("probe thread")).collect()
            });
            let mut out_of_work = false;
            for ((mode, unfinished, mut router, seconds), (_, board, attempt)) in probed.into_iter().zip(wave) {
                // As one probe after the other: a probe the work left would
                // not have started counts for nothing, however many ran side
                // by side (that number follows the memory available).
                if leader.is_some() && work_left() < probe_seconds * 2.0 + 120.0 {
                    out_of_work = true;
                    break;
                }
                let (connect, _, exclusive, plane_stubs) = modes[mode];
                ladder_work += work_of(router.expansions());
                ladder_work_now.set(ladder_work);
                eprintln!(
                    "probe pours={}{}: {unfinished} nets unfinished after {:.0} s{}",
                    if connect { "connect" } else { "tracks" },
                    if exclusive { " (exclusive planes)" } else if plane_stubs { " (plane stubs)" } else { "" },
                    seconds,
                    if side_by_side > 1 { format!(" ({side_by_side} side by side)") } else { String::new() }
                );
                // Experiment hook: continue this rung whatever the probes say.
                let forced = std::env::var("PCB_LADDER_FORCE_RUNG").ok().and_then(|value| value.parse::<usize>().ok());
                if forced.map_or(leader.as_ref().is_none_or(|(best, ..)| unfinished < *best), |rung| rung == mode) {
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
            let remaining = (ladder_budget - spent(ladder_work)).max(60.0);
            // Experiment hook: the open count after a short continuation
            // of a copy (no polish), another statistic a judge could use.
            if let Some(seconds) = std::env::var("PCB_LADDER_SHORT_RESUME").ok().and_then(|value| value.parse::<f64>().ok()) {
                let mut copy = router.clone();
                let short = copy.resume_polished(seconds, false);
                let open: usize = short
                    .status
                    .iter()
                    .map(|status| match status {
                        core::NetStatus::Partial { unconnected_terminals } => *unconnected_terminals,
                        core::NetStatus::Unreachable => 1,
                        _ => 0,
                    })
                    .sum();
                eprintln!("ladder stats: rung {mode} short resume {seconds} s: {open} open");
            }
            let probed = router.expansions();
            // The router's own probe is in its work already.
            router.config_mut().run_work = Some((ladder_budget, ladder_budget - spent(ladder_work) + work_of(probed)));
            // It stops where the run's work ends, not where the wall
            // clock does (the deadline stays as the guard against load).
            if config.deadline_work.is_some() {
                router.config_mut().deadline_work = Some(work_of(probed) + work_left().max(0.0));
            }
            let resumed = (remaining / 3.0).clamp(60.0, 900.0);
            let routed = router.resume(resumed);
            let router_expansions = router.expansions();
            ladder_work += work_of(router_expansions - probed);
            ladder_work_now.set(ladder_work);
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
                &attempt,
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
            let opens = open(&result, output_directory);
            eprintln!(
                "attempt pours={}{} pitch=regular (probed, continued): {} open, {} starved, {} vias, {:.1} s",
                result.pours,
                if exclusive { " (exclusive planes)" } else if plane_stubs { " (plane stubs)" } else { "" },
                opens.0,
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
                reserve.set(if reserve_floor {
                    narrow_step_reserve_floored(work_of(router_expansions), reserve_share, reserve_cap)
                } else {
                    narrow_step_reserve(work_of(router_expansions), reserve_share, reserve_cap)
                });
                eprintln!(
                    "ladder: {:.0} s of work kept for the narrow-signal step ({:.0} s left, the attempt took {:.0} s)",
                    reserve.get(),
                    work_left(),
                    work_of(router_expansions)
                );
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
            let remaining = (ladder_budget - spent(ladder_work)).max(0.0);
            (remaining / modes.len() as f64 / 3.0).clamp(60.0, 900.0)
        });
        let pitch_left = (ladder_budget - spent(ladder_work)).max(0.0);
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
            attempt.router_deadline_work = config.deadline_work.map(|_| {
                if reserve_floor && reserve.get() > 0.0 {
                    deadline_before_reserve(pitch_work_left, reserve.get(), 1)
                } else {
                    pitch_work_left
                }
            });
            if let Some(pitch) = pitch {
                attempt.grid_pitches_mm = Some(pitch.clone());
            }
            attempt
        };
        let rung_directory = |mode: usize| output_directory.with_extension(format!("rung{mode}"));
        // A probed rung continued with what it had is not run again; the
        // other rungs still get their turn when it left connections open.
        let candidates: Vec<usize> =
            (0..modes.len()).filter(|mode| !(pitch.is_none() && probed_mode == Some(*mode))).collect();
        let mut ran: Vec<Option<Result<KiCadBoardRouterResult, String>>> = (0..modes.len()).map(|_| None).collect();
        for (position, &mode) in candidates.iter().enumerate() {
            let (connect, skeleton, exclusive, plane_stubs) = &modes[mode];
            // With work kept for the narrow-signal step, the rung's work
            // as projected from the last attempt (on a finer lattice, by
            // the square of the pitch ratio: DSP-ADAU1452's 0.05 mm rung
            // after its 0.075 mm rungs) must leave all of it.
            let short_of_work = if reserve_floor && reserve.get() > 0.0 {
                let cost = pitch.as_ref().map_or(last_attempt_work.get(), |pitch| {
                    projected_attempt_work(last_attempt_work.get(), last_attempt_pitch.get(), pitch[0])
                });
                !fits_before_reserve(work_left(), reserve.get(), cost)
            } else {
                no_time_for_another()
            };
            if best.is_some() && (past_deadline() || short_of_work) {
                eprintln!("deadline reached: no further attempts");
                skipped.push("further rungs of the attempt ladder did not run: the run's end was near (its deadline, or too little work left for another attempt)".into());
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
            let attempt = rung_attempt(mode);
            if ran[mode].is_none() {
                // The first rung of a pitch without a board yet runs alone
                // (it often completes the board); after it, the rest run
                // side by side, each a router of this pitch and a KiCad
                // check.
                let side_by_side = if best.is_none() {
                    1
                } else {
                    let each = router_mb.map_or(0.0, |(megabytes, previous)| {
                        megabytes * (previous / pitch.as_ref().map_or(previous, |pitch| pitch[0])).powi(2)
                    }) + 2200.0;
                    (1..=candidates.len() - position).rev().find(|count| fits(each * *count as f64, 0.5)).unwrap_or(1)
                };
                let wave: Vec<usize> = candidates[position..position + side_by_side].to_vec();
                if wave.len() > 1 {
                    eprintln!("ladder: {} rungs side by side", wave.len());
                }
                let routed: Vec<(usize, Result<KiCadBoardRouterResult, String>)> = std::thread::scope(|scope| {
                    let handles: Vec<_> = wave
                        .iter()
                        .map(|&other| {
                            let attempt = rung_attempt(other);
                            let directory = rung_directory(other);
                            let connect = modes[other].0;
                            scope.spawn(move || {
                                if directory.exists() {
                                    fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
                                }
                                route_kicad_board_once(source_directory, board_id, &directory, &attempt, connect)
                            })
                        })
                        .collect();
                    wave.iter().copied().zip(handles.into_iter().map(|handle| handle.join().expect("rung thread"))).collect()
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
            let mut opens = open(&result, &directory);
            if router_mb.is_none() {
                router_mb = memory_mb().map(|(peak, _)| (peak, result.grid_pitch_mm));
            }
            // Seeds run side by side, each a router of this pitch, as many
            // at once as fit half the memory available; how many seeds
            // route does not depend on it.
            let seeds_that_fit = |wanted: usize| -> usize {
                let Some((megabytes, previous)) = router_mb else {
                    return wanted;
                };
                let each = megabytes * (previous / result.grid_pitch_mm).powi(2);
                (1..=wanted).rev().find(|count| fits(each * *count as f64, 0.5)).unwrap_or(1)
            };
            // What one attempt at this pitch costs, for projecting finer
            // pitches; a seed retry does not change it.
            // In work, as the ladder's budget.
            let attempt_seconds = work_of(result.expansions);
            ladder_work += attempt_seconds;
            ladder_work_now.set(ladder_work);
            // Open connections often depend on the order: route the same
            // attempt a few perturbed ways and keep the better board.
            let retry = config.retry_seeds.unwrap_or(4);
            let retry_side_by_side = seeds_that_fit(retry);
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
                && (reserve.get() == 0.0 || work_left() - reserve.get() >= retry as f64 * attempt_seconds)
            {
                seeds_tried = true;
                let mut seeded = attempt.clone();
                seeded.seeds = Some(retry);
                seeded.seeds_side_by_side = Some(retry_side_by_side);
                seeded.first_seed = Some(1);
                // Their work counts `retry` times: each stops at its share
                // of what is left beyond the reserve.
                if reserve_floor && reserve.get() > 0.0 {
                    seeded.router_deadline_work = Some(deadline_before_reserve(work_left(), reserve.get(), retry));
                }
                let retry_directory = output_directory.with_extension("seeds");
                if retry_directory.exists() {
                    fs::remove_dir_all(&retry_directory).map_err(|error| error.to_string())?;
                }
                let mut other =
                    route_kicad_board_once(source_directory, board_id, &retry_directory, &seeded, *connect)?;
                other.pours = result.pours.clone();
                // The seeds run side by side: about `retry` times one's work.
                ladder_work += work_of(other.expansions) * retry as f64;
                ladder_work_now.set(ladder_work);
                let other_opens = open(&other, &retry_directory);
                eprintln!(
                    "attempt again with {retry} seeds: {} open, {} vias, {:.1} s",
                    other_opens.0, other.vias, other.routing_seconds
                );
                if (other_opens, other.vias, other.length_mm)
                    .partial_cmp(&(opens, result.vias, result.length_mm))
                    .is_some_and(|order| order.is_lt())
                {
                    fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
                    fs::rename(&retry_directory, &directory).map_err(|error| error.to_string())?;
                    other.routing_seconds += result.routing_seconds;
                    result = other;
                    opens = other_opens;
                } else {
                    fs::remove_dir_all(&retry_directory).map_err(|error| error.to_string())?;
                }
            }
            eprintln!(
                "attempt pours={}{} pitch={}: {} open, {} starved, {} vias, {:.1} s",
                result.pours,
                if *exclusive { " (exclusive planes)" } else if *plane_stubs { " (plane stubs)" } else { "" },
                pitch
                    .as_ref()
                    .map_or("regular".to_string(), |pitch| format!("{:?}", pitch)),
                opens.0,
                opens.1,
                result.vias,
                result.routing_seconds
            );
            last_attempt_work.set(attempt_seconds);
            last_attempt_pitch.set(result.grid_pitch_mm);
            if reserve_wanted && !reserve_set && opens.0 > 0 {
                reserve_set = true;
                first_negotiation = attempt.negotiation_seconds.unwrap_or(900.0);
                reserve.set(if reserve_floor {
                    narrow_step_reserve_floored(attempt_seconds, reserve_share, reserve_cap)
                } else {
                    narrow_step_reserve(attempt_seconds, reserve_share, reserve_cap)
                });
                eprintln!(
                    "ladder: {:.0} s of work kept for the narrow-signal step ({:.0} s left, the attempt took {attempt_seconds:.0} s)",
                    reserve.get(),
                    work_left()
                );
            }
            let seconds = attempt_seconds;
            let used = (attempt_seconds, result.grid_pitch_mm);
            if slowest.is_none_or(|(seconds, _)| used.0 > seconds) {
                slowest = Some(used);
            }
            // Equally open boards: fewer vias, then less copper.
            let better = best.as_ref().is_none_or(|(best_opens, best_result)| {
                (opens, result.vias, result.length_mm)
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
            if reserve_floor && best.as_ref().is_some_and(|(opens, _)| opens.0 == 0) && reserve.get() > 0.0 {
                eprintln!("ladder: the board is complete: the {:.0} s kept for the narrow-signal step are released", reserve.get());
                reserve.set(0.0);
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
            if spent(ladder_work) > ladder_budget {
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
        let mut affordable = ((ladder_budget - spent(ladder_work) - cost) / cost).floor().max(0.0) as usize;
        if reserve.get() > 0.0 {
            affordable = affordable.min(((work_left() - reserve.get()) / cost).floor().max(0.0) as usize);
        }
        let retry = config.retry_seeds.unwrap_or(4).min(affordable);
        // As many at once as fit half the memory available; how many seeds
        // route does not depend on it.
        let side_by_side = router_mb.map_or(retry, |(megabytes, previous)| {
            let each = megabytes * (previous / best_result.grid_pitch_mm).powi(2);
            (1..=retry).rev().find(|count| fits(each * *count as f64, 0.5)).unwrap_or(1)
        });
        if opens.0 == 0 || best_seeded || retry == 0 || past_deadline() || no_time_for_another() {
            break;
        }
        let mut seeded = attempt.clone();
        seeded.seeds = Some(retry);
        seeded.seeds_side_by_side = Some(side_by_side);
        seeded.first_seed = Some(next_seed);
        if reserve_floor && reserve.get() > 0.0 {
            seeded.router_deadline_work = Some(deadline_before_reserve(work_left(), reserve.get(), retry));
        }
        next_seed += retry as u64;
        if scratch.exists() {
            fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
        }
        let connect = *connect;
        let mut other = route_kicad_board_once(source_directory, board_id, &scratch, &seeded, connect)?;
        other.pours = best_result.pours.clone();
        ladder_work += work_of(other.expansions) * retry as f64;
        ladder_work_now.set(ladder_work);
        let other_opens = open(&other, &scratch);
        eprintln!(
            "best rung again with {retry} seeds: {} open, {} starved, {} vias, {:.1} s",
            other_opens.0, other_opens.1, other.vias, other.routing_seconds
        );
        let better = (other_opens, other.vias, other.length_mm)
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
    let step_has_time = || if kept > 0.0 { work_left() >= narrow_step_minimum(kept) } else { !no_time_for_another() };
    let step_budget = |step: &mut KiCadBoardRouterConfig| {
        if kept > 0.0 {
            step.negotiation_seconds = Some(if reserve_floor {
                narrow_step_negotiation_floored(work_left(), step.negotiation_seconds, first_negotiation, kept)
            } else {
                narrow_step_negotiation(work_left(), step.negotiation_seconds)
            });
            step.run_work = Some((ladder_budget, work_left().max(0.0)));
            // The attempt it varies may carry its pitch's work deadline,
            // taken when that pitch started: the step's is what is left now.
            step.router_deadline_work = Some(work_left().max(0.0));
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
            } else {
                "too little work was left for another attempt"
            })
        } else {
            None
        }
    };
    if let (Some((opens, _)), Some(_), Some(narrow)) = (best.as_ref(), best_attempt.as_ref(), config.narrow_signal_mm)
        && opens.0 > 0
        && config.use_narrow_signals.is_none()
        && config.connection_rules.values().chain(config.default_rules.iter()).any(|rules| rules.trace_width_mm > narrow + 1.0e-9)
        && let Some(why) = blocked()
    {
        eprintln!("the narrow-signal step did not run: {why} ({:.0} s of work left)", work_left());
        skipped.push(format!("the narrow-signal step (signals at {narrow} mm) did not run: {why}"));
    }
    // Connections still open at the class widths: the best attempt once
    // more with signal nets at the project's smallest predefined width
    // (designers often keep the default class at a power width and draw
    // signals narrower).
    if let (Some((opens, _)), Some((attempt, connect)), Some(narrow)) =
        (best.as_ref(), best_attempt.as_ref(), config.narrow_signal_mm)
        && opens.0 > 0
        && config.use_narrow_signals.is_none()
        && spent(ladder_work) < 1.5 * ladder_budget
        && !past_deadline()
        && step_has_time()
        && config.connection_rules.values().chain(config.default_rules.iter()).any(|rules| rules.trace_width_mm > narrow + 1.0e-9)
    {
        let mut narrowed = attempt.clone();
        narrowed.use_narrow_signals = Some(true);
        step_budget(&mut narrowed);
        // A project that takes clearance findings as warnings gets its
        // minimum clearance in the same attempt (the A13 module's designer
        // drew 0.1016 tracks 0.149 from balls in classes of 0.2).
        if let Some(relaxed) = config.relaxed_clearance_mm {
            narrowed.relaxed_clearance_mm = None;
            for rules in narrowed.connection_rules.values_mut().chain(narrowed.default_rules.iter_mut()) {
                rules.clearance_mm = rules.clearance_mm.min(relaxed);
            }
            narrow_step_relaxed = true;
        }
        if scratch.exists() {
            fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
        }
        let mut result = route_kicad_board_once(source_directory, board_id, &scratch, &narrowed, *connect)?;
        ladder_work += work_of(result.expansions);
        ladder_work_now.set(ladder_work);
        result.pours = best.as_ref().map(|(_, best)| best.pours.clone()).unwrap_or_default();
        let narrow_opens = open(&result, &scratch);
        eprintln!(
            "attempt with signals at {narrow} mm{}: {} open, {} starved, {} vias, {:.1} s, work {:.0} s",
            config.relaxed_clearance_mm.map(|relaxed| format!(" and clearance {relaxed} mm")).unwrap_or_default(),
            narrow_opens.0, narrow_opens.1, result.vias, result.routing_seconds, work_of(result.expansions)
        );
        let better = best.as_ref().is_some_and(|(best_opens, best_result)| {
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
    if let (Some((opens, _)), Some(_), Some(relaxed)) = (best.as_ref(), best_attempt.as_ref(), config.relaxed_clearance_mm)
        && opens.0 > 0
        && !narrow_step_relaxed
        && config.connection_rules.values().chain(config.default_rules.iter()).any(|rules| rules.clearance_mm > relaxed + 1.0e-9)
        && let Some(why) = blocked()
    {
        skipped.push(format!("the minimum-clearance step (clearance {relaxed} mm) did not run: {why}"));
    }
    if let (Some((opens, _)), Some((attempt, connect)), Some(relaxed)) =
        (best.as_ref(), best_attempt.as_ref(), config.relaxed_clearance_mm)
        && opens.0 > 0
        && !narrow_step_relaxed
        && spent(ladder_work) < 1.5 * ladder_budget
        && !past_deadline()
        && step_has_time()
        && config.connection_rules.values().chain(config.default_rules.iter()).any(|rules| rules.clearance_mm > relaxed + 1.0e-9)
    {
        let mut relaxed_attempt = attempt.clone();
        step_budget(&mut relaxed_attempt);
        relaxed_attempt.relaxed_clearance_mm = None;
        for rules in relaxed_attempt.connection_rules.values_mut().chain(relaxed_attempt.default_rules.iter_mut()) {
            rules.clearance_mm = rules.clearance_mm.min(relaxed);
        }
        if scratch.exists() {
            fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
        }
        let mut result = route_kicad_board_once(source_directory, board_id, &scratch, &relaxed_attempt, *connect)?;
        ladder_work += work_of(result.expansions);
        ladder_work_now.set(ladder_work);
        result.pours = best.as_ref().map(|(_, best)| best.pours.clone()).unwrap_or_default();
        let relaxed_opens = open(&result, &scratch);
        eprintln!(
            "attempt at the minimum clearance {relaxed} mm: {} open, {} starved, {} vias, {:.1} s",
            relaxed_opens.0, relaxed_opens.1, result.vias, result.routing_seconds
        );
        let better = best.as_ref().is_some_and(|(best_opens, best_result)| {
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
            let text = fs::read_to_string(&board_path).map_err(|error| format!("failed to read {}: {error}", board_path.display()))?;
            let mut pcb = parse(&text)?;
            let mut changed = 0;
            if let Expr::List(items) = &mut pcb {
                for footprint in items.iter_mut().filter(|item| item.head() == Some("footprint")) {
                    let reference = footprint_reference(footprint).unwrap_or_default().to_string();
                    let Expr::List(children) = footprint else { continue };
                    for pad in children.iter_mut().filter(|child| child.head() == Some("pad")) {
                        let number = pad.children().get(1).and_then(Expr::atom).unwrap_or("").to_string();
                        if !starved.iter().any(|(r, n)| *r == reference && *n == number) {
                            continue;
                        }
                        if let Expr::List(fields) = pad {
                            fields.retain(|field| field.head() != Some("zone_connect"));
                            fields.push(Expr::List(vec![Expr::Atom("zone_connect".into()), Expr::Atom("2".into())]));
                            changed += 1;
                        }
                    }
                }
            }
            if changed > 0 {
                fs::write(&board_path, format!("{}\n", encode(&pcb)))
                    .map_err(|error| format!("failed to write {}: {error}", board_path.display()))?;
                result.native = Some(verify_materialized_rung(output_directory, board_id)?);
                result.diagnostics.starved_thermals = starved_thermal_diagnostics(output_directory);
                eprintln!(
                    "{changed} starved pads connected solid: {} starved thermals left",
                    result.diagnostics.starved_thermals.len()
                );
            }
        }
    }
    repair_necks(&mut result, output_directory, board_id, config)?;
    result.cut = core::router::wall_cuts();
    result.work_seconds = ladder_work;
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadPourRequest {
    /// The net, as in the board (`GND` or `/GND`).
    pub net: String,
    /// Copper layers, such as `B.Cu`.
    pub layers: Vec<String>,
    /// Clearance between the pour and other nets (default 0.3 mm).
    #[serde(default)]
    pub clearance_mm: Option<f64>,
}

/// The pours to add to `pcb`: the requested ones, or else the automatic
/// planes of a multilayer board (see `automatic_planes`).
pub(crate) fn planned_pours(pcb: &Expr, config: &KiCadBoardRouterConfig) -> Result<Vec<KiCadPourRequest>, String> {
    if !config.add_pours.is_empty() {
        return Ok(config.add_pours.clone());
    }
    if config.automatic_planes == Some(false) {
        return Ok(Vec::new());
    }
    let layers = LayerTable::from_pcb(pcb)?;
    let inner: Vec<&String> = layers.names.iter().filter(|name| name.starts_with("In")).collect();
    if inner.len() < 2 || !pours(pcb, &layers)?.is_empty() {
        return Ok(Vec::new());
    }
    let mut pads: BTreeMap<String, usize> = BTreeMap::new();
    for footprint in pcb.children().iter().filter(|item| item.head() == Some("footprint")) {
        for pad in footprint.children().iter().filter(|item| item.head() == Some("pad")) {
            if let Some(net) = node_net(pad).filter(|net| routable_net(net)) {
                *pads.entry(net.to_string()).or_default() += 1;
            }
        }
    }
    let busiest = |accept: &dyn Fn(&str) -> bool| {
        pads.iter().filter(|(net, count)| accept(net) && **count >= 3).max_by_key(|(_, count)| **count).map(|(net, _)| net.clone())
    };
    let mut requests = Vec::new();
    if let Some(ground) = busiest(&|net| crate::quality::ground(net)) {
        requests.push(KiCadPourRequest { net: ground, layers: vec![inner[0].clone()], clearance_mm: None });
    }
    if let Some(rail) = busiest(&|net| crate::quality::rail_name(net) && !crate::quality::ground(net)) {
        requests.push(KiCadPourRequest { net: rail, layers: vec![inner[inner.len() - 1].clone()], clearance_mm: None });
    }
    Ok(requests)
}

/// Adds a zone over the board outline for every request.
pub(super) fn add_pour_zones(pcb: &mut Expr, requests: &[KiCadPourRequest]) -> Result<(), String> {
    if requests.is_empty() {
        return Ok(());
    }
    let outline = outline::board_loops(pcb)?.outline;
    let layers = LayerTable::from_pcb(pcb)?;
    let mut nets = BTreeSet::new();
    for footprint in pcb.children().iter().filter(|item| item.head() == Some("footprint")) {
        for pad in footprint.children().iter().filter(|child| child.head() == Some("pad")) {
            if let Some(net) = node_net(pad) {
                nets.insert(net.to_string());
            }
        }
    }
    // Zones the board already has, by (net, layer): a requested pour that
    // exists already is not added twice.
    let mut existing = BTreeSet::new();
    for zone in pcb.children().iter().filter(|item| item.head() == Some("zone")) {
        let Some(net) = form_atom(zone, "net_name", 1).or_else(|| node_net(zone)) else {
            continue;
        };
        let names: Vec<String> = zone
            .child("layers")
            .map(|layers| layers.children().iter().skip(1).filter_map(Expr::atom).map(str::to_owned).collect())
            .or_else(|| form_atom(zone, "layer", 1).map(|layer| vec![layer.to_string()]))
            .unwrap_or_default();
        for layer in names {
            existing.insert((normalize_net(net).to_string(), layer));
        }
    }
    let mut zones = Vec::new();
    for request in requests {
        let net = [request.net.clone(), format!("/{}", request.net)]
            .into_iter()
            .find(|name| nets.contains(name))
            .ok_or_else(|| format!("pour for {:?}: no pad has that net", request.net))?;
        let clearance = request.clearance_mm.unwrap_or(0.3);
        for layer in &request.layers {
            if layers.index(layer).is_none() {
                return Err(format!("pour for {:?}: {layer:?} is not a copper layer of the board", request.net));
            }
            if existing.contains(&(normalize_net(&net).to_string(), layer.clone())) {
                eprintln!("pour for {} on {layer}: the board has one already", request.net);
                continue;
            }
            let points: Vec<String> = outline.iter().map(|point| format!("(xy {} {})", point[0], point[1])).collect();
            // A stable identifier from the request.
            let mut hash = 0xcbf2_9ce4_8422_2325u64;
            for byte in net.bytes().chain(layer.bytes()) {
                hash = (hash ^ byte as u64).wrapping_mul(0x100_0000_01b3);
            }
            zones.push(parse(&format!(
                "(zone (net {net:?}) (layer {layer:?}) (uuid \"{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}\") (hatch edge 0.5) \
                 (connect_pads (clearance {clearance})) (min_thickness 0.25) \
                 (fill yes (thermal_gap {clearance}) (thermal_bridge_width 0.3) (island_removal_mode 0)) \
                 (polygon (pts {})))",
                hash >> 32,
                (hash >> 16) & 0xffff,
                hash & 0xfff,
                (hash >> 12) & 0xfff,
                hash & 0xffff_ffff_ffff,
                points.join(" ")
            ))?);
        }
    }
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    items.extend(zones);
    Ok(())
}

/// Routes `board` with seeds `first..first + count` in parallel (seed 0 is
/// the deterministic order) and returns the best result.
fn route_seeds(board: &core::Board, config: &core::Config, first: u64, count: usize, side_by_side: usize) -> core::RoutingResult {
    // In waves of `side_by_side` (memory), in seed order.
    let seeds: Vec<u64> = (first..first + count.max(1) as u64).collect();
    let mut results: Vec<(Option<u64>, core::RoutingResult)> = Vec::new();
    for wave in seeds.chunks(side_by_side.max(1)) {
        let routed: Vec<(Option<u64>, core::RoutingResult)> = std::thread::scope(|scope| {
            let handles: Vec<_> = wave
                .iter()
                .map(|&index| {
                    let mut seeded = config.clone();
                    seeded.seed = (index > 0).then_some(index);
                    seeded.verbose = config.verbose && index == first;
                    scope.spawn(move || (seeded.seed, core::route(board, &seeded)))
                })
                .collect();
            handles.into_iter().map(|handle| handle.join().expect("routing thread")).collect()
        });
        results.extend(routed);
    }
    let key = |result: &core::RoutingResult| {
        let open: usize = result
            .status
            .iter()
            .map(|status| match status {
                core::NetStatus::Partial { unconnected_terminals } => *unconnected_terminals,
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
    let scored: Vec<_> = results.into_iter().map(|(seed, result)| (key(&result), seed, result)).collect();
    for (score, seed, _) in &scored {
        eprintln!("seed {seed:?}: {} open, {} vias, {:.0} mm", score.0, score.1, score.2);
    }
    scored
        .into_iter()
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .expect("at least one seed")
        .2
}

/// The core router configuration for a KiCad configuration.
pub(super) fn core_config(config: &KiCadBoardRouterConfig) -> core::Config {
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
        router_config.negotiation_expansions = (seconds * core::router::EXPANSIONS_PER_SECOND) as u64;
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
        router_config.negotiation_expansions = (seconds * core::router::EXPANSIONS_PER_SECOND) as u64;
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
    if let Some(batch) = config.jacobi_batch {
        router_config.jacobi_batch = batch;
    }
    if let Some(skeleton) = config.plane_skeleton {
        router_config.plane_skeleton = skeleton;
    }
    if let Some(bias) = config.skeleton_bias {
        router_config.skeleton_bias = bias;
    }
    router_config
}

/// Writes a routing result as copper into `pcb` and reports per net.
pub(super) fn emit_routes(
    pcb: &mut Expr,
    board: &core::Board,
    result: &core::RoutingResult,
    layer_names: &[String],
) -> Result<Vec<KiCadBoardRouterNet>, String> {
    let copper_net_names = unambiguous_pad_net_names(pcb)?;
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    items.retain(|item| !matches!(item.head(), Some("segment" | "arc" | "via")));
    let mut nets = Vec::new();
    for (net, route) in result.routes.iter().enumerate() {
        let description = &board.nets[net];
        for segment in &route.segments {
            items.push(supplemental_segment_expr(&SupplementalSegment {
                connection: description.name.clone(),
                start: nanometres(segment.start),
                end: nanometres(segment.end),
                width: segment.width,
                layer: layer_names[segment.layer].clone(),
            }));
        }
        for via in &route.vias {
            items.push(supplemental_via_expr(&SupplementalVia {
                connection: description.name.clone(),
                at: nanometres(via.at),
                size: via.diameter,
                drill: via.drill,
                layers: ["F.Cu".into(), "B.Cu".into()],
            }));
        }
        let (status, unconnected_terminals) = match result.status[net] {
            core::NetStatus::Trivial => ("trivial", 0),
            core::NetStatus::Routed => ("routed", 0),
            core::NetStatus::Partial {
                unconnected_terminals,
            } => ("partial", unconnected_terminals),
            core::NetStatus::Unreachable => {
                ("unreachable", description.terminals.len().saturating_sub(1))
            }
        };
        nets.push(KiCadBoardRouterNet {
            connection: description.name.clone(),
            terminals: description.terminals.len(),
            status: status.into(),
            unconnected_terminals,
            vias: route.vias.len(),
            length_mm: route
                .segments
                .iter()
                .map(|segment| distance_squared(segment.start, segment.end).sqrt())
                .sum(),
        });
    }
    canonicalize_copper_net_names(pcb, &copper_net_names);
    Ok(nets)
}

/// Writes `pcb` (with its copper) as the project in `output_directory`,
/// verifies it natively unless told not to, and builds the report.
#[allow(clippy::too_many_arguments)]
pub(super) fn finish_routed_board(
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
        "board check: internal verification {verify_seconds:.1}s, KiCad {native_verification_seconds:.1}s (lowering {:.1}s, routing {:.1}s); drc_cache: {{calls {}, hits {}}}; so far {lowerings} lowerings {lowering_total:.1}s, {builds} lattices {build_total:.1}s",
        timings[0], timings[1], drc_cache.calls, drc_cache.hits
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
        skipped: Vec::new(),
        neck_repair: None,
        rules: KiCadRulesSummary::of(board, config),
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
pub(crate) fn repair_necks(
    result: &mut KiCadBoardRouterResult,
    directory: &Path,
    board_id: &str,
    config: &KiCadBoardRouterConfig,
) -> Result<(), String> {
    if config.skip_native_verification {
        return Ok(());
    }
    if let Some(repair) = crate::necks::repair_connection_width(directory, board_id, config)? {
        if repair.kept {
            result.native = Some(verify_materialized_rung(directory, board_id)?);
            result.diagnostics.starved_thermals = starved_thermal_diagnostics(directory);
        }
        result.neck_repair = Some(repair);
    }
    Ok(())
}

/// Number of `starved_thermal` findings in the directory's native DRC report.
pub(super) fn starved_thermals(directory: &Path) -> usize {
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

/// A fresh numbered subdirectory of `root` for one routing attempt's frames.
fn attempt_frame_directory(root: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(root)
        .map_err(|error| format!("failed to create {}: {error}", root.display()))?;
    let attempts = fs::read_dir(root)
        .map_err(|error| format!("failed to read {}: {error}", root.display()))?
        .count();
    let directory = root.join(format!("attempt-{attempts:02}"));
    fs::create_dir_all(&directory)
        .map_err(|error| format!("failed to create {}: {error}", directory.display()))?;
    Ok(directory)
}

fn route_kicad_board_once(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardRouterConfig,
    connect_pours: bool,
) -> Result<KiCadBoardRouterResult, String> {
    let started = std::time::Instant::now();
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    let source = fs::read_to_string(&source_board)
        .map_err(|error| format!("failed to read {}: {error}", source_board.display()))?;
    let mut pcb = parse(&source)?;
    let Lowered { board } = lower(&pcb, config, connect_pours)?;
    let layer_names = LayerTable::from_pcb(&pcb)?.names;
    let lowering_seconds = started.elapsed().as_secs_f64();
    let routing_started = std::time::Instant::now();
    let result = match &config.frame_directory {
        _ if config.engine.as_deref() == Some("topological") => {
            let mut topological = pcb_topo::Config {
                iterations: config.topological_rounds.unwrap_or(30),
                seed: config.first_seed.unwrap_or(1).max(1),
                verbose: true,
                ..pcb_topo::Config::default()
            };
            config.topological.apply(&mut topological);
            pcb_topo::route(&board, &topological)
        }
        None if config.seeds.unwrap_or(1) > 1 || config.first_seed.unwrap_or(0) > 0 => route_seeds(
            &board,
            &core_config(config),
            config.first_seed.unwrap_or(0),
            config.seeds.unwrap_or(1),
            config.seeds_side_by_side.unwrap_or(usize::MAX),
        ),
        None => core::route(&board, &core_config(config)),
        Some(root) => {
            let directory = attempt_frame_directory(root)?;
            let mut router = core::router::Router::new(&board, &core_config(config));
            let frame_pcb = pcb.clone();
            let frame_board = board.clone();
            let frame_layers = layer_names.clone();
            router.set_frame_hook(std::sync::Arc::new(move |iteration, snapshot| {
                let mut frame = frame_pcb.clone();
                let written = emit_routes(&mut frame, &frame_board, snapshot, &frame_layers)
                    .and_then(|_| {
                        let path = directory.join(format!("frame-{iteration:04}.kicad_pcb"));
                        fs::write(&path, format!("{}\n", encode(&frame))).map_err(|error| {
                            format!("failed to write {}: {error}", path.display())
                        })
                    });
                if let Err(error) = written {
                    eprintln!("frame {iteration}: {error}");
                }
            }));
            router.run()
        }
    };
    let routing_seconds = routing_started.elapsed().as_secs_f64();
    write_attempt(
        pcb,
        &board,
        result,
        &layer_names,
        source_directory,
        board_id,
        output_directory,
        config,
        [lowering_seconds, routing_seconds],
    )
}

/// Moves every `(layer "X")` that names a layer the board's layer table
/// does not define to `Cmts.User`. Some boards carry such items (a
/// `Rescue` layer from an old KiCad, a user layer removed later); KiCad's
/// editor asks what to do with them and kicad-cli refuses the board.
/// Returns how many were moved.
fn move_undefined_layers(pcb: &mut Expr) -> usize {
    let defined: std::collections::HashSet<String> = pcb
        .child("layers")
        .map(|layers| {
            layers
                .children()
                .iter()
                .skip(1)
                .flat_map(|entry| entry.children().iter().skip(1).take(3).filter_map(Expr::atom).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if defined.is_empty() {
        return 0;
    }
    fn walk(node: &mut Expr, defined: &std::collections::HashSet<String>, top: bool) -> usize {
        let Expr::List(items) = node else {
            return 0;
        };
        // The layer table itself and the stackup name layers by design.
        if !top && matches!(items.first().and_then(Expr::atom), Some("layers" | "stackup")) {
            return 0;
        }
        let mut moved = 0;
        if items.len() == 2 && items[0].atom() == Some("layer") {
            if let Some(name) = items[1].atom()
                && !defined.contains(name)
                && !name.contains('*')
                && !name.contains('&')
            {
                items[1] = Expr::Atom("\"Cmts.User\"".into());
                moved += 1;
            }
            return moved;
        }
        for item in items.iter_mut() {
            moved += walk(item, defined, false);
        }
        moved
    }
    walk(pcb, &defined, true)
}

/// Writes a routed board: the optional tightening, the copper into the
/// board file, and the checks (`finish_routed_board`).
#[allow(clippy::too_many_arguments)]
fn write_attempt(
    mut pcb: Expr,
    board: &core::Board,
    result: core::RoutingResult,
    layer_names: &[String],
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardRouterConfig,
    seconds: [f64; 2],
) -> Result<KiCadBoardRouterResult, String> {
    let [lowering_seconds, mut routing_seconds] = seconds;
    let moved = move_undefined_layers(&mut pcb);
    if moved > 0 {
        eprintln!("{moved} items on layers the board does not define moved to Cmts.User (KiCad would not load the board)");
    }
    let mut result = result;
    if config.tighten && config.engine.as_deref() != Some("topological") {
        let tightening = std::time::Instant::now();
        let (routes, report) = pcb_topo::tighten::tighten(&board, &result.routes);
        eprintln!(
            "tighten: {} of {} pieces pulled tight, {:.1} mm -> {:.1} mm, {:.2}s",
            report.tightened,
            report.pieces,
            report.length_before,
            report.length_after,
            tightening.elapsed().as_secs_f64()
        );
        result.routes = routes;
        routing_seconds += tightening.elapsed().as_secs_f64();
    }
    let nets = emit_routes(&mut pcb, board, &result, layer_names)?;
    finish_routed_board(
        &pcb,
        board,
        &result,
        nets,
        layer_names,
        source_directory,
        board_id,
        output_directory,
        config,
        [lowering_seconds, routing_seconds],
    )
}

#[cfg(test)]
mod pour_request_tests {
    use super::*;

    #[test]
    fn multilayer_boards_without_pours_get_ground_and_rail_planes() {
        let board = |layers: &str, zone: &str| {
            let pad = |number: usize, net: &str| format!(r#"(pad "{number}" smd rect (at {number} 0) (size 0.5 0.5) (layers "F.Cu") (net "{net}"))"#);
            let pads: String = (1..=4).map(|n| pad(n, "GND")).chain((5..=7).map(|n| pad(n, "+3V3"))).chain((8..=10).map(|n| pad(n, "/SIG"))).collect();
            parse(&format!(
                r#"(kicad_pcb (layers {layers} (25 "Edge.Cuts" user))
                  (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
                  (footprint "a" (at 5 5) {pads}) {zone})"#
            ))
            .unwrap()
        };
        let four = r#"(0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (2 "B.Cu" signal)"#;
        let config = KiCadBoardRouterConfig::default();
        let planned = planned_pours(&board(four, ""), &config).unwrap();
        let summary: Vec<(String, Vec<String>)> = planned.into_iter().map(|pour| (pour.net, pour.layers)).collect();
        assert_eq!(summary, vec![("GND".into(), vec!["In1.Cu".into()]), ("+3V3".into(), vec!["In2.Cu".into()])]);
        // Two layers, an existing pour, or switched off: none.
        assert!(planned_pours(&board(r#"(0 "F.Cu" signal) (2 "B.Cu" signal)"#, ""), &config).unwrap().is_empty());
        let zone = r#"(zone (net "GND") (layer "In1.Cu") (polygon (pts (xy 0 0) (xy 20 0) (xy 20 10))))"#;
        assert!(planned_pours(&board(four, zone), &config).unwrap().is_empty());
        let off = KiCadBoardRouterConfig { automatic_planes: Some(false), ..KiCadBoardRouterConfig::default() };
        assert!(planned_pours(&board(four, ""), &off).unwrap().is_empty());
    }

    #[test]
    fn exclusive_planes_keep_the_layers_nearest_the_outside_solid() {
        // A six-layer board whose ground pour covers every inner layer
        // (ESC mini): with a limit of two, In1 and In4 stay exclusive.
        let pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (8 "In3.Cu" signal) (10 "In4.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
              (footprint "a" (at 5 5) (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net "GND")) (pad "2" smd rect (at 2 0) (size 0.5 0.5) (layers "F.Cu") (net "/SIG")))
              (footprint "b" (at 15 5) (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net "GND")) (pad "2" smd rect (at 2 0) (size 0.5 0.5) (layers "F.Cu") (net "/SIG")))
              (zone (net "GND") (layers "In1.Cu" "In2.Cu" "In3.Cu" "In4.Cu") (polygon (pts (xy 0 0) (xy 20 0) (xy 20 10) (xy 0 10)))))"#,
        )
        .unwrap();
        let exclusive = |config: &KiCadBoardRouterConfig| -> Vec<usize> {
            let board = lower(&pcb, config, true).unwrap().board;
            let mut layers: Vec<usize> = board.planes.iter().filter(|plane| plane.exclusive).map(|plane| plane.layer).collect();
            layers.sort_unstable();
            layers
        };
        let rules = KiCadConnectionRoutingRules { trace_width_mm: 0.2, clearance_mm: 0.2, via_size_mm: 0.6, via_drill_mm: 0.3 };
        let all = KiCadBoardRouterConfig { default_rules: Some(rules.clone()), ..KiCadBoardRouterConfig::default() };
        assert_eq!(exclusive(&all), vec![1, 2, 3, 4]);
        let two = KiCadBoardRouterConfig { exclusive_layers: Some(2), ..all.clone() };
        assert_eq!(exclusive(&two), vec![1, 4]);
        let one = KiCadBoardRouterConfig { exclusive_layers: Some(1), ..all.clone() };
        assert_eq!(exclusive(&one), vec![1]);
    }

    #[test]
    fn a_zone_of_a_higher_priority_is_cut_from_the_nets_lower_zone_and_arcs_are_followed() {
        // Telemetry: GND zones along the round board's edge (priority 10,
        // drawn with arcs) over a whole-board GND zone (priority 1). KiCad
        // fills the lower zone less the higher one's outline; the arc side
        // is followed (it was left out, the outline a polygon through the
        // other corners).
        let pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
              (footprint "a" (at 5 5) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND")))
              (footprint "b" (at 15 15) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND")))
              (zone (net "GND") (layers "F.Cu") (priority 1) (polygon (pts (xy 0 0) (xy 20 0) (xy 20 20) (xy 0 20))))
              (zone (net "GND") (layers "F.Cu") (priority 10)
                (polygon (pts (xy 20 0) (xy 10 0) (arc (start 10 0) (mid 17.071068 2.928932) (end 20 10))))))"#,
        )
        .unwrap();
        let rules = KiCadConnectionRoutingRules { trace_width_mm: 0.2, clearance_mm: 0.2, via_size_mm: 0.6, via_drill_mm: 0.3 };
        let config = KiCadBoardRouterConfig { default_rules: Some(rules), ..KiCadBoardRouterConfig::default() };
        let board = lower(&pcb, &config, true).unwrap().board;
        assert_eq!(board.planes.len(), 2);
        let (low, high) = (&board.planes[0], &board.planes[1]);
        assert_eq!(low.above.len(), 1);
        assert!(high.above.is_empty());
        // The corner between the arc and the board's corner is the higher
        // zone's; a point between the arc and its chord is not.
        assert!(core::geometry::point_in_polygon([19.0, 1.0], &low.above[0]));
        assert!(!core::geometry::point_in_polygon([16.0, 4.0], &low.above[0]));
        assert!(high.polygon.len() > 10);
    }

    #[test]
    fn rule_areas_that_keep_fills_out_are_left_out_of_the_pours_on_their_layers() {
        // Sisu: rule areas that allow tracks and vias but no zone fill; the
        // pour model filled them where KiCad leaves them empty.
        let pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
              (footprint "a" (at 5 5) (pad "1" thru_hole circle (at 0 0) (size 1 1) (drill 0.5) (layers "*.Cu") (net "GND")))
              (footprint "b" (at 15 5) (pad "1" thru_hole circle (at 0 0) (size 1 1) (drill 0.5) (layers "*.Cu") (net "GND")))
              (zone (net "GND") (layers "In1.Cu" "In2.Cu") (polygon (pts (xy 0 0) (xy 20 0) (xy 20 10) (xy 0 10))))
              (zone (layers "In2.Cu") (name "no fill")
                (keepout (tracks allowed) (vias allowed) (pads allowed) (copperpour not_allowed) (footprints allowed))
                (polygon (pts (xy 8 2) (xy 12 2) (xy 12 8) (xy 8 8))))
              (zone (layers "In1.Cu") (name "no tracks")
                (keepout (tracks not_allowed) (vias allowed) (pads allowed) (copperpour allowed) (footprints allowed))
                (polygon (pts (xy 8 2) (xy 12 2) (xy 12 8) (xy 8 8)))))"#,
        )
        .unwrap();
        let rules = KiCadConnectionRoutingRules { trace_width_mm: 0.2, clearance_mm: 0.2, via_size_mm: 0.6, via_drill_mm: 0.3 };
        let config = KiCadBoardRouterConfig { default_rules: Some(rules), ..KiCadBoardRouterConfig::default() };
        let board = lower(&pcb, &config, true).unwrap().board;
        let excluded = |layer: usize| board.planes.iter().find(|plane| plane.layer == layer).unwrap().excluded.clone();
        assert!(excluded(1).is_empty());
        assert_eq!(excluded(2).len(), 1);
        assert!(core::geometry::point_in_polygon([10.0, 5.0], &excluded(2)[0]));
        // Neither keeps a track out of In2; the other one keeps them out
        // of In1 as before.
        assert!(!board.obstacles.iter().any(|obstacle| obstacle.label == "no fill"));
        assert!(board.obstacles.iter().any(|obstacle| obstacle.label == "no tracks" && obstacle.blocks_tracks));
    }

    #[test]
    fn fill_keepouts_off_copper_or_drawn_as_circles_do_not_refuse_the_board() {
        // TelemetryOnboard: rule areas that keep fills out, drawn as circles
        // (an outline of one arc) on F.SilkS and on F.Cu.
        let circle = |layer: &str, x: f64| {
            format!(
                r#"(zone (layer "{layer}") (keepout (tracks allowed) (vias allowed) (pads allowed) (copperpour not_allowed) (footprints allowed))
                (polygon (pts (arc (start {x} 5) (mid {} 5) (end {x} 5)))))"#,
                x + 2.0
            )
        };
        let pcb = parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user) (37 "F.SilkS" user))
              (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
              (footprint "a" (at 2 2) (pad "1" thru_hole circle (at 0 0) (size 1 1) (drill 0.5) (layers "*.Cu") (net "GND")))
              (footprint "b" (at 18 8) (pad "1" thru_hole circle (at 0 0) (size 1 1) (drill 0.5) (layers "*.Cu") (net "GND")))
              (zone (net "GND") (layer "F.Cu") (polygon (pts (xy 0 0) (xy 20 0) (xy 20 10) (xy 0 10))))
              {} {})"#,
            circle("F.SilkS", 3.0),
            circle("F.Cu", 9.0)
        ))
        .unwrap();
        let rules = KiCadConnectionRoutingRules { trace_width_mm: 0.2, clearance_mm: 0.2, via_size_mm: 0.6, via_drill_mm: 0.3 };
        let config = KiCadBoardRouterConfig { default_rules: Some(rules), ..KiCadBoardRouterConfig::default() };
        let board = lower(&pcb, &config, true).unwrap().board;
        let excluded = &board.planes.iter().find(|plane| plane.layer == 0).unwrap().excluded;
        assert_eq!(excluded.len(), 1);
        assert!(core::geometry::point_in_polygon([10.0, 5.0], &excluded[0]));
        assert!(!core::geometry::point_in_polygon([4.0, 5.0], &excluded[0]));
    }

    #[test]
    fn mask_openings_keep_copper_half_the_minimum_mask_width_away() {
        let pcb = parse(
            r#"(kicad_pcb
              (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user) (39 "F.Mask" user) (38 "B.Mask" user))
              (setup (solder_mask_min_width 0.1))
              (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
              (footprint "a" (at 12 5)
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/A"))
                (pad "2" smd rect (at 4 0) (size 1 1) (layers "F.Cu") (net "/A")))
              (gr_rect (start 4 4) (end 6 6) (stroke (width 0) (type solid)) (fill yes) (layer "F.Mask")))"#,
        )
        .unwrap();
        let rules = KiCadConnectionRoutingRules { trace_width_mm: 0.2, clearance_mm: 0.2, via_size_mm: 0.6, via_drill_mm: 0.3 };
        let config = KiCadBoardRouterConfig { default_rules: Some(rules), ..KiCadBoardRouterConfig::default() };
        let board = lower(&pcb, &config, false).unwrap().board;
        let opening = board.obstacles.iter().find(|obstacle| obstacle.label.starts_with("solder mask opening")).unwrap();
        assert!((opening.clearance - 0.06).abs() < 1.0e-9, "{}", opening.clearance);
    }

    #[test]
    fn mask_openings_keep_the_solder_mask_to_copper_clearance_and_their_expansion() {
        // KiCad's bridge test: copper within the project's solder-mask-to-
        // copper clearance plus the aperture's expansion (a graphic's own
        // margin, else the board's pad_to_mask_clearance) is exposed.
        // USB_Cable_Tester: 0.05 + 0.05 mm, a track 0.094 mm from a
        // battery holder's F.Mask polygon bridged it to the GND fill. The
        // web rule (half the minimum width, 0.075 mm here) stays a floor; a
        // text has no expansion.
        let lowered = |margin: &str, mask_to_copper: Option<f64>, text: bool| {
            let opening = if text {
                r#"(gr_text "X" (at 5 5 0) (layer "F.Mask") (effects (font (size 1 1) (thickness 0.15))) (render_cache "X" 0 (polygon (pts (xy 4.5 4.5) (xy 5.5 4.5) (xy 5.5 5.5) (xy 4.5 5.5)))))"#.to_string()
            } else {
                format!(r#"(gr_rect (start 4 4) (end 6 6) (stroke (width 0) (type solid)) (fill yes) {margin} (layer "F.Mask"))"#)
            };
            let pcb = parse(&format!(
                r#"(kicad_pcb
                  (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user) (39 "F.Mask" user) (38 "B.Mask" user))
                  (setup (pad_to_mask_clearance 0.05) (solder_mask_min_width 0.13))
                  (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
                  (footprint "a" (at 12 5)
                    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/A"))
                    (pad "2" smd rect (at 4 0) (size 1 1) (layers "F.Cu") (net "/A")))
                  {opening})"#
            ))
            .unwrap();
            let rules = KiCadConnectionRoutingRules { trace_width_mm: 0.2, clearance_mm: 0.2, via_size_mm: 0.6, via_drill_mm: 0.3 };
            let config = KiCadBoardRouterConfig { default_rules: Some(rules), solder_mask_to_copper_mm: mask_to_copper, ..KiCadBoardRouterConfig::default() };
            let board = lower(&pcb, &config, false).unwrap().board;
            board.obstacles.iter().find(|obstacle| obstacle.label.starts_with("solder mask opening")).unwrap().clearance
        };
        let close = |value: f64, expected: f64| assert!((value - expected).abs() < 1.0e-9, "{value} against {expected}");
        close(lowered("", Some(0.05), false), 0.05 + 0.05);
        close(lowered("(solder_mask_margin 0.2)", Some(0.05), false), 0.05 + 0.2);
        close(lowered("", None, false), 0.075);
        close(lowered("", Some(0.05), true), 0.075);
    }

    #[test]
    fn a_knockout_text_is_its_plate() {
        let text = parse(
            r#"(gr_text "0b" (at 0 0 0) (layer "F.Cu" knockout) (effects (font (size 7 7) (thickness 0.6)))
              (render_cache "0b" 0 (polygon (pts (xy 1 1) (xy 3 1) (xy 3 4) (xy 1 4))) (polygon (pts (xy 5 1) (xy 8 1) (xy 8 4) (xy 5 4)))))"#,
        )
        .unwrap();
        let shapes = copper_graphic_shapes(&text).unwrap();
        assert_eq!(shapes.len(), 1);
        let bounds = shapes[0].aabb();
        assert!((bounds.minimum[0] - 0.4).abs() < 1e-9 && (bounds.maximum[0] - 8.6).abs() < 1e-9);
        assert!((bounds.minimum[1] - 0.4).abs() < 1e-9 && (bounds.maximum[1] - 4.6).abs() < 1e-9);
        // Without the knockout, the glyphs alone.
        let plain = parse(
            r#"(gr_text "0b" (at 0 0 0) (layer "F.Cu") (effects (font (size 7 7) (thickness 0.6)))
              (render_cache "0b" 0 (polygon (pts (xy 1 1) (xy 3 1) (xy 3 4) (xy 1 4))) (polygon (pts (xy 5 1) (xy 8 1) (xy 8 4) (xy 5 4)))))"#,
        )
        .unwrap();
        assert_eq!(copper_graphic_shapes(&plain).unwrap().len(), 2);
    }

    #[test]
    fn a_polygon_counts_its_stroke_and_only_a_fill_it_has() {
        let shapes = |text: &str| copper_graphic_shapes(&parse(text).unwrap()).unwrap();
        let reaches = |shapes: &[core::Shape], point: [f64; 2]| shapes.iter().any(|shape| shape.contains(point));
        let stroked = shapes(
            r#"(fp_poly (pts (xy 0 0) (xy 4 0) (xy 4 4) (xy 0 4)) (stroke (width 0.1) (type solid)) (fill yes) (layer "F.Mask"))"#,
        );
        assert!(reaches(&stroked, [2.0, 2.0]));
        assert!(reaches(&stroked, [4.04, 2.0]));
        assert!(!reaches(&stroked, [4.06, 2.0]));
        let outline = shapes(
            r#"(gr_poly (pts (xy 0 0) (xy 4 0) (xy 4 4) (xy 0 4)) (stroke (width 0.2) (type solid)) (fill no) (layer "F.Mask"))"#,
        );
        assert!(!reaches(&outline, [2.0, 2.0]));
        assert!(reaches(&outline, [3.95, 2.0]));
    }

    #[test]
    fn mask_openings_belong_to_the_net_they_expose() {
        let lowered = |zone: &str| {
            let pcb = parse(&format!(
                r#"(kicad_pcb
              (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user) (39 "F.Mask" user) (38 "B.Mask" user))
              (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
              (footprint "a" (at 5 5)
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/A"))
                (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "/B")))
              (gr_rect (start 4.6 4.6) (end 5.4 5.4) (stroke (width 0) (type solid)) (fill yes) (layer "F.Mask"))
              (gr_rect (start 4.6 4.6) (end 7.4 5.4) (stroke (width 0) (type solid)) (fill yes) (layer "F.Mask"))
              (gr_text "LABEL" (at 14 5) (layer "F.Mask") (effects (font (size 1 1) (thickness 0.15)))) {zone})"#
            ))
            .unwrap();
            let rules = KiCadConnectionRoutingRules { trace_width_mm: 0.2, clearance_mm: 0.2, via_size_mm: 0.6, via_drill_mm: 0.3 };
            let config = KiCadBoardRouterConfig { default_rules: Some(rules), ..KiCadBoardRouterConfig::default() };
            lower(&pcb, &config, false).unwrap().board
        };
        let openings = |board: &core::Board| -> Vec<(Option<core::NetId>, String)> {
            board
                .obstacles
                .iter()
                .filter(|obstacle| obstacle.label.starts_with("solder mask opening"))
                .map(|obstacle| (obstacle.net, obstacle.label.clone()))
                .collect()
        };
        let board = lowered("");
        let net_b = board.nets.iter().position(|net| net.name.ends_with('B')).map(|index| index as core::NetId);
        let net_a = board.nets.iter().position(|net| net.name.ends_with('A')).map(|index| index as core::NetId);
        // The first rectangle (its fill; no stroke) is pad 1's; the
        // second spans two pads and keeps everything out; so does text
        // over bare board (two nets under it would bridge).
        let bare = openings(&board);
        assert_eq!(bare.len(), 3);
        assert!(bare[..1].iter().all(|(net, _)| net.is_some() && *net == net_a));
        assert!(bare[1..2].iter().all(|(net, label)| net.is_none() && label.ends_with("(gr_rect)")));
        assert_eq!(bare[2], (None, "solder mask opening (gr_text)".to_string()));
        // Over a pour, the text exposes the fill: it is the pour's net's.
        let zone = r#"(zone (net "/B") (layer "F.Cu") (polygon (pts (xy 10 0) (xy 20 0) (xy 20 10) (xy 10 10))))"#;
        let poured = openings(&lowered(zone));
        assert_eq!(poured.len(), 3);
        assert_eq!(poured[2], (net_b, "solder mask opening (gr_text)".to_string()));
    }

    #[test]
    fn a_mask_only_pad_opens_the_mask_by_its_expansion() {
        // hackrf's mask-only test point: a 0.762 mm pad on F.Mask alone,
        // the board's mask expansion 0.05 mm.
        let lowered = |zone: &str, config: KiCadBoardRouterConfig| {
            let pcb = parse(&format!(
                r#"(kicad_pcb
              (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user) (39 "F.Mask" user) (38 "B.Mask" user))
              (setup (pad_to_mask_clearance 0.05))
              (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
              (footprint "TP" (layer "F.Cu") (at 5 5) (property "Reference" "TP1")
                (pad "" smd circle (at 0 0) (size 0.762 0.762) (layers "F.Mask")))
              (footprint "a" (at 15 5)
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/A"))
                (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "/B"))) {zone})"#
            ))
            .unwrap();
            let rules = KiCadConnectionRoutingRules { trace_width_mm: 0.2, clearance_mm: 0.2, via_size_mm: 0.6, via_drill_mm: 0.3 };
            let config = KiCadBoardRouterConfig { default_rules: Some(rules), ..config };
            lower(&pcb, &config, false).unwrap().board
        };
        let openings = |board: &core::Board| -> Vec<(Option<core::NetId>, f64, u32, String)> {
            board
                .obstacles
                .iter()
                .filter(|obstacle| obstacle.label.starts_with("solder mask opening"))
                .map(|obstacle| (obstacle.net, obstacle.clearance, obstacle.layers, obstacle.label.clone()))
                .collect()
        };
        // Over bare board it is no net's: nothing goes under it, by the
        // expansion, and on the front only.
        let bare = openings(&lowered("", KiCadBoardRouterConfig::default()));
        assert_eq!(bare.len(), 1);
        assert_eq!(bare[0].0, None);
        assert!((bare[0].1 - 0.05).abs() < 1.0e-9, "{}", bare[0].1);
        assert_eq!(bare[0].2, 1);
        assert_eq!(bare[0].3, "solder mask opening (pad TP1.)");
        // Over a pour it is the pour's net's.
        let zone = r#"(zone (net "/B") (layer "F.Cu") (polygon (pts (xy 0 0) (xy 10 0) (xy 10 10) (xy 0 10))))"#;
        let board = lowered(zone, KiCadBoardRouterConfig::default());
        let net_b = board.nets.iter().position(|net| net.name.ends_with('B')).map(|index| index as core::NetId);
        assert_eq!(openings(&board)[0].0, net_b);
        // Off: the pad is nothing to the router.
        let off = KiCadBoardRouterConfig { mask_only_pad_openings: Some(false), ..KiCadBoardRouterConfig::default() };
        assert!(openings(&lowered("", off)).is_empty());
    }

    #[test]
    fn graphics_on_several_layers_carry_their_net() {
        let pcb = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user) (39 "F.Mask" user))
          (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
          (gr_rect (start 2 2) (end 4 4) (stroke (width 0) (type default)) (fill yes) (layers "F.Cu" "F.Mask") (net "GND"))
          (footprint "a" (at 10 5)
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND"))
            (pad "2" smd rect (at 3 0) (size 1 1) (layers "F.Cu") (net "GND"))))"#,
        )
        .unwrap();
        let rules = KiCadConnectionRoutingRules { trace_width_mm: 0.2, clearance_mm: 0.2, via_size_mm: 0.6, via_drill_mm: 0.3 };
        let config = KiCadBoardRouterConfig { default_rules: Some(rules), ..KiCadBoardRouterConfig::default() };
        let board = lower(&pcb, &config, false).unwrap().board;
        let ground = board.nets.iter().position(|net| net.name == "GND").map(|index| index as core::NetId);
        assert!(ground.is_some());
        let copper: Vec<_> = board.obstacles.iter().filter(|obstacle| obstacle.label == "copper gr_rect").collect();
        assert_eq!(copper.len(), 1);
        assert_eq!((copper[0].net, copper[0].layers), (ground, 1));
        let opening: Vec<_> = board.obstacles.iter().filter(|obstacle| obstacle.label.starts_with("solder mask opening")).collect();
        assert_eq!(opening.len(), 1);
        assert_eq!(opening[0].net, ground);
    }

    #[test]
    fn overlapping_pads_of_one_number_are_one_terminal() {
        let pcb = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
          (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
          (footprint "a" (at 5 5) (property "Reference" "TP1")
            (pad "1" thru_hole roundrect (at 0 0) (size 1.9 4.5) (drill 1) (layers "*.Cu") (roundrect_rratio 0.25) (net "/W"))
            (pad "1" thru_hole circle (at 0.5 1.2) (size 0.6 0.6) (drill 0.4) (layers "*.Cu") (net "/W"))
            (pad "1" thru_hole circle (at -0.5 -1.2) (size 0.6 0.6) (drill 0.4) (layers "*.Cu") (net "/W")))
          (footprint "b" (at 15 5) (property "Reference" "TP2")
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/W"))))"#,
        )
        .unwrap();
        let rules = KiCadConnectionRoutingRules { trace_width_mm: 0.2, clearance_mm: 0.2, via_size_mm: 0.6, via_drill_mm: 0.3 };
        let config = KiCadBoardRouterConfig { default_rules: Some(rules), ..KiCadBoardRouterConfig::default() };
        let board = lower(&pcb, &config, false).unwrap().board;
        let net = board.nets.iter().find(|net| net.name.ends_with('W')).unwrap();
        assert_eq!(net.terminals.len(), 2);
        // The large pad stays, and the terminal points at its copper.
        assert_eq!(net.terminals[0].anchor, [5.0, 5.0]);
        assert_eq!(board.obstacles[net.terminals[0].pad].kind, core::ObstacleKind::Copper);
    }

    #[test]
    fn an_unfilled_circle_in_a_custom_pad_is_a_ring() {
        let pcb = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
          (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
          (footprint "dome" (at 10 5) (property "Reference" "SW1")
            (pad "1" smd circle (at 0 0) (size 4 4) (layers "F.Cu") (net "/A"))
            (pad "2" smd custom (at 2.75 0) (size 0.7 0.7) (layers "F.Cu") (net "/B")
              (options (clearance outline) (anchor circle))
              (primitives (gr_circle (center -2.75 0) (end 0 0) (width 0.7) (fill no))))))"#,
        )
        .unwrap();
        let rules = KiCadConnectionRoutingRules { trace_width_mm: 0.2, clearance_mm: 0.2, via_size_mm: 0.6, via_drill_mm: 0.3 };
        let config = KiCadBoardRouterConfig { default_rules: Some(rules), ..KiCadBoardRouterConfig::default() };
        let board = lower(&pcb, &config, false).unwrap().board;
        let ring = board.obstacles.iter().find(|obstacle| obstacle.label == "SW1.2").unwrap();
        // The inner pad's centre is free of the ring; the ring itself is copper.
        assert!(ring.shape.distance_to_point([10.0, 5.0]) > 2.0);
        assert!(ring.shape.distance_to_point([12.75, 5.0]) < 1.0e-6);
    }

    #[test]
    fn other_nets_vias_keep_the_hole_clearance_from_a_plated_hole() {
        // jetson-nano-baseboard's J2 pad 1: a 0.55 drill in 0.75 copper,
        // clearance 0.125 and hole clearance 0.25. A 0.5 via of another
        // net 0.769 from the pad's centre keeps 0.144 from its copper but
        // only 0.244 from its hole (KiCad: hole_clearance, 0.2438). The
        // pad's own net is exempt.
        let pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 10 10) (layer "Edge.Cuts"))
              (footprint "dp" (at 5 5) (property "Reference" "J2")
                (pad "1" thru_hole circle (at 0 0) (size 0.75 0.75) (drill 0.55) (layers "*.Cu" "*.Mask") (net "GND"))
                (pad "2" thru_hole circle (at 0 -3) (size 0.75 0.75) (drill 0.55) (layers "*.Cu" "*.Mask") (net "3V3_SYS")))
              (footprint "c" (at 8 8) (property "Reference" "C1")
                (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net "GND"))
                (pad "2" smd rect (at 1 0) (size 0.5 0.5) (layers "F.Cu") (net "3V3_SYS"))))"#,
        )
        .unwrap();
        let rules = KiCadConnectionRoutingRules { trace_width_mm: 0.1, clearance_mm: 0.125, via_size_mm: 0.5, via_drill_mm: 0.15 };
        let config = KiCadBoardRouterConfig {
            default_rules: Some(rules),
            hole_clearance_mm: 0.25,
            hole_to_hole_clearance_mm: 0.25,
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, false).unwrap().board;
        let net = |name: &str| board.nets.iter().position(|net| net.name == name).unwrap() as core::NetId;
        let (ground, supply) = (net("GND"), net("3V3_SYS"));
        let hole = board.obstacles.iter().find(|obstacle| obstacle.label == "J2.1 plated hole").unwrap();
        assert!(hole.blocks_vias && hole.net == Some(ground));
        // The via's place on a lattice node: J2.1 at (5, 5), the via at
        // (-0.741, +0.205) from it as on jetson.
        let at = [5.0 - 0.741, 5.0 + 0.205];
        let pitch = 0.1;
        let grid = core::grid::Grid { origin: [at[0] - 42.0 * pitch, at[1] - 52.0 * pitch], pitch, nx: 99, ny: 99 };
        let node = grid.nearest_node(at).unwrap();
        assert!(core::geometry::distance(grid.center_of(node), at) < 1.0e-9);
        let class = board.nets[supply as usize].class;
        let maps = core::grid::StaticMaps::build(&board, &grid, class, true);
        assert!(!maps.via_allowed(node, supply), "another net's via within the hole clearance");
        assert!(maps.via_allowed(node, ground), "the pad's own net keeps the copper rule");
        // One node further out (0.866 from the centre, 0.775 needed) is free.
        assert!(maps.via_allowed(node - 1, supply));
        // The exact verifier reports the via against the hole.
        let via_at = |net: core::NetId| {
            let mut routes = vec![core::NetRoute::default(); board.nets.len()];
            routes[net as usize].vias.push(core::board::Via { at, diameter: 0.5, drill: 0.15 });
            core::verify(&board, &routes)
        };
        let found = via_at(supply);
        let hole_finding = found.iter().find(|violation| violation.other == "J2.1 plated hole").expect("a hole clearance violation");
        assert!((hole_finding.required - 0.25).abs() < 1.0e-9);
        assert!((hole_finding.actual - 0.2439).abs() < 1.0e-3, "{}", hole_finding.actual);
        assert!(!via_at(ground).iter().any(|violation| violation.other.starts_with("J2.1")));
    }

    #[test]
    fn starved_thermals_come_with_advice() {
        let directory = std::env::temp_dir().join(format!("pcb-maker-starved-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("drc.json"),
            r#"{"violations": [
              {"type": "starved_thermal", "description": "Thermal relief connection to zone incomplete (layer F.Cu; zone min spoke count 2; actual 1)",
               "items": [{"description": "Zone [GND] on F.Cu", "pos": {"x": 1, "y": 2}}, {"description": "Pad 4 [GND] of U201 on F.Cu", "pos": {"x": 3.5, "y": 4}}]},
              {"type": "starved_thermal", "description": "Thermal relief connection to zone incomplete (layer F.Cu; 4 spokes connected to isolated island)",
               "items": [{"description": "PTH pad 4 [MCU_GND] of J102", "pos": {"x": 5, "y": 6}}]},
              {"type": "clearance", "description": "x", "items": []}]}"#,
        )
        .unwrap();
        let found = starved_thermal_diagnostics(&directory);
        fs::remove_dir_all(&directory).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].layer.as_str(), found[0].spokes, found[0].needed, found[0].at), ("F.Cu", Some(1), Some(2), [3.5, 4.0]));
        assert!(found[0].advice.contains("only 1 of 2 spokes"));
        assert!(found[1].advice.contains("not joined"));
        assert!(found[1].pad.contains("J102"));
    }

    #[test]
    fn requested_pours_cover_the_outline_once_per_net_and_layer() {
        let mut pcb = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
          (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
          (footprint "a" (at 5 5) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/GND")))
          (zone (net "/GND") (layer "B.Cu") (polygon (pts (xy 0 0) (xy 20 0) (xy 20 10)))))"#,
        )
        .unwrap();
        let request = |layers: &[&str]| KiCadPourRequest {
            net: "GND".into(),
            layers: layers.iter().map(|layer| layer.to_string()).collect(),
            clearance_mm: None,
        };
        add_pour_zones(&mut pcb, &[request(&["F.Cu", "B.Cu"])]).unwrap();
        let zones: Vec<_> = pcb.children().iter().filter(|item| item.head() == Some("zone")).collect();
        // B.Cu had one already; F.Cu got a new one over the outline.
        assert_eq!(zones.len(), 2);
        assert_eq!(form_atom(zones[1], "layer", 1), Some("F.Cu"));
        assert_eq!(node_net(zones[1]), Some("/GND"));
        let unknown = KiCadPourRequest { net: "VCC".into(), ..request(&["F.Cu"]) };
        assert!(add_pour_zones(&mut pcb, &[unknown]).is_err());
        assert!(add_pour_zones(&mut pcb, &[request(&["In1.Cu"])]).is_err());
    }
}

#[cfg(test)]
mod narrow_step_tests {
    use super::*;

    fn rules(width: f64, clearance: f64) -> KiCadConnectionRoutingRules {
        KiCadConnectionRoutingRules { trace_width_mm: width, clearance_mm: clearance, via_size_mm: 0.6, via_drill_mm: 0.3 }
    }

    #[test]
    fn the_step_applies_only_with_a_narrower_width_or_a_relaxed_clearance() {
        let plain = KiCadBoardRouterConfig { default_rules: Some(rules(0.2, 0.2)), ..KiCadBoardRouterConfig::default() };
        assert!(!narrow_step_applies(&plain));
        // A menu width no narrower than every class: nothing to narrow.
        let same = KiCadBoardRouterConfig { narrow_signal_mm: Some(0.2), ..plain.clone() };
        assert!(!narrow_step_applies(&same));
        let narrower = KiCadBoardRouterConfig { narrow_signal_mm: Some(0.15), ..plain.clone() };
        assert!(narrow_step_applies(&narrower));
        // Already narrow (the step itself): it does not apply again.
        let narrowed = KiCadBoardRouterConfig { use_narrow_signals: Some(true), ..narrower.clone() };
        assert!(!narrow_step_applies(&narrowed));
        let relaxed = KiCadBoardRouterConfig { relaxed_clearance_mm: Some(0.1), ..plain.clone() };
        assert!(narrow_step_applies(&relaxed));
        let not_smaller = KiCadBoardRouterConfig { relaxed_clearance_mm: Some(0.2), ..plain.clone() };
        assert!(!narrow_step_applies(&not_smaller));
        // A wide class among narrow ones is enough.
        let mut classes = plain.clone();
        classes.default_rules = Some(rules(0.125, 0.2));
        classes.connection_rules.insert("/PWR".into(), rules(0.5, 0.2));
        classes.narrow_signal_mm = Some(0.125);
        assert!(narrow_step_applies(&classes));
    }

    #[test]
    fn the_reserve_is_a_share_of_the_first_attempt_within_its_floor_and_cap() {
        assert_eq!(narrow_step_reserve(400.0, 0.75, 600.0), 300.0);
        // A cheap first attempt still keeps the ladder's floor of 120 s.
        assert_eq!(narrow_step_reserve(50.0, 0.75, 600.0), 120.0);
        assert_eq!(narrow_step_reserve(1000.0, 0.75, 600.0), 600.0);
        // The cap wins over the floor (little work left at all).
        assert_eq!(narrow_step_reserve(50.0, 0.75, 80.0), 80.0);
        assert_eq!(narrow_step_reserve(400.0, 0.0, 600.0), 0.0);
    }

    #[test]
    fn the_step_needs_half_its_reserve_and_three_floors_of_negotiation() {
        // The A13 module: 278 s kept, about 600 s left: it runs.
        assert_eq!(narrow_step_minimum(278.0), 180.0);
        assert_eq!(narrow_step_minimum(600.0), 300.0);
        // Jetson: 129 s kept, 70 s left: it does not.
        assert!(70.0 < narrow_step_minimum(129.0));
    }

    #[test]
    fn the_step_negotiates_a_third_of_what_is_left() {
        assert_eq!(narrow_step_negotiation(450.0, None), 150.0);
        assert_eq!(narrow_step_negotiation(90.0, None), 60.0);
        assert_eq!(narrow_step_negotiation(6000.0, None), 900.0);
        // Never more than the attempt it varies had.
        assert_eq!(narrow_step_negotiation(450.0, Some(100.0)), 100.0);
        assert_eq!(narrow_step_negotiation(450.0, Some(200.0)), 150.0);
    }

    #[test]
    fn the_step_negotiates_at_least_what_the_first_attempt_did_within_the_reserve() {
        // A finer rung with 60 s of negotiation is the best attempt; the
        // first attempt negotiated 120 s, 300 s kept and 330 s left.
        assert_eq!(narrow_step_negotiation(330.0, Some(60.0)), 60.0);
        assert_eq!(narrow_step_negotiation_floored(330.0, Some(60.0), 120.0, 300.0), 120.0);
        // Bounded by the reserve and by half of what is left.
        assert_eq!(narrow_step_negotiation_floored(1000.0, Some(60.0), 400.0, 250.0), 250.0);
        assert_eq!(narrow_step_negotiation_floored(300.0, Some(60.0), 400.0, 600.0), 150.0);
        // Never less than before.
        assert_eq!(narrow_step_negotiation_floored(900.0, None, 100.0, 300.0), 300.0);
        assert_eq!(narrow_step_negotiation_floored(900.0, None, 0.0, 300.0), 300.0);
    }

    #[test]
    fn the_floored_reserve_is_enough_for_the_step_to_run() {
        // TelemetryOnboard: a short first attempt kept 120 s, below the
        // step's 180 s minimum; as a floor it keeps 240 s.
        assert_eq!(narrow_step_reserve(100.0, 0.75, 600.0), 120.0);
        assert!(narrow_step_reserve(100.0, 0.75, 600.0) < narrow_step_minimum(120.0));
        let kept = narrow_step_reserve_floored(100.0, 0.75, 600.0);
        assert_eq!(kept, NARROW_RESERVE_FLOOR);
        assert!(kept >= narrow_step_minimum(kept));
        // Share and cap as before.
        assert_eq!(narrow_step_reserve_floored(400.0, 0.75, 600.0), 300.0);
        assert_eq!(narrow_step_reserve_floored(1000.0, 0.75, 600.0), 600.0);
        assert_eq!(narrow_step_reserve_floored(400.0, 0.0, 600.0), 0.0);
        // A cap under the floor still keeps what the step can use...
        assert_eq!(narrow_step_reserve_floored(100.0, 0.75, 200.0), 200.0);
        // ...and nothing when the step could never run on it.
        assert_eq!(narrow_step_reserve_floored(100.0, 0.75, 150.0), 0.0);
        // Whatever the first attempt, a kept reserve lets the step run.
        for work in [10.0, 100.0, 320.0, 500.0, 2000.0] {
            for cap in [100.0, 179.0, 180.0, 240.0, 400.0, 600.0] {
                let kept = narrow_step_reserve_floored(work, 0.75, cap);
                assert!(kept == 0.0 || kept >= narrow_step_minimum(kept), "{work} {cap}");
            }
        }
    }

    #[test]
    fn no_attempt_starts_into_the_reserve() {
        // DSP-ADAU1452: 300 s kept, 450 s left, the last 0.075 mm rung took
        // 100 s: another rung at that pitch fits, one at 0.05 mm (225 s
        // projected) does not.
        let next = projected_attempt_work(100.0, 0.075, 0.075);
        assert_eq!(next, 100.0);
        assert!(fits_before_reserve(450.0, 300.0, next));
        let finer = projected_attempt_work(100.0, 0.075, 0.05);
        assert!((finer - 225.0).abs() < 1.0e-9);
        assert!(!fits_before_reserve(450.0, 300.0, finer));
        // A coarser or equal pitch is not scaled down.
        assert_eq!(projected_attempt_work(100.0, 0.075, 0.1), 100.0);
        // An attempt counts at least 120 s.
        assert!(!fits_before_reserve(400.0, 300.0, 30.0));
        assert!(fits_before_reserve(420.0, 300.0, 30.0));
        // The whole reserve, not half of it: 150 s left beyond half of a
        // 300 s reserve is not enough for a 130 s rung.
        assert!(!fits_before_reserve(300.0, 300.0, 130.0));
    }

    #[test]
    fn attempts_stop_where_the_reserve_begins() {
        assert_eq!(deadline_before_reserve(700.0, 300.0, 1), 400.0);
        // Four seeds side by side: their work counts four times.
        assert_eq!(deadline_before_reserve(700.0, 300.0, 4), 100.0);
        assert_eq!(deadline_before_reserve(200.0, 300.0, 1), 0.0);
        assert_eq!(deadline_before_reserve(700.0, 300.0, 0), 400.0);
    }

    #[test]
    fn the_minimum_connection_floors_classes_the_narrow_width_and_the_neck() {
        let mut config = KiCadBoardRouterConfig { default_rules: Some(rules(0.09, 0.09)), ..KiCadBoardRouterConfig::default() };
        config.connection_rules.insert("/PWR".into(), rules(0.5, 0.2));
        config.narrow_signal_mm = Some(0.11);
        config.neck_width_mm = Some(0.1);
        assert!(narrow_step_applies(&config));
        config.min_connection_mm = Some(0.16);
        config.apply_width_floor();
        assert_eq!(config.default_rules.as_ref().unwrap().trace_width_mm, 0.16);
        assert_eq!(config.connection_rules["/PWR"].trace_width_mm, 0.5);
        assert_eq!(config.narrow_signal_mm, Some(0.16));
        assert_eq!(config.neck_width_mm, Some(0.16));
        // The power class is still wider: the narrow step routes the signals
        // at the floor.
        assert!(narrow_step_applies(&config));
        // No floor, nothing changes.
        let mut plain = KiCadBoardRouterConfig { default_rules: Some(rules(0.09, 0.09)), ..KiCadBoardRouterConfig::default() };
        plain.apply_width_floor();
        assert_eq!(plain.default_rules.as_ref().unwrap().trace_width_mm, 0.09);
    }

    #[test]
    fn a_routed_board_has_no_track_narrower_than_the_minimum_connection() {
        // Two 0.25 mm pads side by side force a neck-down; class, narrow
        // signal and neck widths are all below the floor.
        let pcb = parse(
            r#"(kicad_pcb
              (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 12 8) (layer "Edge.Cuts"))
              (footprint "a" (at 3 4)
                (pad "1" smd rect (at 0 0) (size 0.25 0.8) (layers "F.Cu") (net "/A"))
                (pad "2" smd rect (at 0.5 0) (size 0.25 0.8) (layers "F.Cu") (net "/B")))
              (footprint "b" (at 9 4)
                (pad "1" smd rect (at 0 0) (size 0.8 0.8) (layers "F.Cu") (net "/A"))
                (pad "2" smd rect (at 0 2) (size 0.8 0.8) (layers "F.Cu") (net "/B"))))"#,
        )
        .unwrap();
        let mut config = KiCadBoardRouterConfig { default_rules: Some(rules(0.3, 0.1)), ..KiCadBoardRouterConfig::default() };
        config.narrow_signal_mm = Some(0.1);
        config.use_narrow_signals = Some(true);
        config.neck_width_mm = Some(0.1);
        config.min_connection_mm = Some(0.15);
        let board = lower(&pcb, &config, false).unwrap().board;
        assert_eq!(board.neck_width, 0.15);
        for net in &board.nets {
            assert_eq!(board.classes[net.class].trace_width, 0.15, "{}", net.name);
        }
        let result = core::route(&board, &core_config(&config));
        let widths: Vec<f64> = result.routes.iter().flat_map(|route| route.segments.iter().map(|segment| segment.width)).collect();
        assert!(!widths.is_empty());
        assert!(widths.iter().all(|width| *width >= 0.15 - 1.0e-12), "{widths:?}");
        // Without the floor the same board routes at the narrow width.
        config.min_connection_mm = None;
        let board = lower(&pcb, &config, false).unwrap().board;
        assert!(board.nets.iter().all(|net| board.classes[net.class].trace_width == 0.1));
    }

    #[test]
    fn the_step_config_narrows_signals_and_relaxes_clearance_once() {
        let mut config = KiCadBoardRouterConfig { default_rules: Some(rules(0.2, 0.2)), ..KiCadBoardRouterConfig::default() };
        config.connection_rules.insert("/PWR".into(), rules(0.5, 0.25));
        config.narrow_signal_mm = Some(0.125);
        config.relaxed_clearance_mm = Some(0.1);
        let step = narrow_step_config(&config);
        assert_eq!(step.use_narrow_signals, Some(true));
        assert_eq!(step.relaxed_clearance_mm, None);
        assert_eq!(step.default_rules.as_ref().unwrap().clearance_mm, 0.1);
        assert_eq!(step.connection_rules["/PWR"].clearance_mm, 0.1);
        // The widths stay the classes' (lowering narrows the signals).
        assert_eq!(step.connection_rules["/PWR"].trace_width_mm, 0.5);
        assert!(!narrow_step_applies(&step));
        // Only a relaxed clearance: no narrow width to use.
        let relaxed = KiCadBoardRouterConfig { narrow_signal_mm: None, ..config.clone() };
        let step = narrow_step_config(&relaxed);
        assert_eq!(step.use_narrow_signals, Some(false));
        assert_eq!(step.default_rules.as_ref().unwrap().clearance_mm, 0.1);
        assert!(!narrow_step_applies(&step));
    }
}
