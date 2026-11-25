// Copyright (C) 2026 Toit contributors.

//! What a routed board reports: per net, violations, rules and diagnostics.

use super::*;
use pcb_router as core;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadBoardRouterNet {
    pub connection: String,
    pub terminals: usize,
    pub status: String,
    pub unconnected_terminals: usize,
    pub vias: usize,
    pub length_mm: f64,
    /// A `partial` or `unreachable` net's open terminals by why the router
    /// left them open (`pcb_router::OpenReason`), as many as
    /// `unconnected_terminals` or more (an unreachable net lists all its
    /// terminals); where the router recorded fewer, the rest under
    /// `unknown`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub open: Vec<KiCadOpenTerminals>,
    /// How the hard repair found an open net it took up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repair: Option<KiCadNetRepair>,
    /// What route mode's open repair after the ladder did to the net
    /// (`route_open_repair.rs`): the net's other fields then describe the
    /// board written, this one what the router had left.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_repair: Option<super::route_open_repair::KiCadNetOpenRepair>,
    /// Where the router keeps several nets of this name (a panel's
    /// sub-boards or cells: `outline_pieces::split_router_nets`): which of
    /// them this is, counted in `nets` order from 0, and all its
    /// terminals' labels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copy: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pads: Vec<String>,
}

/// At most this many labels in a `KiCadOpenTerminals::pads`.
pub const OPEN_PADS_LISTED: usize = 16;

/// A net's open terminals for one reason (`KiCadBoardRouterNet::open`).
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct KiCadOpenTerminals {
    /// `pcb_router::OpenReason::name`: `no_access`, `net_not_routable`,
    /// `no_path_static`, `no_lattice_exit`, `repair_skipped:<why>`,
    /// `no_path_hard:<scope>`, `lost:<phase>`,
    /// `pour_apart_before_stitching`, `copper_check`, `unknown`.
    pub reason: String,
    /// How many (exact).
    pub terminals: usize,
    /// Their labels (`U1.3`), the first `OPEN_PADS_LISTED`.
    pub pads: Vec<String>,
    /// `no_path_static`, `no_lattice_exit`: the pads the wall is at
    /// (`pcb_router::OpenTerminal::walled`: the net's pads whose pocket is
    /// closed on the lattice, which may be the pads the search started
    /// from and none of `pads`), the first `OPEN_PADS_LISTED`. Absent
    /// where no pad's pocket is closed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub walled: Vec<String>,
    /// `no_path_static` with `walled`: whose copper bounds the region the
    /// rules leave there: `input` (the board's pads, copper, rule areas,
    /// holes and edge), `router` (vias the router planned before it
    /// routed: a BGA's fan-out, vias in pads) or `input_and_router`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wall: Option<&'static str>,
    /// What bounds it, by label, the first `WALL_LISTED`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<String>,
}

/// At most this many labels in a `KiCadOpenTerminals::blocked_by`.
pub const WALL_LISTED: usize = 8;

/// Whether an obstacle of this label is copper the router planned before
/// it routed, not the input's: a BGA fan-out's stub, via and hole
/// (`lower/fanout.rs`) or a via in a pad and its hole (`lower/pad.rs`).
pub(crate) fn router_planned(label: &str) -> bool {
    [
        " fan-out stub",
        " fan-out via ring",
        " fan-out via",
        " fan-out hole",
        " via in pad ring",
        " via in pad",
        " via in pad hole",
    ]
    .iter()
    .any(|suffix| label.ends_with(suffix))
}

/// The hard repair's account of a net (`pcb_router::RepairEntry`).
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct KiCadNetRepair {
    /// When the negotiation ended: whether it had joined all its
    /// terminals, and whether it was in conflict with other nets' copper.
    pub complete: bool,
    pub in_conflict: bool,
    /// Why the repair did not search it: `work_deadline`, `hopeless`,
    /// `budget`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub skipped: String,
    /// The nets whose copper covered its first conflicting nodes, and
    /// where the first one was (its layer, `via` for the via map).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflicted_with: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_at: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_layer: Option<String>,
}

/// `net`'s open terminals of `diagnostics` by reason, in the order the
/// reasons first come, `unconnected` of them at least (the rest
/// `unknown`).
pub(crate) fn open_terminals_of(
    diagnostics: &core::Diagnostics,
    net: core::NetId,
    unconnected: usize,
) -> Vec<KiCadOpenTerminals> {
    let mut groups: Vec<KiCadOpenTerminals> = Vec::new();
    let mut listed = 0;
    for open in diagnostics.open.iter().filter(|open| open.net == net) {
        let reason = open.reason.name();
        let index = match groups.iter().position(|group| group.reason == reason) {
            Some(index) => index,
            None => {
                groups.push(KiCadOpenTerminals {
                    reason,
                    terminals: 0,
                    pads: Vec::new(),
                    walled: Vec::new(),
                    wall: None,
                    blocked_by: Vec::new(),
                });
                groups.len() - 1
            }
        };
        let group = &mut groups[index];
        group.terminals += 1;
        if group.pads.len() < OPEN_PADS_LISTED {
            group.pads.push(open.label.clone());
        }
        for label in &open.walled {
            if !group.walled.contains(label) && group.walled.len() < OPEN_PADS_LISTED {
                group.walled.push(label.clone());
            }
        }
        for label in &open.wall {
            group.wall = Some(match (group.wall, router_planned(label)) {
                (None | Some("router"), true) => "router",
                (None | Some("input"), false) => "input",
                _ => "input_and_router",
            });
            if !group.blocked_by.contains(label) && group.blocked_by.len() < WALL_LISTED {
                group.blocked_by.push(label.clone());
            }
        }
        listed += 1;
    }
    if listed < unconnected {
        match groups.iter_mut().find(|group| group.reason == "unknown") {
            Some(group) => group.terminals += unconnected - listed,
            None => groups.push(KiCadOpenTerminals {
                reason: "unknown".into(),
                terminals: unconnected - listed,
                pads: Vec::new(),
                walled: Vec::new(),
                wall: None,
                blocked_by: Vec::new(),
            }),
        }
    }
    groups
}

