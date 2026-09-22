// Copyright (C) 2026 Toit contributors.

//! Whole-board component placement through the `pcb-placer` core.

use super::*;
use pcb_placer as core;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadBoardPlacerConfig {
    /// Minimum gap between two courtyards on colliding sides.
    pub spacing_mm: f64,
    /// Footprint origins are snapped to multiples of this.
    pub grid_mm: f64,
    pub keep_rotation: bool,
    /// References that must keep their pose, in addition to locked
    /// footprints, footprints without nets, and footprints on the board edge.
    pub fixed: Vec<String>,
    /// Glob patterns (`*` and `?`) of references that stay fixed.
    pub fixed_patterns: Vec<String>,
    /// Through-hole parts with at least four pins (connectors, headers)
    /// whose body comes within this distance of the outline stay where the
    /// designer put them: at the edge, where they are reachable.
    #[serde(default = "default_edge_keep_mm")]
    pub edge_keep_mm: f64,
    /// References that may move even though a default rule would fix them.
    pub free: Vec<String>,
    /// Share of the whitespace taken by filler charge: 1 packs the parts as
    /// tightly as halos allow, 0 spreads them over the whole board.
    pub whitespace_fill: f64,
    /// Every footprint keeps a halo of `pins / pins_per_halo_track` tracks
    /// (at `track_pitch_mm`, capped at `maximum_halo_mm`) free around its
    /// courtyard so its pins can escape.
    pub track_pitch_mm: f64,
    pub pins_per_halo_track: f64,
    pub maximum_halo_mm: f64,
    /// Per-reference halos replacing the pin-count rule (routing feedback).
    pub halo_overrides_mm: BTreeMap<String, f64>,
    /// Share of the free board that bodies, halos and spacing may claim.
    pub maximum_utilization: f64,
    /// Distance movable courtyards keep from the board edge (at least the
    /// copper-to-edge clearance, since pads may reach the courtyard).
    pub edge_margin_mm: f64,
    pub seed: u64,
}

