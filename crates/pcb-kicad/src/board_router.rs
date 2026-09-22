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
    /// the previous attempt stays below this (default 240 s); no attempt of
    /// any kind follows one that took longer than twice this.
    #[serde(default)]
    pub refine_budget_seconds: Option<f64>,
    /// Skip the final native KiCad verification (for timing the router).
    #[serde(default)]
    pub skip_native_verification: bool,
    /// How nets with copper pours are connected.
    #[serde(default)]
    pub pours: KiCadPourMode,
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
                name if name.ends_with(".Cu") => {
                    mask |= 1
                        << self
                            .index(name)
                            .ok_or_else(|| format!("unknown copper layer {name:?}"))?;
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

fn routable_net(name: &str) -> bool {
    !name.is_empty()
        && !name.bytes().all(|byte| byte.is_ascii_digit())
        && !name.starts_with("unconnected-(")
}

pub(super) struct Lowered {
    pub board: core::Board,
}

pub(super) fn lower(
    pcb: &Expr,
    config: &KiCadBoardRouterConfig,
    connect_pours: bool,
) -> Result<Lowered, String> {
    let loops = outline::board_loops(pcb)?;
    let layers = LayerTable::from_pcb(pcb)?;
    let mut classes: Vec<core::RuleClass> = Vec::new();
    let mut nets: Vec<core::Net> = Vec::new();
    let mut net_ids = BTreeMap::<String, core::NetId>::new();
    let mut obstacles = Vec::new();

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
        let class = core::RuleClass {
            trace_width: rules.trace_width_mm,
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
                for pad in item
                    .children()
                    .iter()
                    .filter(|child| child.head() == Some("pad"))
                {
                    let pad_name = pad.children().get(1).and_then(Expr::atom).unwrap_or("");
                    let label = format!("{reference}.{}", pad_name.trim_matches('"'));
                    let lowered = lower_pad(pad, footprint_at)?;
                    let pad_type = pad.children().get(2).and_then(Expr::atom).unwrap_or("");
                    // Plated holes of pads without a net are just holes to
                    // everyone; the board's hole clearance applies.
                    if pad_type == "thru_hole"
                        && !node_net(pad).map(normalize_net).is_some_and(routable_net)
                        && let Some(drill) = pad.child("drill")
                    {
                        let values: Vec<f64> = drill
                            .children()
                            .iter()
                            .skip(1)
                            .filter_map(Expr::atom)
                            .filter_map(|value| value.parse::<f64>().ok())
                            .collect();
                        if let Some(&diameter) = values.first() {
                            let (sin, cos) = (-form_at(pad)?[2]).to_radians().sin_cos();
                            let shape = match values.get(1) {
                                Some(&height) if (height - diameter).abs() > 1.0e-9 => {
                                    let (long, short) =
                                        (diameter.max(height), diameter.min(height));
                                    let axis = if diameter >= height {
                                        [1.0, 0.0]
                                    } else {
                                        [0.0, 1.0]
                                    };
                                    let half = (long - short) / 2.0;
                                    let offset = [
                                        half * (axis[0] * cos - axis[1] * sin),
                                        half * (axis[0] * sin + axis[1] * cos),
                                    ];
                                    core::Shape::Capsule {
                                        start: [
                                            lowered.center[0] - offset[0],
                                            lowered.center[1] - offset[1],
                                        ],
                                        end: [
                                            lowered.center[0] + offset[0],
                                            lowered.center[1] + offset[1],
                                        ],
                                        radius: short / 2.0,
                                    }
                                }
                                _ => core::Shape::Circle {
                                    center: lowered.center,
                                    radius: diameter / 2.0,
                                },
                            };
                            obstacles.push(core::Obstacle {
                                shape,
                                layers: layers.all(),
                                kind: core::ObstacleKind::Hole,
                                net: None,
                                clearance: local_clearance::pad_clearance(pad, item)?,
                                blocks_tracks: true,
                                blocks_vias: true,
                                label: format!("{label} plated hole"),
                            });
                        }
                    }
                    if pad_type == "np_thru_hole"
                        && let Some(drill) = pad.child("drill")
                    {
                        let diameter = drill
                            .children()
                            .iter()
                            .skip(1)
                            .filter_map(Expr::atom)
                            .filter_map(|value| value.parse::<f64>().ok())
                            .fold(0.0_f64, f64::max);
                        if diameter > 0.0 {
                            obstacles.push(core::Obstacle {
                                shape: core::Shape::Circle {
                                    center: lowered.center,
                                    radius: diameter / 2.0,
                                },
                                layers: layers.all(),
                                kind: core::ObstacleKind::Hole,
                                net: None,
                                clearance: local_clearance::pad_clearance(pad, item)?,
                                blocks_tracks: true,
                                blocks_vias: true,
                                label: format!("{label} hole"),
                            });
                        }
                    }
                    let pad_layers = layers.mask_of_item(pad)?;
                    if pad_layers == 0 {
                        continue;
                    }
                    let net = node_net(pad)
                        .map(normalize_net)
                        .filter(|name| routable_net(name))
                        .map(|name| net_id(name, &mut nets, &mut classes))
                        .transpose()?;
                    obstacles.push(core::Obstacle {
                        shape: shape(&lowered.geometry),
                        layers: pad_layers,
                        kind: core::ObstacleKind::Copper,
                        net,
                        clearance: local_clearance::pad_clearance(pad, item)?,
                        blocks_tracks: true,
                        blocks_vias: true,
                        label: label.clone(),
                    });
                    if let Some(net) = net {
                        nets[net as usize].terminals.push(core::Terminal {
                            anchor: lowered.center,
                            layers: pad_layers,
                            pad: obstacles.len() - 1,
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
                    .map(normalize_net)
                    .filter(|name| routable_net(name))
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
                    blocks_tracks: true,
                    blocks_vias: true,
                    label: "existing track".into(),
                });
            }
            Some("via") => {
                let net = node_net(item)
                    .map(normalize_net)
                    .filter(|name| routable_net(name))
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
                    blocks_tracks: true,
                    blocks_vias: true,
                    label: "existing via".into(),
                });
            }
            Some("arc") => {
                return Err("the board router does not yet lower existing arc tracks".into());
            }
            Some("gr_text" | "gr_line" | "gr_rect" | "gr_arc" | "gr_circle" | "gr_poly")
                if form_atom(item, "layer", 1)
                    .and_then(|name| layers.index(name))
                    .is_some() =>
            {
                let layer = form_atom(item, "layer", 1)
                    .and_then(|name| layers.index(name))
                    .unwrap();
                for shape in copper_graphic_shapes(item)? {
                    obstacles.push(core::Obstacle {
                        shape,
                        layers: 1 << layer,
                        kind: core::ObstacleKind::Copper,
                        net: None,
                        clearance: 0.0,
                        blocks_tracks: true,
                        blocks_vias: true,
                        label: format!("copper {}", item.head().unwrap_or("graphic")),
                    });
                }
            }
            Some("zone") if is_rule_area(item) => {
                let allowed = |kind: &str| {
                    item.child("keepout")
                        .and_then(|keepout| keepout.child(kind))
                        .and_then(|form| form.children().get(1))
                        .and_then(Expr::atom)
                        != Some("not_allowed")
                };
                let (blocks_tracks, blocks_vias) = (!allowed("tracks"), !allowed("vias"));
                if blocks_tracks || blocks_vias {
                    obstacles.push(core::Obstacle {
                        shape: core::Shape::Polygon {
                            points: rule_area_polygon_points(item)?,
                        },
                        layers: layers.mask_of_item(item)?,
                        kind: core::ObstacleKind::Keepout,
                        net: None,
                        clearance: 0.0,
                        blocks_tracks,
                        blocks_vias,
                        label: form_atom(item, "name", 1)
                            .unwrap_or("unnamed rule area")
                            .to_string(),
                    });
                }
            }
            _ => {}
        }
    }
    for cutout in &loops.cutouts {
        obstacles.push(core::Obstacle {
            shape: core::Shape::Polygon {
                points: cutout.clone(),
            },
            layers: layers.all(),
            kind: core::ObstacleKind::Hole,
            net: None,
            clearance: 0.0,
            blocks_tracks: true,
            blocks_vias: true,
            label: "board cutout".into(),
        });
    }
    let pours = pours(pcb, &layers)?;
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
            edge_clearance: config.edge_clearance_mm,
            hole_clearance: config.hole_clearance_mm,
            hole_to_hole: config.hole_to_hole_clearance_mm,
            classes,
            obstacles,
            nets,
            planes,
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
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    fs::write(destination, format!("{}\n", encode(&pcb)))
        .map_err(|error| format!("failed to write {}: {error}", destination.display()))
}