/// The hard repair's entry for `net` in `diagnostics`, its nets named (by
/// `nets`, the board's) and its place in mm (rounded to µm) on KiCad's
/// layer (`layer_names`; the board's `layer_count` is the via map).
pub(crate) fn net_repair_of(
    diagnostics: &core::Diagnostics,
    nets: &[core::Net],
    layer_count: usize,
    net: core::NetId,
    layer_names: &[String],
) -> Option<KiCadNetRepair> {
    let entry = diagnostics.repair.iter().find(|entry| entry.net == net)?;
    let round = |value: f64| (value * 1000.0).round() / 1000.0;
    Some(KiCadNetRepair {
        complete: entry.complete,
        in_conflict: entry.conflicted,
        skipped: entry.skipped.to_string(),
        conflicted_with: entry
            .conflicted_with
            .iter()
            .filter_map(|other| nets.get(*other as usize))
            .map(|other| other.name.clone())
            .collect(),
        conflict_at: entry
            .conflict_at
            .map(|(_, at)| [round(at[0]), round(at[1])]),
        conflict_layer: entry.conflict_at.map(|(layer, _)| {
            layer_names
                .get(layer)
                .filter(|_| layer < layer_count)
                .cloned()
                .unwrap_or_else(|| "via".into())
        }),
    })
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

#[derive(Clone, Debug, Default, Serialize)]
pub struct KiCadBoardRouterResult {
    pub board_id: String,
    /// Exact prior copper considered in route mode and its native admission.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_reuse: Option<super::KiCadRouteReuse>,
    /// In-process proof for sharing the selected closed trial's strict DRC.
    #[doc(hidden)]
    #[serde(skip)]
    pub closed_reuse_drc: Option<verification::ClosedReuseDrc>,
    /// The ordinary final native check's exact in-process fill receipt.
    #[doc(hidden)]
    #[serde(skip)]
    pub checked_layout_refill: Option<verification::CheckedRefill>,
    /// Short pad leaves emitted by this route, for the native-gated repair.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub generated_pad_leaf_ids: Vec<String>,
    pub routable_connections: usize,
    pub routed_connections: usize,
    pub unconnected_terminals: usize,
    /// Legacy total of `nets[].vias` from route emission.
    pub vias: usize,
    /// Legacy total of `nets[].length_mm` from route emission.
    pub length_mm: f64,
    /// All track and via objects on the saved PCB, including retained copper,
    /// pair routes and repairs that ran after the ordinary router.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_copper: Option<pcb_kicad_file::board_stats::KiCadSavedCopper>,
    /// Independent KiCad evidence for preserving an input route or plane.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_connectivity: Option<super::KiCadInputConnectivity>,
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
    /// Scoped severity-error DRC count used while ranking checked rungs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_candidate_errors: Option<usize>,
    /// Native verification and `saved_copper` include a later repair. Core
    /// search metrics precede it, and so do the per-net route totals and
    /// the counts summed from them, but after route mode's open repair
    /// (`open_repair`: its nets carry `nets[].open_repair` and the totals
    /// count its copper).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub model_precedes_additive_repair: bool,
    /// Route mode's open repair after the ladder (`route_open_repair.rs`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_repair: Option<KiCadRouteOpenRepair>,
    /// The saved copper and native result include a verified pad-via move.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub model_precedes_generated_pad_via_repair: bool,
    /// A verified postroute pad relocation retained the saved tracks and vias.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub model_precedes_body_near_grid_retry: bool,
    /// `connect`, `tracks`, or `none` when the board has no pours.
    pub pours: String,
    pub nets: Vec<KiCadBoardRouterNet>,
    /// Congestion history on the routing lattice (row major), for placement
    /// feedback. Not serialized.
    #[serde(skip)]
    pub congestion: Vec<f32>,
    #[serde(skip)]
    pub grid_origin: [f64; 2],
    /// Route mode: the board's own tracks and vias, kept, and the
    /// connections they made (`existing_tracks`); absent on a board
    /// without them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept_copper: Option<super::KiCadKeptCopper>,
    /// What the via-in-pad rule did (`via_in_pad`): the vias it put in
    /// pads, by footprint with the rule that admits them, and the
    /// footprints whose enclosed pads it does not admit; absent on a board
    /// with neither.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vias_in_pads: Option<super::KiCadViaInPadReport>,
    /// Route mode with `diff_pairs`: the differential pairs found and what
    /// became of them; absent with the switch off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_pairs: Option<super::KiCadDiffPairs>,
    /// The length and skew rules' nets after the route (`length_matching`,
    /// docs/router.md "Length and skew rules"): per rule and net its
    /// length, target, bounds and meander; absent without such rules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lengths: Vec<super::KiCadNetLength>,
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
    /// The part of `work_seconds` that is KiCad's checks (their modelled
    /// work, `check_work`), and how many checks it counts.
    #[serde(default)]
    pub kicad_check_work_seconds: f64,
    #[serde(default)]
    pub kicad_checks_charged: usize,
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
    /// The largest lattice maps of this result's router, in megabytes
    /// (its `memory: N MB in maps`): what the ladder sizes the routers it
    /// runs side by side from (`MemoryLimits`).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub maps_mb: f64,
    /// Route mode: the most work the ladder could do (its budget and the
    /// tolerance, `ladder_work_bound`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_bound_seconds: Option<f64>,
    /// The memory limits as the ladder read them when it started, the
    /// tightest first (`MemoryLimits::describe`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub memory_limits: String,
    /// Attempts of the ladder that panicked, each with its message: the
    /// ladder went on without them (`Ladder`, "fault isolation").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempts_failed: Vec<String>,
    /// The work of the router's manual spokes (pcb-router's
    /// `RoutingResult::manual_spokes_work`, seconds of an idle machine):
    /// beside the search expansions, route mode's ladder counts it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub manual_spokes_work_seconds: f64,
    /// The router's search expansions up to the end of its first
    /// negotiation iteration (`RoutingResult::first_iteration_expansions`).
    #[serde(skip)]
    pub first_iteration_expansions: u64,
    /// The work of the router's pour analyses and fill probes, in every
    /// phase (pcb-router's `RoutingResult::pour_work`; the manual spokes'
    /// among them), seconds of an idle machine: route mode's ladder
    /// charges it beside the search.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pour_work_seconds: f64,
    /// The work of building the router's lattice, modelled
    /// (`RoutingResult::build_work`).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub build_work_seconds: f64,
    /// The parts of `build_work_seconds` routers of the same inputs share
    /// (`Router::shared_build`), for the ladder's charge.
    #[serde(skip)]
    pub shared_build: pcb_router::router::SharedBuild,
    /// The work of lowering the board for this attempt, modelled
    /// (`lower::lowering_work`; the ladder's lowering of the rung's board
    /// where it lowered it for its hold).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub lowering_work_seconds: f64,
    /// Where the run's wall clock went (`KiCadWallAccounting`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_accounting: Option<KiCadWallAccounting>,
    /// What the board's reading left out without an error
    /// (`lower::input_warnings`): copper on a layer the stack lacks, a
    /// text box without its box. Empty on an ordinary board.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// How whole the pours stayed on the board written, per zone and
    /// layer: pieces, the largest one's share, coverage, the other nets'
    /// tracks that cut it (`KiCadPourQuality`), measured by the pour model
    /// after the final route (route mode and a layout's result; not a
    /// layout's own ladder attempts). The verdict's `pours`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pour_quality: Vec<KiCadPourQuality>,
    /// The pads the pour model finds starved on the board as written
    /// (`KiCadPredictedStarved`; what KiCad's DRC should report as
    /// starved thermals), measured with `pour_quality`.
    #[serde(default)]
    pub predicted_starved_thermals: Vec<KiCadPredictedStarved>,
    /// The pads connected solid because the pour model found them starved
    /// on a board KiCad does not check (`measured_board`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub solid_by_model: Vec<String>,
    /// What `solid_starved_thermals` did on the board KiCad checked
    /// (`solidify_starved_thermals`): the pads it connected solid and
    /// whether KiCad's check kept them (`KiCadSolidStarved`); none when it
    /// did not run on this board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solid_starved: Option<KiCadSolidStarved>,
    /// How much of its pours an attempt lost (`pour_loss`), as the
    /// ladder measured it to rank attempts (`pour_integrity`); none when
    /// it was not measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pour_loss: Option<f64>,
    /// Why this board's router ended its last negotiation (pcb-router's
    /// `RoutingResult::negotiation_stop`): `conflict_free`, `stalled`,
    /// `work_deadline`, ... Empty for a router that did not negotiate.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub negotiation_stop: String,
    /// Route mode's ladder: every attempt it judged, in order, the one
    /// whose board was written `selected` (`KiCadAttemptRecord`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempts: Vec<KiCadAttemptRecord>,
    /// Route mode's ladder: whether the lattice had lines at the pin rows
    /// and why, in one line (`pin_lines` set, or the ladder's decision at
    /// the probes or after its leader's run); absent where it ran without
    /// them. Before trial 8 only stderr said so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_lines: Option<String>,
}

/// The tries of `solid_starved_thermals` on a checked board, round by
/// round (`solid_rounds`): the pads KiCad found starved that it connected
/// solid in any round (`REF.NUMBER`), whether a round's board was kept
/// (KiCad finds no more unconnected items on it and fewer findings,
/// `solid_kept`), the rounds checked, the pads of the round KiCad's check
/// undid (it ends the step; empty when none was), and KiCad's unconnected
/// items and design findings before the first round (with spokes) and on
/// the last board checked. Trial 4's Quanta75: U1's exposed pad stayed
/// starved twice while the verdict said the setting connects such pads
/// solid.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct KiCadSolidStarved {
    pub pads: Vec<String>,
    pub kept: bool,
    pub rounds: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub undone: Vec<String>,
    /// With spokes, then on the last board checked.
    pub unconnected: [usize; 2],
    pub findings: [usize; 2],
}