impl Default for KiCadBoardPlacerConfig {
    fn default() -> Self {
        Self {
            spacing_mm: 0.5,
            grid_mm: 0.635,
            keep_rotation: false,
            fixed: Vec::new(),
            fixed_patterns: Vec::new(),
            edge_keep_mm: default_edge_keep_mm(),
            free: Vec::new(),
            whitespace_fill: 0.6,
            track_pitch_mm: 0.65,
            pins_per_halo_track: 8.0,
            maximum_halo_mm: 4.0,
            halo_overrides_mm: BTreeMap::new(),
            maximum_utilization: 0.6,
            edge_margin_mm: 0.5,
            seed: 1,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadPlacedFootprint {
    pub reference: String,
    pub fixed: bool,
    /// Halo requested for this footprint, before fitting to the board.
    pub halo_mm: f64,
    /// Final body centre and half extents on the board.
    pub body_center: [f64; 2],
    pub body_half: [f64; 2],
    pub source_at: [f64; 3],
    pub at: [f64; 3],
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadBoardPlacerResult {
    pub board_id: String,
    pub components: usize,
    pub movable: usize,
    pub nets: usize,
    pub global_iterations: usize,
    pub global_overflow: f64,
    pub wirelength_source_mm: f64,
    pub wirelength_global_mm: f64,
    pub wirelength_legal_mm: f64,
    pub wirelength_final_mm: f64,
    pub unplaced: Vec<String>,
    pub illegal: Vec<String>,
    pub seconds: f64,
    pub footprints: Vec<KiCadPlacedFootprint>,
}

fn placer_net(name: &str) -> bool {
    !name.is_empty()
        && !name.bytes().all(|byte| byte.is_ascii_digit())
        && !name.starts_with("unconnected-(")
}

fn normalize_angle(angle: f64) -> f64 {
    let wrapped = angle.rem_euclid(360.0);
    if wrapped.abs() < 1.0e-9 || (wrapped - 360.0).abs() < 1.0e-9 {
        0.0
    } else {
        wrapped
    }
}

/// Bounding box, in the footprint's own frame, of its courtyard and pads.
fn local_body(footprint: &Expr) -> Result<([f64; 2], [f64; 2], bool), String> {
    let mut minimum = [f64::INFINITY; 2];
    let mut maximum = [f64::NEG_INFINITY; 2];
    let mut include = |point: [f64; 2], radius: f64| {
        for axis in 0..2 {
            minimum[axis] = minimum[axis].min(point[axis] - radius);
            maximum[axis] = maximum[axis].max(point[axis] + radius);
        }
    };
    let footprint_angle = form_at(footprint)?[2];
    let mut circles = 0;
    let mut straight = 0;
    for child in footprint.children() {
        match child.head() {
            Some("fp_line" | "fp_rect" | "fp_arc")
                if form_atom(child, "layer", 1).is_some_and(|layer| layer.ends_with(".CrtYd")) =>
            {
                straight += 1;
                for head in ["start", "mid", "end"] {
                    if child.child(head).is_some() {
                        include(form_xy(child, head)?, 0.0);
                    }
                }
            }
            Some("fp_circle")
                if form_atom(child, "layer", 1).is_some_and(|layer| layer.ends_with(".CrtYd")) =>
            {
                circles += 1;
                let center = form_xy(child, "center")?;
                let end = form_xy(child, "end")?;
                include(center, distance_squared(center, end).sqrt());
            }
            Some("fp_poly")
                if form_atom(child, "layer", 1).is_some_and(|layer| layer.ends_with(".CrtYd")) =>
            {
                straight += 1;
                for point in child
                    .child("pts")
                    .map(Expr::children)
                    .unwrap_or_default()
                    .iter()
                    .filter(|point| point.head() == Some("xy"))
                {
                    include(
                        [
                            expression_coordinate(point, 1, "courtyard x")?,
                            expression_coordinate(point, 2, "courtyard y")?,
                        ],
                        0.0,
                    );
                }
            }
            Some("pad") => {
                let at = form_at(child)?;
                let size = form_xy(child, "size").unwrap_or([0.0, 0.0]);
                // Pad angles are absolute; bring the pad box into the local frame.
                let (sin, cos) = (-(at[2] - footprint_angle)).to_radians().sin_cos();
                // Copper must keep a hole's local clearance, so no other
                // part's pads may come that close either.
                let keep_away = if child.children().get(2).and_then(Expr::atom)
                    == Some("np_thru_hole")
                {
                    local_clearance::pad_clearance(child, footprint)?
                } else {
                    0.0
                };
                let half = [
                    (cos.abs() * size[0] + sin.abs() * size[1]) / 2.0 + keep_away,
                    (sin.abs() * size[0] + cos.abs() * size[1]) / 2.0 + keep_away,
                ];
                for corner in [[-1.0, -1.0], [1.0, 1.0]] {
                    include(
                        [at[0] + corner[0] * half[0], at[1] + corner[1] * half[1]],
                        0.0,
                    );
                }
            }
            _ => {}
        }
    }
    if !minimum[0].is_finite() {
        return Ok(([0.0, 0.0], [1.0, 1.0], false));
    }
    let size = [
        (maximum[0] - minimum[0]).max(0.2),
        (maximum[1] - minimum[1]).max(0.2),
    ];
    // A single circular courtyard containing all pads is a disc.
    let round = circles == 1 && straight == 0 && (size[0] - size[1]).abs() < 1.0e-6;
    Ok((
        [(minimum[0] + maximum[0]) / 2.0, (minimum[1] + maximum[1]) / 2.0],
        size,
        round,
    ))
}

/// Moves a footprint to `pose`, keeping pad and text angles (which KiCad
/// stores as absolute values) consistent. Returns the written pose.
pub(super) fn write_footprint_pose(footprint: &mut Expr, pose: core::Pose) -> Result<[f64; 3], String> {
    let current = form_at(footprint)?;
    let at = [
        (pose.position[0] * 1.0e6).round() / 1.0e6,
        (pose.position[1] * 1.0e6).round() / 1.0e6,
        normalize_angle(pose.angle),
    ];
    let delta = normalize_angle(at[2] - current[2]);
    set_form_at(footprint, at)?;
    if delta != 0.0
        && let Expr::List(children) = footprint
    {
        for child in children.iter_mut() {
            if matches!(child.head(), Some("pad" | "property" | "fp_text"))
                && child.child("at").is_some()
            {
                let child_at = form_at(child)?;
                set_form_at(
                    child,
                    [child_at[0], child_at[1], normalize_angle(child_at[2] + delta)],
                )?;
            }
        }
    }
    Ok(at)
}

pub(super) struct LoweredPlacement {
    pub problem: core::Problem,
    pub references: Vec<String>,
    pub source_at: Vec<[f64; 3]>,
}

fn default_edge_keep_mm() -> f64 {
    3.0
}

/// `*` matches any run of characters, `?` one character.
fn glob_matches(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let mut table = vec![vec![false; text.len() + 1]; pattern.len() + 1];
    table[0][0] = true;
    for p in 1..=pattern.len() {
        if pattern[p - 1] == '*' {
            table[p][0] = table[p - 1][0];
        }
        for t in 1..=text.len() {
            table[p][t] = match pattern[p - 1] {
                '*' => table[p - 1][t] || table[p][t - 1],
                '?' => table[p - 1][t - 1],
                c => table[p - 1][t - 1] && c == text[t - 1],
            };
        }
    }
    table[pattern.len()][text.len()]
}

pub(super) fn lower_placement(
    pcb: &Expr,
    config: &KiCadBoardPlacerConfig,
) -> Result<LoweredPlacement, String> {
    let loops = outline::board_loops(pcb)?;
    let mut net_ids = BTreeMap::<String, usize>::new();
    let mut components = Vec::new();
    let mut poses = Vec::new();
    let mut references = Vec::new();
    let mut source_at = Vec::new();
    let mut locked = Vec::new();
    let mut connector = Vec::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let at = form_at(footprint)?;
        let reference = footprint_reference(footprint).unwrap_or_default();
        let (body_center, body_size, round) = local_body(footprint)?;
        let mut pins = Vec::new();
        let mut through = false;
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            let pad_type = pad.children().get(2).and_then(Expr::atom).unwrap_or("");
            through |= matches!(pad_type, "thru_hole" | "np_thru_hole");
            let Some(net) = node_net(pad).map(normalize_net).filter(|net| placer_net(net)) else {
                continue;
            };
            let next = net_ids.len();
            let net = *net_ids.entry(net.to_string()).or_insert(next);
            let pad_at = form_at(pad)?;
            pins.push(core::Pin {
                offset: [pad_at[0], pad_at[1]],
                net,
            });
        }
        let side = if through {
            core::Side::Both
        } else if form_atom(footprint, "layer", 1) == Some("B.Cu") {
            core::Side::Back
        } else {
            core::Side::Front
        };
        let angle_options = if config.keep_rotation {
            vec![at[2]]
        } else {
            (0..4)
                .map(|quarter| normalize_angle(at[2] + 90.0 * quarter as f64))
                .collect()
        };
        connector.push(through && pins.len() >= 4);
        locked.push(
            footprint.child("locked").is_some()
                || footprint
                    .children()
                    .iter()
                    .any(|child| child.atom() == Some("locked")),
        );
        components.push(core::Component {
            name: reference.clone(),
            body_center,
            body_size,
            round,
            halo: if let Some(halo) = config.halo_overrides_mm.get(&reference) {
                *halo
            } else if config.pins_per_halo_track > 0.0 {
                ((pins.len() as f64 / config.pins_per_halo_track).ceil() * config.track_pitch_mm)
                    .min(config.maximum_halo_mm)
            } else {
                0.0
            },
            pins,
            side,
            fixed: false,
            angle_options,
        });
        poses.push(core::Pose {
            position: [at[0], at[1]],
            angle: at[2],
        });
        references.push(reference);
        source_at.push(at);
    }

    // Rule areas that forbid footprints are obstacles too, and a part the
    // designer placed reaching into one (an antenna module at its keepout)
    // is there on purpose and stays.
    let footprint_count = components.len();
    let mut keepouts: Vec<pcb_router::geometry::Aabb> = Vec::new();
    for item in pcb.children() {
        if item.head() != Some("zone")
            || !item.child("keepout").is_some_and(|keepout| {
                form_atom(keepout, "footprints", 1) == Some("not_allowed")
            })
        {
            continue;
        }
        let Some(polygon) = item.child("polygon").and_then(|polygon| polygon.child("pts")) else {
            continue;
        };
        let mut bounds: Option<pcb_router::geometry::Aabb> = None;
        for point in polygon.children().iter().filter(|child| child.head() == Some("xy")) {
            let coordinate = |index: usize| -> Result<f64, String> {
                point
                    .children()
                    .get(index)
                    .and_then(Expr::atom)
                    .ok_or_else(|| "keepout point without coordinates".to_string())?
                    .parse::<f64>()
                    .map_err(|error| format!("invalid keepout coordinate: {error}"))
            };
            let xy = [coordinate(1)?, coordinate(2)?];
            let corner = pcb_router::geometry::Aabb {
                minimum: xy,
                maximum: xy,
            };
            bounds = Some(bounds.map_or(corner, |bounds| bounds.union(corner)));
        }
        let Some(bounds) = bounds else {
            continue;
        };
        let layers: Vec<&str> = item
            .child("layers")
            .map(|layers| layers.children().iter().skip(1).filter_map(Expr::atom).collect())
            .or_else(|| form_atom(item, "layer", 1).map(|layer| vec![layer]))
            .unwrap_or_default();
        let side = match (layers.iter().any(|l| *l == "F.Cu"), layers.iter().any(|l| *l == "B.Cu")) {
            (true, false) => core::Side::Front,
            (false, true) => core::Side::Back,
            _ => core::Side::Both,
        };
        keepouts.push(bounds);
        components.push(core::Component {
            name: "keepout".into(),
            body_center: [0.0, 0.0],
            body_size: [
                bounds.maximum[0] - bounds.minimum[0],
                bounds.maximum[1] - bounds.minimum[1],
            ],
            round: false,
            halo: 0.0,
            pins: Vec::new(),
            side,
            fixed: true,
            angle_options: vec![0.0],
        });
        poses.push(core::Pose {
            position: [
                (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
                (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
            ],
            angle: 0.0,
        });
    }

    // Copper-layer text and graphics are part of the board: parts must not
    // be placed on top of them. They follow the footprints, so footprint
    // indices stay aligned with the board file.
    for item in pcb.children() {
        let Some(layer) = form_atom(item, "layer", 1).and_then(copper_layer_index) else {
            continue;
        };
        if !matches!(
            item.head(),
            Some("gr_text" | "gr_line" | "gr_rect" | "gr_arc" | "gr_circle" | "gr_poly")
        ) {
            continue;
        }
        let Some(bounds) = board_router::copper_graphic_shapes(item)?
            .iter()
            .map(pcb_router::Shape::aabb)
            .reduce(pcb_router::geometry::Aabb::union)
        else {
            continue;
        };
        components.push(core::Component {
            name: "copper graphic".into(),
            body_center: [0.0, 0.0],
            body_size: [
                bounds.maximum[0] - bounds.minimum[0],
                bounds.maximum[1] - bounds.minimum[1],
            ],
            round: false,
            halo: 0.0,
            pins: Vec::new(),
            side: if layer == 0 {
                core::Side::Front
            } else {
                core::Side::Back
            },
            fixed: true,
            angle_options: vec![0.0],
        });
        poses.push(core::Pose {
            position: [
                (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
                (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
            ],
            angle: 0.0,
        });
        references.push("copper graphic".into());
        source_at.push([0.0; 3]);
        locked.push(true);
    }

    let mut pin_counts = vec![0usize; net_ids.len()];
    for component in &components {
        for pin in &component.pins {
            pin_counts[pin.net] += 1;
        }
    }
    // Large nets (power) would otherwise dominate and collapse the layout.
    let net_weights = pin_counts
        .iter()
        .map(|pins| (3.0 / (*pins as f64 - 1.0).max(1.0)).min(1.0))
        .collect();
    let mut problem = core::Problem {
        outline: loops.outline.clone(),
        components,
        net_weights,
        poses,
        spacing: config.spacing_mm,
        grid: config.grid_mm,
        edge_margin: config.edge_margin_mm,
    };
    for index in 0..footprint_count {
        let reference = &references[index];
        let component = &problem.components[index];
        let pose = problem.poses[index];
        let (center, half) = (component.center(pose), component.half_extent(pose.angle));
        let reach = config.edge_keep_mm;
        let in_keepout = keepouts.iter().any(|keepout| {
            center[0] + half[0] + reach > keepout.minimum[0]
                && center[0] - half[0] - reach < keepout.maximum[0]
                && center[1] + half[1] + reach > keepout.minimum[1]
                && center[1] - half[1] - reach < keepout.maximum[1]
        });
        let on_edge = !core::legal::body_inside_outline(&problem, index, problem.poses[index])
            || in_keepout
            || (connector[index]
                && core::legal::body_near_outline(&problem, index, problem.poses[index], config.edge_keep_mm));
        let default_fixed = locked[index] || problem.components[index].pins.is_empty() || on_edge;
        problem.components[index].fixed = config.fixed.contains(reference)
            || config.fixed_patterns.iter().any(|pattern| glob_matches(pattern, reference))
            || (default_fixed && !config.free.contains(reference));
    }
    Ok(LoweredPlacement {
        problem,
        references,
        source_at,
    })
}

/// Places the movable footprints of `<source>/<board_id>.kicad_pcb`, removes
/// all routed copper, and writes the project plus an HTML playback to
/// `output`.
pub fn place_kicad_board(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardPlacerConfig,
) -> Result<KiCadBoardPlacerResult, String> {
    let started = std::time::Instant::now();
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    let source = fs::read_to_string(&source_board)
        .map_err(|error| format!("failed to read {}: {error}", source_board.display()))?;
    let mut pcb = parse(&source)?;
    let lowered = lower_placement(&pcb, config)?;
    let mut placer_config = core::Config::new();
    placer_config.global.whitespace_fill = config.whitespace_fill;
    placer_config.maximum_utilization = config.maximum_utilization;
    placer_config.global.seed = config.seed;
    let placement = core::place(&lowered.problem, &placer_config);

    let Expr::List(items) = &mut pcb else {
        return Err("PCB root is not a list".into());
    };
    items.retain(|item| !matches!(item.head(), Some("segment" | "arc" | "via")));
    let mut index = 0;
    let mut footprints = Vec::new();
    for footprint in items
        .iter_mut()
        .filter(|item| item.head() == Some("footprint"))
    {
        let pose = placement.poses[index];
        let source_at = lowered.source_at[index];
        let at = write_footprint_pose(footprint, pose)?;
        footprints.push(KiCadPlacedFootprint {
            reference: lowered.references[index].clone(),
            fixed: lowered.problem.components[index].fixed,
            halo_mm: lowered.problem.components[index].halo,
            body_center: lowered.problem.components[index].center(pose),
            body_half: lowered.problem.components[index].half_extent(pose.angle),
            source_at,
            at,
        });
        index += 1;
    }

    // The written board must put every pad where the placer believed it is.
    index = 0;
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let footprint_at = form_at(footprint)?;
        let component = &lowered.problem.components[index];
        let mut pin = 0;
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            if !node_net(pad).map(normalize_net).is_some_and(placer_net) {
                continue;
            }
            let written = lower_pad(pad, footprint_at)?.center;
            let expected = component.pin_position(&component.pins[pin], placement.poses[index]);
            if distance_squared(written, expected).sqrt() > 1.0e-5 {
                return Err(format!(
                    "internal error: pad of {} written at {written:?}, expected {expected:?}",
                    lowered.references[index]
                ));
            }
            pin += 1;
        }
        index += 1;
    }

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
    let html = core::view::playback_html(
        &lowered.problem,
        &placement.frames,
        &format!("{board_id} placement"),
    );
    let html_path = output_directory.join("placement.html");
    fs::write(&html_path, html)
        .map_err(|error| format!("failed to write {}: {error}", html_path.display()))?;

    let names = |indices: &[usize]| -> Vec<String> {
        indices
            .iter()
            .map(|index| lowered.references[*index].clone())
            .collect()
    };
    let result = KiCadBoardPlacerResult {
        board_id: board_id.into(),
        components: lowered.problem.components.len(),
        movable: lowered
            .problem
            .components
            .iter()
            .filter(|component| !component.fixed)
            .count(),
        nets: lowered.problem.net_weights.len(),
        global_iterations: placement.global_iterations,
        global_overflow: placement.global_overflow,
        wirelength_source_mm: placement.wirelength_initial,
        wirelength_global_mm: placement.wirelength_global,
        wirelength_legal_mm: placement.wirelength_legal,
        wirelength_final_mm: placement.wirelength_final,
        unplaced: names(&placement.unplaced),
        illegal: names(&placement.illegal),
        seconds: started.elapsed().as_secs_f64(),
        footprints,
    };
    let report_path = output_directory.join("board-placer.json");
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", report_path.display()))?;
    Ok(result)
}
