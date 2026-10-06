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
    /// Price of sharing when a via reduction round starts.
    #[serde(default)]
    pub via_reduction_present: Option<f64>,
    /// Keep other nets' tracks off inner-layer planes entirely.
    #[serde(default)]
    pub exclusive_planes: Option<bool>,
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
    #[serde(default)]
    pub cleanup_via_cost_mm: Option<f64>,
    #[serde(default)]
    pub cleanup_passes: Option<usize>,
    #[serde(default)]
    pub bend_cost_mm: Option<f64>,
    #[serde(default)]
    pub via_reduction_rounds: Option<usize>,
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

pub(super) fn lower(
    pcb: &Expr,
    config: &KiCadBoardRouterConfig,
    connect_pours: bool,
) -> Result<Lowered, String> {
    note_jumper_nets(pcb);
    let loops = outline::board_loops(pcb)?;
    let layers = LayerTable::from_pcb(pcb)?;
    let mut classes: Vec<core::RuleClass> = Vec::new();
    let mut nets: Vec<core::Net> = Vec::new();
    let mut net_ids = BTreeMap::<String, core::NetId>::new();
    let mut obstacles = Vec::new();
    let mut solid_pads: Vec<String> = Vec::new();
    // Openings in the solder mask (graphics on a mask layer), placed once
    // the pads under them are known.
    let mut mask_openings: Vec<(Vec<core::Shape>, usize, String)> = Vec::new();
    // Board copper graphics, with the net they carry, placed once the nets
    // of the pads are known.
    let mut copper_graphics: Vec<(Vec<core::Shape>, core::LayerMask, Option<String>, String)> = Vec::new();
    let mut isolated_pads: Vec<String> = Vec::new();
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
            trace_width: narrow.unwrap_or(rules.trace_width_mm),
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
                            mask_openings.push((shapes, layer, graphic.head().unwrap_or("graphic").to_string()));
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
                    // Other nets keep the hole clearance from a plated hole
                    // too, which reaches past a thin annular ring (a thermal
                    // via pad: 0.3 drill in 0.5 copper, eurorack-pmod's U1).
                    // Vias stay with the pad's copper rule.
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
                            blocks_vias: false,
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
                for side in sides {
                    mask_openings.push((shapes.clone(), side, kind.clone()));
                }
            }
            Some("zone") if is_rule_area(item) => {
                if let Some(obstacle) = rule_area_obstacle(item, &layers, "unnamed rule area")? {
                    obstacles.push(obstacle);
                }
            }
            _ => {}
        }
    }
    // Pads of one number that overlap are one pad to KiCad (a solder pad
    // with holes in it, an exposed pad's thermal vias): the largest is the
    // terminal, the others just its copper. OpenRX's panel hangs half of
    // each wire pad's holes over the edge, unreachable as terminals.
    for net in &mut nets {
        let area = |terminal: &core::Terminal| {
            let bounds = obstacles[terminal.pad].shape.aabb();
            (bounds.maximum[0] - bounds.minimum[0]) * (bounds.maximum[1] - bounds.minimum[1])
        };
        let mut order: Vec<usize> = (0..net.terminals.len()).collect();
        order.sort_by(|a, b| area(&net.terminals[*b]).total_cmp(&area(&net.terminals[*a])).then(a.cmp(b)));
        let mut keep = vec![true; net.terminals.len()];
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
                }
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
    for (shapes, layer, kind) in mask_openings {
        let bounds = shapes.iter().skip(1).fold(shapes[0].aabb(), |bounds, shape| bounds.union(shape.aabb()));
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
        let net = match exposed.len() {
            1 => exposed.first().copied(),
            // Only text's glyph strokes open the mask, and its estimated
            // box would close whole pad rows (OpenESC's B.Mask label over
            // U11): over bare board it opens nothing that matters.
            0 if kind == "gr_text" => continue,
            _ => None,
        };
        obstacles.extend(shapes.into_iter().map(|shape| core::Obstacle {
            shape,
            layers: 1 << layer,
            kind: core::ObstacleKind::Keepout,
            net,
            clearance: 0.0,
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
                    .collect(),
                connect: connect_pours,
                thermal_reach: pour.thermal_reach,
                thermal_gap: pour.thermal_gap,
                // An inner layer poured over nearly the whole board is a
                // plane: it carries no signals.
                exclusive: layers.names[layer].starts_with("In") && polygon_area(&pour.polygon) >= 0.9 * board_area,
                solid: pour.solid,
            });
        }
    }
    if classes.is_empty() {
        return Err("the board has no routable connections".into());
    }
    Ok(Lowered {
        board: core::Board {
            layer_count: layers.len(),
            neck_width: config.neck_width_mm.unwrap_or_else(|| {
                classes
                    .iter()
                    .map(|class| class.trace_width)
                    .fold(f64::INFINITY, f64::min)
            }),
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
        },
    })
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
    Ok(match item.head() {
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
            vec![core::Shape::rectangle(
                [at[0] + offset[0], at[1] + offset[1]],
                half,
                -at[2],
            )]
        }
        _ => Vec::new(),
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
    net: String,
    layers: core::LayerMask,
    priority: i64,
    clearance: f64,
    min_thickness: f64,
    thermal_reach: f64,
    thermal_gap: f64,
    /// `(connect_pads yes ...)`: pads join wherever the fill touches.
    solid: bool,
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
            polygon: rule_area_polygon_points(zone)?,
        });
    }
    Ok(result)
}

