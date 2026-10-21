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