/// Copper-layer graphics as obstacle shapes. Text becomes its (generous)
/// bounding box.
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
        Some("gr_line") => vec![capsule(form_xy(item, "start")?, form_xy(item, "end")?)],
        Some("gr_arc") => outline::arc_points(
            form_xy(item, "start")?,
            form_xy(item, "mid")?,
            form_xy(item, "end")?,
        )
        .windows(2)
        .map(|pair| capsule(pair[0], pair[1]))
        .collect(),
        Some("gr_rect") => {
            let (a, b) = (form_xy(item, "start")?, form_xy(item, "end")?);
            let corners = [a, [b[0], a[1]], b, [a[0], b[1]]];
            let mut shapes: Vec<_> = (0..4)
                .map(|index| capsule(corners[index], corners[(index + 1) % 4]))
                .collect();
            if !matches!(form_atom(item, "fill", 1), None | Some("none" | "no")) {
                shapes.push(core::Shape::Polygon {
                    points: corners.to_vec(),
                });
            }
            shapes
        }
        Some("gr_circle") => {
            let center = form_xy(item, "center")?;
            let radius = distance_squared(center, form_xy(item, "end")?).sqrt();
            vec![core::Shape::Circle {
                center,
                radius: radius + width / 2.0,
            }]
        }
        Some("gr_poly") => vec![core::Shape::Polygon {
            points: rule_area_like_points(item)?,
        }],
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
            let lines = text.split("\\n").count().max(1) as f64;
            let longest = text
                .split("\\n")
                .map(|line| line.chars().count())
                .max()
                .unwrap_or(0) as f64;
            let half = [
                longest * size[1] * 0.55 + thickness,
                lines * size[0] * 0.85 + thickness,
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
            let offset = rotate_vector([shift, 0.0], -at[2]);
            vec![core::Shape::rectangle(
                [at[0] + offset[0], at[1] + offset[1]],
                half,
                -at[2],
            )]
        }
        _ => Vec::new(),
    })
}