fn nanometres(point: [f64; 2]) -> [f64; 2] {
    point.map(|value| (value * 1.0e6).round() / 1.0e6)
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
        with_classes = crate::net_classes::with_net_classes(config, &parse(&text)?)?;
        &with_classes
    };
    // With interchangeable pins, the ladder routes a copy with the pins
    // assigned; every attempt copies its project from there.
    // Requested pours and net classes go into a copy of the project that the
    // attempts use.
    let poured_directory = output_directory.with_extension("poured");
    let planned = {
        let board = source_directory.join(format!("{board_id}.kicad_pcb"));
        let text = fs::read_to_string(&board).map_err(|error| format!("failed to read {}: {error}", board.display()))?;
        planned_pours(&parse(&text)?, config)?
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
        add_pour_zones(&mut pcb, &planned)?;
        fs::write(&board, format!("{}\n", encode(&pcb))).map_err(|error| format!("failed to write {}: {error}", board.display()))?;
        if !config.net_classes.is_empty() {
            crate::net_classes::write_net_classes(&poured_directory, board_id, &pcb, &config.net_classes)?;
        }
        poured_directory.as_path()
    };
    let swapped_directory = output_directory.with_extension("swapped");
    let (source_directory, pin_swaps) = match &config.pin_swaps {
        Some(path) => {
            let spec = read_pin_swap_spec(&source_directory.join(path))?;
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
    let mut extra_rung_tried = false;
    let ladder_budget = config.ladder_budget_seconds.unwrap_or(1200.0);
    let ladder_started = std::time::Instant::now();
    // The ladder's budget is spent in work (search expansions in seconds of
    // an idle machine), so that a busy machine routes the same way; the
    // wall clock counts only beyond `GUARD` times that.
    let mut ladder_work = 0.0f64;
    let work_of = |expansions: u64| expansions as f64 / core::router::EXPANSIONS_PER_SECOND;
    let spent = |work: f64| work.max(ladder_started.elapsed().as_secs_f64() / core::router::GUARD);
    let past_deadline = || config.deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline);
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
        for (mode, (connect, skeleton, exclusive, plane_stubs)) in modes.iter().enumerate() {
            if leader.is_some() && past_deadline() {
                break;
            }
            let mut attempt = config.clone();
            attempt.plane_skeleton = Some(*skeleton);
            attempt.exclusive_planes = Some(*exclusive);
            attempt.fixed_plane_stubs = Some(*plane_stubs);
            let board = lower(&parsed, &attempt, *connect)?.board;
            let connections: usize = board.nets.iter().map(|net| net.terminals.len().saturating_sub(1)).sum();
            if mode == 0 && (board.layer_count < 4 || connections < 120) {
                break;
            }
            let started = std::time::Instant::now();
            let mut router = core::router::Router::new(&board, &core_config(&attempt));
            let unfinished = router.probe(probe_seconds);
            ladder_work += work_of(router.expansions());
            eprintln!(
                "probe pours={}{}: {unfinished} nets unfinished after {:.0} s",
                if *connect { "connect" } else { "tracks" },
                if *exclusive { " (exclusive planes)" } else if *plane_stubs { " (plane stubs)" } else { "" },
                started.elapsed().as_secs_f64()
            );
            if leader.as_ref().is_none_or(|(best, ..)| unfinished < *best) {
                leader = Some((unfinished, mode, router, board, attempt));
            }
            // A rung that already finishes everything needs no rival.
            if leader.as_ref().is_some_and(|(best, ..)| *best == 0) {
                break;
            }
        }
        if let Some((_, mode, mut router, board, attempt)) = leader {
            let started = std::time::Instant::now();
            let remaining = (ladder_budget - spent(ladder_work)).max(60.0);
            let probed = router.expansions();
            let routed = router.resume((remaining / 3.0).clamp(60.0, 900.0));
            ladder_work += work_of(router.expansions() - probed);
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
                break;
            }
        }
        for (mode, (connect, skeleton, exclusive, plane_stubs)) in modes.iter().enumerate() {
            // A probed rung continued with what it had is not run again;
            // the other rungs still get their turn when it left
            // connections open.
            if pitch.is_none() && probed_mode == Some(mode) {
                continue;
            }
            if best.is_some() && past_deadline() {
                eprintln!("deadline reached: no further attempts");
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
                continue;
            }
            let mut attempt = config.clone();
            attempt.plane_skeleton = Some(*skeleton);
            attempt.exclusive_planes = Some(*exclusive);
            attempt.fixed_plane_stubs = Some(*plane_stubs);
            // The rungs left at this pitch share what is left of the
            // ladder's budget evenly; an attempt spends up to about three
            // times its negotiation in repair and polish, so negotiation
            // gets a third of the share (zpn_devboard: two rungs took 1300
            // s and the two that complete such boards never ran).
            if config.negotiation_seconds.is_none() {
                let remaining = (ladder_budget - spent(ladder_work)).max(0.0);
                let rungs_left = (modes.len() - mode) as f64;
                attempt.negotiation_seconds = Some((remaining / rungs_left / 3.0).clamp(60.0, 900.0));
            }
            if let Some(pitch) = pitch {
                attempt.grid_pitches_mm = Some(pitch.clone());
            }
            let directory = if best.is_none() {
                output_directory.to_path_buf()
            } else {
                scratch.clone()
            };
            if directory.exists() {
                fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
            }
            let mut result =
                route_kicad_board_once(source_directory, board_id, &directory, &attempt, *connect)?;
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
            // What one attempt at this pitch costs, for projecting finer
            // pitches; a seed retry does not change it.
            // In work, as the ladder's budget.
            let attempt_seconds = work_of(result.expansions);
            ladder_work += attempt_seconds;
            // Open connections often depend on the order: route the same
            // attempt a few perturbed ways and keep the better board.
            let retry = config.retry_seeds.unwrap_or(4);
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
            {
                seeds_tried = true;
                let mut seeded = attempt.clone();
                seeded.seeds = Some(retry);
                seeded.first_seed = Some(1);
                let retry_directory = output_directory.with_extension("seeds");
                if retry_directory.exists() {
                    fs::remove_dir_all(&retry_directory).map_err(|error| error.to_string())?;
                }
                let mut other =
                    route_kicad_board_once(source_directory, board_id, &retry_directory, &seeded, *connect)?;
                other.pours = result.pours.clone();
                // The seeds run side by side: about `retry` times one's work.
                ladder_work += work_of(other.expansions) * retry as f64;
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
                if directory != output_directory {
                    fs::remove_dir_all(output_directory).map_err(|error| error.to_string())?;
                    fs::rename(&directory, output_directory).map_err(|error| error.to_string())?;
                }
                best = Some((opens, result));
                best_attempt = Some((attempt.clone(), *connect));
                best_seeded = seeds_tried || attempt.seeds.unwrap_or(1) > 1;
            } else if directory.exists() {
                fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
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
                break 'ladder;
            }
        }
    }
    // The best rung's deferred seed retry.
    let retry = config.retry_seeds.unwrap_or(4);
    if let (Some((opens, best_result)), Some((attempt, connect))) = (best.as_ref(), best_attempt.as_ref())
        && opens.0 > 0
        && !best_seeded
        && retry > 0
        && work_of(best_result.expansions) <= budget / 2.0
        && spent(ladder_work) < 1.5 * ladder_budget
        && !past_deadline()
    {
        let mut seeded = attempt.clone();
        seeded.seeds = Some(retry);
        seeded.first_seed = Some(1);
        if scratch.exists() {
            fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
        }
        let connect = *connect;
        let mut other = route_kicad_board_once(source_directory, board_id, &scratch, &seeded, connect)?;
        other.pours = best_result.pours.clone();
        ladder_work += work_of(other.expansions) * retry as f64;
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
        && config.connection_rules.values().chain(config.default_rules.iter()).any(|rules| rules.trace_width_mm > narrow + 1.0e-9)
    {
        let mut narrowed = attempt.clone();
        narrowed.use_narrow_signals = Some(true);
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
        result.pours = best.as_ref().map(|(_, best)| best.pours.clone()).unwrap_or_default();
        let narrow_opens = open(&result, &scratch);
        eprintln!(
            "attempt with signals at {narrow} mm{}: {} open, {} starved, {} vias, {:.1} s",
            config.relaxed_clearance_mm.map(|relaxed| format!(" and clearance {relaxed} mm")).unwrap_or_default(),
            narrow_opens.0, narrow_opens.1, result.vias, result.routing_seconds
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
    if let (Some((opens, _)), Some((attempt, connect)), Some(relaxed)) =
        (best.as_ref(), best_attempt.as_ref(), config.relaxed_clearance_mm)
        && opens.0 > 0
        && !narrow_step_relaxed
        && spent(ladder_work) < 1.5 * ladder_budget
        && !past_deadline()
        && config.connection_rules.values().chain(config.default_rules.iter()).any(|rules| rules.clearance_mm > relaxed + 1.0e-9)
    {
        let mut relaxed_attempt = attempt.clone();
        relaxed_attempt.relaxed_clearance_mm = None;
        for rules in relaxed_attempt.connection_rules.values_mut().chain(relaxed_attempt.default_rules.iter_mut()) {
            rules.clearance_mm = rules.clearance_mm.min(relaxed);
        }
        if scratch.exists() {
            fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;
        }
        let mut result = route_kicad_board_once(source_directory, board_id, &scratch, &relaxed_attempt, *connect)?;
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
    let mut result = best.expect("at least one routing attempt").1;
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
fn route_seeds(board: &core::Board, config: &core::Config, first: u64, count: usize) -> core::RoutingResult {
    let results: Vec<(Option<u64>, core::RoutingResult)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (first..first + count.max(1) as u64)
            .map(|index| {
                let mut seeded = config.clone();
                seeded.seed = (index > 0).then_some(index);
                seeded.verbose = config.verbose && index == first;
                scope.spawn(move || (seeded.seed, core::route(board, &seeded)))
            })
            .collect();
        handles.into_iter().map(|handle| handle.join().expect("routing thread")).collect()
    });
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
        ..core::Config::default()
    };
    if let Some(via_cost) = config.via_cost_mm {
        router_config.via_cost = via_cost;
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
    let violations = core::verify(board, &result.routes);
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
    };
    let report_path = output_directory.join("board-router.json");
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", report_path.display()))?;
    Ok(report)
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
        // second spans two pads and keeps everything out; text over bare
        // board opens nothing that matters.
        let bare = openings(&board);
        assert_eq!(bare.len(), 2);
        assert!(bare[..1].iter().all(|(net, _)| net.is_some() && *net == net_a));
        assert!(bare[1..].iter().all(|(net, label)| net.is_none() && label.ends_with("(gr_rect)")));
        // Over a pour, the text exposes the fill: it is the pour's net's.
        let zone = r#"(zone (net "/B") (layer "F.Cu") (polygon (pts (xy 10 0) (xy 20 0) (xy 20 10) (xy 10 10))))"#;
        let poured = openings(&lowered(zone));
        assert_eq!(poured.len(), 3);
        assert_eq!(poured[2], (net_b, "solder mask opening (gr_text)".to_string()));
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
