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
    #[serde(default)]
    pub grid_pitches_mm: Option<Vec<f64>>,
    /// Skip the final native KiCad verification (for timing the router).
    #[serde(default)]
    pub skip_native_verification: bool,
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
                (Some(trace_width_mm), Some(clearance_mm), Some(via_size_mm), Some(via_drill_mm)) => {
                    Some(KiCadConnectionRoutingRules {
                        trace_width_mm,
                        clearance_mm,
                        via_size_mm,
                        via_drill_mm,
                    })
                }
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
        serde_json::from_value(value).map_err(|error| format!("invalid board-router config: {error}"))
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

fn mask(layers: [bool; 2]) -> core::LayerMask {
    layers[0] as u32 | (layers[1] as u32) << 1
}

fn routable_net(name: &str) -> bool {
    !name.is_empty()
        && !name.bytes().all(|byte| byte.is_ascii_digit())
        && !name.starts_with("unconnected-(")
}

struct Lowered {
    board: core::Board,
}

fn lower(pcb: &Expr, config: &KiCadBoardRouterConfig) -> Result<Lowered, String> {
    let loops = outline::board_loops(pcb)?;
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
                                layers: 0b11,
                                kind: core::ObstacleKind::Hole,
                                net: None,
                                clearance: 0.0,
                                blocks_tracks: true,
                                blocks_vias: true,
                                label: format!("{label} hole"),
                            });
                        }
                    }
                    if !lowered.layers[0] && !lowered.layers[1] {
                        continue;
                    }
                    let net = node_net(pad)
                        .map(normalize_net)
                        .filter(|name| routable_net(name))
                        .map(|name| net_id(name, &mut nets, &mut classes))
                        .transpose()?;
                    obstacles.push(core::Obstacle {
                        shape: shape(&lowered.geometry),
                        layers: mask(lowered.layers),
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
                            layers: mask(lowered.layers),
                            pad: obstacles.len() - 1,
                            label,
                        });
                    }
                }
            }
            Some("segment") => {
                let Some(layer) = form_atom(item, "layer", 1).and_then(copper_layer_index) else {
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
                    layers: 0b11,
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
                        layers: mask(rule_area_layer_mask(item)?),
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
            layers: 0b11,
            kind: core::ObstacleKind::Hole,
            net: None,
            clearance: 0.0,
            blocks_tracks: true,
            blocks_vias: true,
            label: "board cutout".into(),
        });
    }
    if classes.is_empty() {
        return Err("the board has no routable connections".into());
    }
    Ok(Lowered {
        board: core::Board {
            layer_count: 2,
            outline: loops.outline.clone(),
            edge_clearance: config.edge_clearance_mm,
            hole_clearance: config.hole_clearance_mm,
            hole_to_hole: config.hole_to_hole_clearance_mm,
            classes,
            obstacles,
            nets,
        },
    })
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
    let started = std::time::Instant::now();
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    let source = fs::read_to_string(&source_board)
        .map_err(|error| format!("failed to read {}: {error}", source_board.display()))?;
    let mut pcb = parse(&source)?;
    let copper_net_names = unambiguous_pad_net_names(&pcb)?;
    let Lowered { board } = lower(&pcb, config)?;
    let lowering_seconds = started.elapsed().as_secs_f64();

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
    let routing_started = std::time::Instant::now();
    let result = core::route(&board, &router_config);
    let routing_seconds = routing_started.elapsed().as_secs_f64();

    let verification_started = std::time::Instant::now();
    let violations = core::verify(&board, &result.routes);
    let internal_verification_seconds = verification_started.elapsed().as_secs_f64();

    let Expr::List(items) = &mut pcb else {
        return Err("PCB root is not a list".into());
    };
    let mut nets = Vec::new();
    for (net, route) in result.routes.iter().enumerate() {
        let description = &board.nets[net];
        for segment in &route.segments {
            items.push(supplemental_segment_expr(&SupplementalSegment {
                connection: description.name.clone(),
                start: nanometres(segment.start),
                end: nanometres(segment.end),
                width: segment.width,
                layer: copper_layer_name(segment.layer).into(),
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
    canonicalize_copper_net_names(&mut pcb, &copper_net_names);

    if output_directory.exists() {
        return Err(format!(
            "output directory {} already exists",
            output_directory.display()
        ));
    }
    copy_directory_tree(source_directory, output_directory)?;
    let output_board = output_directory.join(format!("{board_id}.kicad_pcb"));
    fs::write(&output_board, format!("{}\n", encode(&pcb)))
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
        lowering_seconds,
        routing_seconds,
        internal_verification_seconds,
        native_verification_seconds,
        internal_violations: violations
            .iter()
            .map(|violation| KiCadBoardRouterViolation {
                connection: board.nets[violation.net as usize].name.clone(),
                other: violation.other.clone(),
                layer: copper_layer_name(violation.layer).into(),
                at: violation.at,
                required_mm: violation.required,
                actual_mm: violation.actual,
            })
            .collect(),
        native,
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