/// One attempt of route mode's ladder, as it was judged: the record
/// behind its `attempt ...` line on stderr (`attempts` in
/// board-router.json). A record of what ran, not an input.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct KiCadAttemptRecord {
    /// Its place in the ladder's order (0 first).
    pub id: usize,
    /// What the ladder tried: the rung, its pitch, its seeds or step
    /// (`rung pours=connect (exclusive planes) pitch=regular`, `the
    /// narrow-signal step`).
    pub name: String,
    pub grid_pitch_mm: f64,
    pub pours: String,
    /// The ladder's ranking count (`Ladder::open`): the router's
    /// unconnected terminals and internal violations, plus KiCad's
    /// unconnected items when KiCad checked the attempt (`counted_by`
    /// `kicad`; `router` when the check was skipped because the attempt
    /// could not win); and the starved thermals.
    pub open: usize,
    pub starved: usize,
    pub counted_by: String,
    /// The native severity-error count when the rung was fully checked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_candidate_errors: Option<usize>,
    pub vias: usize,
    pub iterations: usize,
    /// Its work as the ladder charged it and its routing's wall clock.
    pub work_seconds: f64,
    pub routing_seconds: f64,
    /// Why its router ended its negotiation (`negotiation_stop`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub negotiation_stop: String,
    /// Whether its lattice had lines at the pin rows (`pin_lines`, set or
    /// decided by the ladder).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pin_lines: bool,
    /// Whether its board is the one written.
    pub selected: bool,
}