fn rule_area_like_points(item: &Expr) -> Result<Vec<[f64; 2]>, String> {
    let mut points = Vec::new();
    for point in item
        .child("pts")
        .map(Expr::children)
        .unwrap_or_default()
        .iter()
        .filter(|point| point.head() == Some("xy"))
    {
        points.push([
            expression_coordinate(point, 1, "polygon x")?,
            expression_coordinate(point, 2, "polygon y")?,
        ]);
    }
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
            .map(normalize_net)
            .filter(|net| routable_net(net))
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
    let modes: Vec<(bool, bool)> = match (has_pours, config.pours) {
        (false, _) | (true, KiCadPourMode::Tracks) => vec![(false, false)],
        (true, KiCadPourMode::Connect) => vec![(true, skeleton)],
        (true, KiCadPourMode::Auto) if skeleton => vec![(true, true), (false, false)],
        (true, KiCadPourMode::Auto) if two_layers => vec![(true, false), (true, true), (false, false)],
        (true, KiCadPourMode::Auto) => vec![(true, false), (false, false)],
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
    'ladder: for pitch in &pitches {
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
        for (connect, skeleton) in &modes {
            // The skeleton only helps when the plain pour connection left
            // pads of a pour net open; elsewhere it just takes room.
            if *skeleton
                && !config.plane_skeleton.unwrap_or(false)
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
            let opens = open(&result, &directory);
            eprintln!(
                "attempt pours={} pitch={}: {} open, {} starved, {} vias, {:.1} s",
                result.pours,
                pitch
                    .as_ref()
                    .map_or("regular".to_string(), |pitch| format!("{:?}", pitch)),
                opens.0,
                opens.1,
                result.vias,
                result.routing_seconds
            );
            let seconds = result.routing_seconds;
            let used = (result.routing_seconds, result.grid_pitch_mm);
            if slowest.is_none_or(|(seconds, _)| used.0 > seconds) {
                slowest = Some(used);
            }
            let better = best
                .as_ref()
                .is_none_or(|(best_opens, _)| opens < *best_opens);
            if better {
                if directory != output_directory {
                    fs::remove_dir_all(output_directory).map_err(|error| error.to_string())?;
                    fs::rename(&directory, output_directory).map_err(|error| error.to_string())?;
                }
                best = Some((opens, result));
            } else if directory.exists() {
                fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
            }
            // Another way of connecting the pours is worth trying up to
            // twice the budget; a board that slow gets no more attempts.
            if opens.0 == 0 || seconds > 2.0 * budget {
                break 'ladder;
            }
        }
    }
    let result = best.expect("at least one routing attempt").1;
    let report_path = output_directory.join("board-router.json");
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", report_path.display()))?;
    Ok(result)
}

/// The core router configuration for a KiCad configuration.
pub(super) fn core_config(config: &KiCadBoardRouterConfig) -> core::Config {
    let mut router_config = core::Config {
        verbose: true,
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
fn starved_thermals(directory: &Path) -> usize {
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
    let nets = emit_routes(&mut pcb, &board, &result, &layer_names)?;
    finish_routed_board(
        &pcb,
        &board,
        &result,
        nets,
        &layer_names,
        source_directory,
        board_id,
        output_directory,
        config,
        [lowering_seconds, routing_seconds],
    )
}