/// Where a route's wall clock went, beside its work: the routers' phases
/// summed over every router and thread of the process (routers side by
/// side add up, so the thread seconds can exceed the wall clock), with
/// the work each counted. Route mode's ladder charges every kind of it
/// (`charged`; a layout's ladder the search expansions and KiCad's
/// checks); what the phases' thread seconds have beyond their work is
/// the load, or work no model counts.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct KiCadWallAccounting {
    /// The ladder's wall clock and its work.
    pub wall_seconds: f64,
    pub work_seconds: f64,
    /// The machine's search rate measured at the start over the rate
    /// applied (1 with the rates measured; below 1 where a pinned rate is
    /// faster than the machine) and the wall guards' stretch for it.
    pub machine_share: f64,
    pub wall_stretch: f64,
    /// Lattices built (`Router::new`) and their thread seconds.
    pub lattices: u64,
    pub lattice_seconds: f64,
    /// Lowerings of the board and their thread seconds.
    pub lowerings: u64,
    pub lowering_seconds: f64,
    /// The routers' preparation (escapes, skeletons, plane stubs, the
    /// global plan), thread seconds.
    pub prepare_seconds: f64,
    /// Per router phase: its thread seconds, its search work and its pour
    /// analyses' work (seconds of an idle machine).
    pub phases: Vec<KiCadPhaseAccounting>,
    /// kicad-cli runs (the DRC cache's answers among them) and their
    /// thread seconds.
    pub kicad_runs: u64,
    pub kicad_seconds: f64,
    /// The lattices' and the lowerings' work as modelled (thread
    /// seconds of the reference: `RoutingResult::build_work`,
    /// `lower::lowering_work`), beside their thread seconds above.
    #[serde(default)]
    pub lattice_work_seconds: f64,
    #[serde(default)]
    pub lowering_work_seconds: f64,
    /// The ladder's work by kind (`work_seconds` is their sum).
    #[serde(default)]
    pub charged: KiCadWorkCharges,
}

/// The ladder's work by kind, in seconds of work (`Ladder::charge`): the
/// search expansions, the pour analyses and fill probes, the lattice
/// builds, the lowerings, KiCad's checks (a layout's). Route mode charges
/// all of them since 2026-10-21.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct KiCadWorkCharges {
    pub search_seconds: f64,
    pub pour_seconds: f64,
    pub lattice_seconds: f64,
    pub lowering_seconds: f64,
    pub kicad_check_seconds: f64,
}

/// One phase of `KiCadWallAccounting`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct KiCadPhaseAccounting {
    pub phase: String,
    pub wall_seconds: f64,
    pub search_work_seconds: f64,
    pub pour_work_seconds: f64,
}

impl KiCadWallAccounting {
    /// The process's accounting so far, for a ladder of `wall` seconds and
    /// `work` seconds of work of the kinds `charged`.
    pub(crate) fn now(wall: f64, work: f64, charged: KiCadWorkCharges) -> Self {
        let routers = pcb_router::router::process_accounting();
        let (lowerings, lowering_seconds) = super::lower::lowering_totals();
        let (kicad_runs, kicad_seconds) = crate::native_report::kicad_totals();
        let rate = crate::work_rate::work_rate();
        let machine_share = rate.measured.as_ref().map_or(1.0, |measured| {
            measured.expansions_per_second / rate.expansions_per_second
        });
        Self {
            wall_seconds: wall,
            work_seconds: work,
            machine_share,
            wall_stretch: rate.wall_stretch(false),
            lattices: routers.builds,
            lattice_seconds: routers.build_seconds,
            lowerings,
            lowering_seconds,
            prepare_seconds: routers.prepare_seconds,
            phases: routers
                .phases
                .iter()
                .map(|(phase, wall, search, pour)| KiCadPhaseAccounting {
                    phase: phase.to_string(),
                    wall_seconds: *wall,
                    search_work_seconds: *search,
                    pour_work_seconds: *pour,
                })
                .collect(),
            kicad_runs,
            kicad_seconds,
            lattice_work_seconds: routers.build_work_seconds,
            lowering_work_seconds: super::lower::lowering_work_total(),
            charged,
        }
    }

    /// One line for the log.
    pub fn line(&self) -> String {
        let phases: Vec<String> = self
            .phases
            .iter()
            .filter(|phase| phase.wall_seconds >= 0.05)
            .map(|phase| {
                format!(
                    "{} {:.1} (search work {:.1}, pour work {:.1})",
                    phase.phase,
                    phase.wall_seconds,
                    phase.search_work_seconds,
                    phase.pour_work_seconds
                )
            })
            .collect();
        let charged = &self.charged;
        format!(
            "wall accounting: {:.1} s of wall clock for {:.1} s of the ladder's work (search {:.1}, pour {:.1}, lattices {:.1}, lowerings {:.2}, KiCad's checks {:.1}; the machine at {:.2} of the work rate, the wall guards stretched {:.2} times); thread seconds: {} lattices {:.1} (work {:.1}), {} lowerings {:.1} (work {:.2}), preparation {:.1}, {}; {} kicad-cli runs {:.1}",
            self.wall_seconds,
            self.work_seconds,
            charged.search_seconds,
            charged.pour_seconds,
            charged.lattice_seconds,
            charged.lowering_seconds,
            charged.kicad_check_seconds,
            self.machine_share,
            self.wall_stretch,
            self.lattices,
            self.lattice_seconds,
            self.lattice_work_seconds,
            self.lowerings,
            self.lowering_seconds,
            self.lowering_work_seconds,
            self.prepare_seconds,
            phases.join(", "),
            self.kicad_runs,
            self.kicad_seconds
        )
    }
}

fn is_zero(value: &f64) -> bool {
    *value == 0.0
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
    /// The nets the narrow-signal step routed below their class's width,
    /// per class (absent when the step did not run or narrowed none):
    /// KiCad's DRC does not check a class's width, so nothing else says it
    /// (trial 7: Sisu's RF_ANT nets, 0.1422 mm for 50 ohms, written at
    /// 0.1 mm).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub narrowed: Vec<KiCadNarrowedClass>,
    /// Whether vias could pierce only some layers (`blind_vias`, or
    /// `PCB_ROUTER_BLIND_VIAS`): without it every via is a through via,
    /// whatever the designer's board used (`via_spans_designer`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub blind_vias: bool,
    /// KiCad's defaults taken and the custom rules read, kept or skipped.
    #[serde(flatten)]
    pub project: crate::project_rules::KiCadProjectRulesReport,
}

/// A net class whose nets the narrow-signal step routed narrower than the
/// class's width (`KiCadRulesSummary::narrowed`).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct KiCadNarrowedClass {
    /// The project's class that gives the nets their width ("Default" for
    /// nets in none).
    pub class: String,
    pub class_width_mm: f64,
    /// The width the step routed them at (the project's narrowest
    /// predefined width, `narrow_signal_mm`).
    pub routed_width_mm: f64,
    /// The nets, as the router names them.
    pub nets: Vec<String>,
}

/// The classes the narrow-signal step narrowed: per net of `nets` (name and
/// routed width) whose width lies below its class's, grouped by the class
/// (`net_classes`: a net's classes, the highest priority first;
/// `class_values`: the classes' own widths), the classes other than Default
/// first, each by its nets.
pub fn narrowed_classes(
    nets: &[(String, f64)],
    class_width: impl Fn(&str) -> f64,
    net_classes: &BTreeMap<String, Vec<String>>,
    class_values: &BTreeMap<String, [Option<f64>; 2]>,
) -> Vec<KiCadNarrowedClass> {
    let mut classes: BTreeMap<(String, i64, i64), Vec<String>> = BTreeMap::new();
    for (name, routed) in nets {
        let width = class_width(name);
        if *routed >= width - 1.0e-6 {
            continue;
        }
        // The class whose width the net takes: the first by priority that
        // states one.
        let class =
            crate::project_rules::width_class(net_classes.get(name), class_values).to_string();
        let micro = |value: f64| (value * 1.0e6).round() as i64;
        classes
            .entry((class, micro(width), micro(*routed)))
            .or_default()
            .push(name.clone());
    }
    let mut narrowed: Vec<KiCadNarrowedClass> = classes
        .into_iter()
        .map(|((class, width, routed), nets)| KiCadNarrowedClass {
            class,
            class_width_mm: width as f64 / 1.0e6,
            routed_width_mm: routed as f64 / 1.0e6,
            nets,
        })
        .collect();
    narrowed.sort_by_key(|class| {
        (
            class.class == "Default",
            std::cmp::Reverse(class.nets.len()),
        )
    });
    narrowed
}

impl KiCadRulesSummary {
    pub(super) fn of(board: &core::Board, config: &KiCadBoardRouterConfig) -> Self {
        let narrowest = board
            .nets
            .iter()
            .map(|net| board.classes[net.class].trace_width)
            .fold(f64::INFINITY, f64::min);
        Self {
            width_floor_mm: config.min_connection_mm.filter(|floor| *floor > 0.0),
            connection_width_severity: config.connection_width_severity.clone(),
            narrowest_track_mm: if narrowest.is_finite() {
                narrowest
            } else {
                0.0
            },
            neck_width_mm: board.neck_width,
            blind_vias: super::via_spans::blind_vias_on(config),
            narrowed: if config.use_narrow_signals == Some(true) {
                let project = config.project_rules.clone().unwrap_or_default();
                let nets: Vec<(String, f64)> = board
                    .nets
                    .iter()
                    .map(|net| (net.name.clone(), board.classes[net.class].trace_width))
                    .collect();
                narrowed_classes(
                    &nets,
                    |name| {
                        config
                            .connection_rules
                            .get(name)
                            .or(config.default_rules.as_ref())
                            .map_or(0.0, |rules| rules.trace_width_mm)
                    },
                    &project.net_classes,
                    &project.class_values,
                )
            } else {
                Vec::new()
            },
            project: config.project_rules.clone().unwrap_or_default(),
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
    /// first, each with its tile's pads, nets and nets still in conflict
    /// (`KiCadHotSpotLinks`): observations, not a cause.
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
pub(super) fn starved_thermal_diagnostics(directory: &Path) -> Vec<KiCadStarvedThermal> {
    let Ok(text) = fs::read_to_string(directory.join("drc.json")) else {
        return Vec::new();
    };
    let Ok(report) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let number_after = |text: &str, key: &str| -> Option<usize> {
        let rest = &text[text.find(key)? + key.len()..];
        rest.trim_start()
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()
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
            .find(|item| {
                item["description"]
                    .as_str()
                    .is_some_and(|text| text.contains("ad "))
            })
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
            at: [
                pad["pos"]["x"].as_f64().unwrap_or(0.0),
                pad["pos"]["y"].as_f64().unwrap_or(0.0),
            ],
            spokes,
            needed,
            advice,
        });
    }
    found
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadDeadPad {
    pub pad: String,
    pub net: String,
    pub at: [f64; 2],
    pub near: Vec<String>,
    /// What walls the pad: per layer, the item nearest to being cleared
    /// at the pad's best point for the net's narrowest track, or what
    /// blocks the stubs from a legal track end to the nearest nodes.
    pub wall: String,
    /// How open the pad is, the most open of its layers
    /// (`pcb_router::DeadPadAccess::name`): `no_legal_access` (no point
    /// of the pad keeps the clearances for the net's narrowest track: by
    /// the router's rules no track ends on it), `off_lattice` (no node
    /// within 2 mm: the pad lies off the routed area), `no_lattice_node` (a track
    /// may end on it, no node within 2 mm admits the class's track),
    /// `stubs_blocked`, `start_blocked` (free from the pad's best point,
    /// not from where the router's stubs start), `escape_not_kept`;
    /// `unknown` where not measured. Only the first two are walls of the
    /// input by the router's rules; KiCad's DRC is the judge.
    #[serde(default = "unknown_access")]
    pub access: String,
}

fn unknown_access() -> String {
    "unknown".into()
}

impl KiCadDeadPad {
    pub(crate) fn of(
        dead: &core::router::DeadPad,
        board: &core::Board,
        layer_names: &[String],
    ) -> Self {
        Self {
            pad: dead.label.clone(),
            net: board.nets[dead.net as usize].name.clone(),
            at: dead.anchor.map(|value| (value * 1000.0).round() / 1000.0),
            near: dead.near.clone(),
            wall: name_layers(&dead.wall, layer_names),
            access: dead.access.name().into(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadHotSpot {
    pub layer: String,
    pub at: [f64; 2],
    pub history: f32,
    /// The tile: `[min x, min y, max x, max y]` in mm.
    #[serde(rename = "box")]
    pub bounds: [f64; 4],
    /// What lies in the tile on its layer when the router stopped
    /// (`hot_spot_links`): observations, not a cause.
    #[serde(flatten)]
    pub links: KiCadHotSpotLinks,
}

/// The objects of a hot spot's tile on its layer, as the board stood when
/// the router stopped: the pads whose copper reaches into the tile, the
/// nets whose copper (tracks, vias, kept copper) crosses it, and those of
/// the nets of both that were still in conflict.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct KiCadHotSpotLinks {
    /// Nearest the tile's centre first, at most `HOT_SPOT_PADS`.
    pub pads: Vec<KiCadHotSpotPad>,
    /// All pads in the tile (`pads` may list fewer).
    pub pads_in_tile: usize,
    /// Sorted.
    pub nets: Vec<String>,
    /// The nets of `pads` and `nets` the router still had in conflict
    /// when negotiation stopped (`conflicted_nets`), sorted.
    pub conflicted_nets: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct KiCadHotSpotPad {
    #[serde(rename = "ref")]
    pub reference: String,
    pub number: String,
    pub net: Option<String>,
    pub at: [f64; 2],
}

/// How many pads a hot spot lists.
pub const HOT_SPOT_PADS: usize = 12;

/// The pad an obstacle is (`REF.NUMBER`, a padstack layer's
/// `REF.NUMBER padstack`), split into reference and number.
pub fn pad_of_label(label: &str) -> Option<(&str, &str)> {
    let label = label.strip_suffix(" padstack").unwrap_or(label);
    if label.contains(' ') {
        return None;
    }
    label.split_once('.')
}

/// The objects in a hot spot's tile (`bounds`, `[min x, min y, max x,
/// max y]`) on `layer`: pads of `board` (copper obstacles labelled as
/// pads) whose copper reaches into it, and the nets whose copper crosses
/// it: `board`'s own copper of a net (kept tracks and vias, graphics) and
/// `routes` (per net, the router's). `conflicted`: the nets still in
/// conflict.
pub fn hot_spot_links(
    board: &core::Board,
    routes: &[core::NetRoute],
    layer: usize,
    bounds: [f64; 4],
    conflicted: &BTreeSet<String>,
) -> KiCadHotSpotLinks {
    let tile = core::Shape::Polygon {
        points: vec![
            [bounds[0], bounds[1]],
            [bounds[2], bounds[1]],
            [bounds[2], bounds[3]],
            [bounds[0], bounds[3]],
        ],
    };
    let tile_box = tile.aabb();
    let reaches = |shape: &core::Shape| {
        shape.aabb().intersects(tile_box) && core::verify::shape_distance(shape, &tile) <= 0.0
    };
    let net_name = |net: core::NetId| board.nets.get(net as usize).map(|net| net.name.clone());
    let centre = [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0];
    let mut pads: Vec<(f64, KiCadHotSpotPad)> = Vec::new();
    let mut nets: BTreeSet<String> = BTreeSet::new();
    for obstacle in &board.obstacles {
        if obstacle.kind != core::ObstacleKind::Copper
            || obstacle.layers & (1 << layer) == 0
            || !reaches(&obstacle.shape)
        {
            continue;
        }
        match pad_of_label(&obstacle.label) {
            Some((reference, number)) => {
                let aabb = obstacle.shape.aabb();
                let at = [
                    (aabb.minimum[0] + aabb.maximum[0]) / 2.0,
                    (aabb.minimum[1] + aabb.maximum[1]) / 2.0,
                ];
                let pad = KiCadHotSpotPad {
                    reference: reference.to_string(),
                    number: number.to_string(),
                    net: obstacle.net.and_then(net_name),
                    at: [
                        (at[0] * 10_000.0).round() / 10_000.0,
                        (at[1] * 10_000.0).round() / 10_000.0,
                    ],
                };
                if !pads.iter().any(|(_, known)| {
                    known.reference == pad.reference && known.number == pad.number
                }) {
                    let distance = (at[0] - centre[0]).hypot(at[1] - centre[1]);
                    pads.push((distance, pad));
                }
            }
            None => {
                if let Some(name) = obstacle.net.and_then(net_name) {
                    nets.insert(name);
                }
            }
        }
    }
    for (net, route) in routes.iter().enumerate() {
        let crosses = route.segments.iter().any(|segment| {
            segment.layer == layer
                && reaches(&core::Shape::Capsule {
                    start: segment.start,
                    end: segment.end,
                    radius: segment.width / 2.0,
                })
        }) || route.vias.iter().any(|via| {
            via.on_layer(layer)
                && reaches(&core::Shape::Circle {
                    center: via.at,
                    radius: via.diameter / 2.0,
                })
        });
        if crosses && let Some(name) = net_name(net as core::NetId) {
            nets.insert(name);
        }
    }
    pads.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then_with(|| a.1.reference.cmp(&b.1.reference))
            .then_with(|| a.1.number.cmp(&b.1.number))
    });
    let pads_in_tile = pads.len();
    let pads: Vec<KiCadHotSpotPad> = pads
        .into_iter()
        .take(HOT_SPOT_PADS)
        .map(|(_, pad)| pad)
        .collect();
    let conflicted_nets: Vec<String> = nets
        .iter()
        .cloned()
        .chain(pads.iter().filter_map(|pad| pad.net.clone()))
        .filter(|net| conflicted.contains(net))
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect();
    KiCadHotSpotLinks {
        pads,
        pads_in_tile,
        nets: nets.into_iter().collect(),
        conflicted_nets,
    }
}

/// `text` (the router's words, which number its layers: "layer 7: ...")
/// with KiCad's layer names (`layer_names`, by the router's index): "B.Cu:
/// ..." (trial 8: a dead pad's wall said "layer 7").
pub(crate) fn name_layers(text: &str, layer_names: &[String]) -> String {
    let mut named = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("layer ") {
        let after = &rest[at + "layer ".len()..];
        let digits = after.chars().take_while(char::is_ascii_digit).count();
        let index = after[..digits].parse::<usize>().ok();
        match index.and_then(|index| layer_names.get(index)) {
            Some(name) if digits > 0 && after[digits..].starts_with(':') => {
                named.push_str(&rest[..at]);
                named.push_str(name);
                rest = &after[digits..];
            }
            _ => {
                named.push_str(&rest[..at + "layer ".len()]);
                rest = after;
            }
        }
    }
    named.push_str(rest);
    named
}

pub(super) fn diagnostics(
    board: &core::Board,
    result: &core::RoutingResult,
    layer_names: &[String],
) -> KiCadRoutingDiagnostics {
    let round = |point: [f64; 2]| {
        [
            (point[0] * 1000.0).round() / 1000.0,
            (point[1] * 1000.0).round() / 1000.0,
        ]
    };
    let conflicted: BTreeSet<String> = result
        .diagnostics
        .conflicted
        .iter()
        .map(|net| board.nets[*net as usize].name.clone())
        .collect();
    KiCadRoutingDiagnostics {
        dead_pads: result
            .diagnostics
            .dead_pads
            .iter()
            .map(|dead| KiCadDeadPad::of(dead, board, layer_names))
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
            .map(|spot| {
                let bounds = [
                    spot.center[0] - spot.half_size,
                    spot.center[1] - spot.half_size,
                    spot.center[0] + spot.half_size,
                    spot.center[1] + spot.half_size,
                ];
                let corners = [round([bounds[0], bounds[1]]), round([bounds[2], bounds[3]])];
                KiCadHotSpot {
                    layer: layer_names.get(spot.layer).cloned().unwrap_or_default(),
                    at: round(spot.center),
                    history: spot.history,
                    bounds: [corners[0][0], corners[0][1], corners[1][0], corners[1][1]],
                    links: hot_spot_links(board, &result.routes, spot.layer, bounds, &conflicted),
                }
            })
            .collect(),
        starved_thermals: Vec::new(),
    }
}

/// A pad KiCad finds short of thermal spokes, for the verdict's line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KiCadStarvedPad {
    /// `REF.NUMBER` (`U1.57`), as `solid_starved.pads` names pads.
    pub pad: String,
    pub layer: String,
    pub advice: String,
}

/// A starved thermal of `diagnostics.starved_thermals` for the verdict's
/// line: its pad as `REF.NUMBER`, the name `solid_starved.pads` gives it
/// (a pad KiCad names without a number, "Pad [GND] of M12", as KiCad
/// names it).
pub fn starved_pad(finding: &crate::board_router::KiCadStarvedThermal) -> KiCadStarvedPad {
    let pad = crate::board_router::solid_pads_of_findings(std::slice::from_ref(finding))
        .first()
        .filter(|pad| !pad.number.is_empty() && !pad.number.contains(['[', ' ']))
        .map_or_else(
            || {
                finding
                    .pad
                    .split(" on ")
                    .next()
                    .unwrap_or(&finding.pad)
                    .to_string()
            },
            |pad| format!("{}.{}", pad.reference, pad.number),
        );
    KiCadStarvedPad {
        pad,
        layer: finding.layer.clone(),
        advice: finding.advice.clone(),
    }
}

#[cfg(test)]
mod layer_name_tests {
    use super::name_layers;

    #[test]
    fn a_dead_pads_wall_names_kicads_layers() {
        let names: Vec<String> = ["F.Cu", "In1.Cu", "B.Cu"].map(String::from).to_vec();
        assert_eq!(
            name_layers(
                "layer 2: a 0.100 mm track may end at (1, 2); layer 0: the contact holds no track end",
                &names
            ),
            "B.Cu: a 0.100 mm track may end at (1, 2); F.Cu: the contact holds no track end"
        );
        // Not a layer of the list, or no colon: as it was.
        assert_eq!(name_layers("layer 9: x", &names), "layer 9: x");
        assert_eq!(
            name_layers("the layer 1 of it", &names),
            "the layer 1 of it"
        );
    }
}

#[cfg(test)]
mod open_report_tests {
    use super::*;

    fn open(net: core::NetId, label: &str, reason: core::OpenReason) -> core::OpenTerminal {
        core::OpenTerminal {
            net,
            terminal: 0,
            label: label.into(),
            reason,
            walled: Vec::new(),
            wall: Vec::new(),
        }
    }

    #[test]
    fn a_nets_open_terminals_are_grouped_by_reason_with_exact_counts() {
        let mut diagnostics = core::Diagnostics::default();
        for index in 0..20 {
            diagnostics.open.push(open(
                0,
                &format!("U1.{index}"),
                core::OpenReason::NoPathHard("board"),
            ));
        }
        diagnostics
            .open
            .push(open(0, "R1.1", core::OpenReason::Lost("clean_up")));
        diagnostics
            .open
            .push(open(1, "R2.1", core::OpenReason::NoPathStatic));
        let groups = open_terminals_of(&diagnostics, 0, 21);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].reason, "no_path_hard:board");
        assert_eq!(groups[0].terminals, 20);
        // The labels bounded, the count exact.
        assert_eq!(groups[0].pads.len(), OPEN_PADS_LISTED);
        assert_eq!(groups[0].pads[0], "U1.0");
        assert_eq!(
            groups[1],
            KiCadOpenTerminals {
                reason: "lost:clean_up".into(),
                terminals: 1,
                pads: vec!["R1.1".into()],
                walled: Vec::new(),
                wall: None,
                blocked_by: Vec::new(),
            }
        );
        // Fewer recorded than the net's count: the rest is `unknown`.
        let groups = open_terminals_of(&diagnostics, 1, 3);
        assert_eq!(groups[0].reason, "no_path_static");
        assert_eq!(
            groups[1],
            KiCadOpenTerminals {
                reason: "unknown".into(),
                terminals: 2,
                pads: Vec::new(),
                walled: Vec::new(),
                wall: None,
                blocked_by: Vec::new(),
            }
        );
        assert!(open_terminals_of(&diagnostics, 2, 0).is_empty());
    }

    #[test]
    fn a_static_wall_names_the_walled_pads_and_whose_copper_it_is() {
        let walled = |label: &str, reason, at: &[&str], wall: &[&str]| core::OpenTerminal {
            walled: at.iter().map(|pad| pad.to_string()).collect(),
            wall: wall.iter().map(|label| label.to_string()).collect(),
            ..open(0, label, reason)
        };
        let group = |open: Vec<core::OpenTerminal>| {
            let diagnostics = core::Diagnostics {
                open,
                ..Default::default()
            };
            let count = diagnostics.open.len();
            let mut groups = open_terminals_of(&diagnostics, 0, count);
            assert_eq!(groups.len(), 1);
            groups.remove(0)
        };
        // The tree started at the walled pad: the open pads are the ones
        // it did not reach, the wall is at C18.1, by the router's own
        // fan-out vias.
        let router = group(vec![
            walled(
                "U30.H8",
                core::OpenReason::NoPathStatic,
                &["C18.1"],
                &["U30.G8 fan-out hole", "U30.G9 fan-out via"],
            ),
            walled(
                "C20.1",
                core::OpenReason::NoPathStatic,
                &["C18.1"],
                &["U30.G8 fan-out hole", "U30.G9 fan-out via"],
            ),
        ]);
        assert_eq!(router.reason, "no_path_static");
        assert_eq!(router.pads, ["U30.H8", "C20.1"]);
        assert_eq!(router.walled, ["C18.1"]);
        assert_eq!(router.wall, Some("router"));
        assert_eq!(
            router.blocked_by,
            ["U30.G8 fan-out hole", "U30.G9 fan-out via"]
        );
        // The board's own pads and edge.
        let input = group(vec![walled(
            "R62.1",
            core::OpenReason::NoPathStatic,
            &["R62.1"],
            &["R62.2", "board edge"],
        )]);
        assert_eq!(input.wall, Some("input"));
        // Both.
        let both = group(vec![walled(
            "U11.R13",
            core::OpenReason::NoPathStatic,
            &["U11.R13"],
            &["SW9.1 via in pad", "U11.R12"],
        )]);
        assert_eq!(both.wall, Some("input_and_router"));
        // Every label the lowering gives the copper it plans.
        for label in [
            "U30.G8 fan-out stub",
            "U30.G8 fan-out via ring",
            "U30.G8 fan-out via",
            "U30.G8 fan-out hole",
            "SW9.1 via in pad ring",
            "SW9.1 via in pad",
            "SW9.1 via in pad hole",
        ] {
            assert!(router_planned(label), "{label}");
        }
        assert!(!router_planned("U11 keepout") && !router_planned("board edge"));
        // The lattice's wall names the pads and no copper; a wall that is
        // at no pad names nothing.
        let lattice = group(vec![walled(
            "U1.6",
            core::OpenReason::NoLatticeExit,
            &["U1.6"],
            &[],
        )]);
        assert_eq!(lattice.reason, "no_lattice_exit");
        assert_eq!(lattice.walled, ["U1.6"]);
        assert_eq!((lattice.wall, lattice.blocked_by.len()), (None, 0));
        let elsewhere = group(vec![open(0, "J3.5", core::OpenReason::NoPathStatic)]);
        assert!(elsewhere.walled.is_empty() && elsewhere.wall.is_none());
        let json = serde_json::to_value(&elsewhere).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"reason": "no_path_static", "terminals": 1, "pads": ["J3.5"]})
        );
    }

    #[test]
    fn the_repairs_entry_names_the_nets_and_kicads_layer() {
        let net = |name: &str| core::Net {
            name: name.into(),
            class: 0,
            terminals: Vec::new(),
        };
        let nets = vec![net("A"), net("B"), net("C")];
        let names: Vec<String> = ["F.Cu", "B.Cu"].map(String::from).to_vec();
        let mut diagnostics = core::Diagnostics::default();
        diagnostics.repair.push(core::RepairEntry {
            net: 0,
            complete: true,
            conflicted: true,
            skipped: "budget",
            conflict_at: Some((1, [1.23456, 7.0])),
            conflicted_with: vec![1, 2],
        });
        diagnostics.repair.push(core::RepairEntry {
            net: 1,
            complete: false,
            conflicted: true,
            skipped: "",
            conflict_at: Some((2, [0.0, 0.0])),
            conflicted_with: vec![0],
        });
        assert_eq!(
            net_repair_of(&diagnostics, &nets, 2, 0, &names),
            Some(KiCadNetRepair {
                complete: true,
                in_conflict: true,
                skipped: "budget".into(),
                conflicted_with: vec!["B".into(), "C".into()],
                conflict_at: Some([1.235, 7.0]),
                conflict_layer: Some("B.Cu".into()),
            })
        );
        // The layer count stands for the via map.
        let via = net_repair_of(&diagnostics, &nets, 2, 1, &names).unwrap();
        assert_eq!(via.conflict_layer.as_deref(), Some("via"));
        assert!(net_repair_of(&diagnostics, &nets, 2, 2, &names).is_none());
    }

    #[test]
    fn a_panels_copies_of_a_net_are_counted_in_net_order() {
        let names = [
            Some("GND"),
            Some("A"),
            Some("GND"),
            None,
            Some("A"),
            Some("GND"),
            Some("B"),
        ];
        assert_eq!(
            crate::board_router::attempt::copy_indices(&names),
            vec![Some(0), Some(0), Some(1), None, Some(1), Some(2), None]
        );
        // A copy not reported is not counted.
        assert_eq!(
            crate::board_router::attempt::copy_indices(&[Some("C"), None]),
            vec![None, None]
        );
    }
}
