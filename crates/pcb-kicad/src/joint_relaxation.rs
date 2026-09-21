// Copyright (C) 2026 Toit contributors.

//! Bounded, opt-in joint movement of simple F.Cu chains. No routing policy.
use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadJointRelaxationConfig {
    /// Explicit native-compiled rules; imported candidate defaults are ignored.
    pub routing: KiCadGridRouteConfig,
    pub movable_connections: Vec<String>,
    pub solver: KiCadJointSolverConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadJointSolverConfig {
    pub steps: usize,
    pub timestep: f64,
    pub projection_iterations: usize,
    pub trace_tension_strength: f64,
    pub maximum_trace_tension_step_mm: f64,
    pub constraint_guard_mm: f64,
    pub maximum_point_motion_mm: f64,
    pub subdivision_length_mm: f64,
    pub maximum_constraint_pairs: usize,
    pub maximum_points: usize,
    pub maximum_movable_connections: usize,
    pub render_every_steps: usize,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum KiCadJointRelaxationStatus {
    Running,
    Unsupported,
    Rejected,
    Admitted,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadJointRelaxationResult {
    pub schema_version: u32,
    pub status: KiCadJointRelaxationStatus,
    pub reason: Option<String>,
    pub source_board_sha256: String,
    pub target_candidate_sha256: String,
    pub config: KiCadJointRelaxationConfig,
    /// Published atomically with joint-relaxation.json, only after admission.
    pub selected_directory: PathBuf,
    pub baseline_native: VerificationReport,
    pub final_native: Option<VerificationReport>,
    pub candidate_directory: Option<PathBuf>,
    pub frame_directories: Vec<PathBuf>,
    pub frames: usize,
    pub segment_pairs: usize,
    pub segment_body_pairs: usize,
    pub maximum_point_motion_mm: f64,
    pub per_net_pad_connectivity: BTreeMap<String, bool>,
    pub fixed_design_preserved: bool,
    pub source_unchanged: bool,
    pub contract: String,
}

#[derive(Clone)]
struct Chain {
    candidate: KiCadRouteCandidate,
    points: Vec<[f64; 2]>,
    width: f64,
    clearance: f64,
    model: KiCadRoutingModel,
}

fn validate_solver(s: &KiCadJointSolverConfig) -> Result<(), String> {
    if s.steps == 0
        || s.projection_iterations == 0
        || s.maximum_points < 2
        || s.maximum_constraint_pairs == 0
        || s.render_every_steps == 0
        || [
            s.timestep,
            s.maximum_trace_tension_step_mm,
            s.maximum_point_motion_mm,
            s.subdivision_length_mm,
        ]
        .iter()
        .any(|v| !v.is_finite() || *v <= 0.0)
        || [s.trace_tension_strength, s.constraint_guard_mm]
            .iter()
            .any(|v| !v.is_finite() || *v < 0.0)
    {
        return Err("joint relaxation requires finite geometry and positive bounded work".into());
    }
    Ok(())
}

fn validate_selection(config: &KiCadJointRelaxationConfig, target: &str) -> Result<(), String> {
    if target.is_empty() || normalize_net(target) != target {
        return Err("joint target identity must be canonical and nonempty".into());
    }
    let mut identities = BTreeSet::from([target.to_string()]);
    for net in &config.movable_connections {
        if net.is_empty() || normalize_net(net) != net || !identities.insert(net.clone()) {
            return Err(
                "joint identities must be canonical, distinct, and exclude the target".into(),
            );
        }
    }
    for net in identities {
        if !config.routing.connection_rules.contains_key(&net) {
            return Err(format!("explicit joint rules omit selected net {net:?}"));
        }
    }
    Ok(())
}

fn point_key(at: [f64; 2]) -> (u64, u64) {
    let bits = |v: f64| {
        if v == 0.0 {
            0.0_f64.to_bits()
        } else {
            v.to_bits()
        }
    };
    (bits(at[0]), bits(at[1]))
}

/// Materialized copper is authoritative; branch annotations cannot hide stubs.
fn simple_chain(candidate: &KiCadRouteCandidate) -> Result<(Vec<[f64; 2]>, f64), String> {
    if !candidate.supplemental_vias.is_empty()
        || !candidate.footprint_placements.is_empty()
        || !candidate.reference_placements.is_empty()
        || candidate.supplemental_segments.is_empty()
    {
        return Err("joint movement supports via-free chains without placement changes".into());
    }
    let width = candidate.supplemental_segments[0].width;
    let mut points = BTreeMap::new();
    let mut graph = BTreeMap::<_, Vec<_>>::new();
    let mut edges = BTreeSet::new();
    for segment in &candidate.supplemental_segments {
        if segment.connection != candidate.connection
            || segment.layer != "F.Cu"
            || !segment.width.is_finite()
            || segment.width <= 0.0
            || segment.width != width
            || segment
                .start
                .iter()
                .chain(segment.end.iter())
                .any(|v| !v.is_finite())
            || segment.start == segment.end
        {
            return Err(
                "joint movement requires finite uniform-width F.Cu segments of one net".into(),
            );
        }
        let a = point_key(segment.start);
        let b = point_key(segment.end);
        if !edges.insert(if a < b { (a, b) } else { (b, a) }) {
            return Err("duplicate/overlapping chain edges are unsupported".into());
        }
        points.insert(a, segment.start);
        points.insert(b, segment.end);
        graph.entry(a).or_default().push(b);
        graph.entry(b).or_default().push(a);
    }
    let ends: Vec<_> = graph
        .iter()
        .filter(|(_, v)| v.len() == 1)
        .map(|(k, _)| *k)
        .collect();
    if ends.len() != 2 || graph.values().any(|v| v.len() > 2) {
        return Err("branches, loops and non-chain junctions are unsupported".into());
    }
    let mut previous = None;
    let mut current = ends[0];
    let mut visited = BTreeSet::new();
    let mut ordered = Vec::new();
    loop {
        if !visited.insert(current) {
            return Err("cyclic chain is unsupported".into());
        }
        ordered.push(points[&current]);
        let next = graph[&current]
            .iter()
            .find(|&&x| Some(x) != previous)
            .copied();
        let Some(next) = next else {
            break;
        };
        previous = Some(current);
        current = next;
    }
    if visited.len() != points.len() {
        return Err("disconnected copper/stubs are unsupported".into());
    }
    for triple in ordered.windows(3) {
        let u = [triple[1][0] - triple[0][0], triple[1][1] - triple[0][1]];
        let v = [triple[2][0] - triple[1][0], triple[2][1] - triple[1][1]];
        let dot = u[0] * v[0] + u[1] * v[1];
        let cross = u[0] * v[1] - u[1] * v[0];
        if dot < 0.0 && cross.abs() <= 1.0e-12 * u[0].hypot(u[1]) * v[0].hypot(v[1]) {
            return Err("adjacent collinear backtracking is unsupported".into());
        }
    }
    for i in 0..ordered.len() - 1 {
        for j in i + 2..ordered.len() - 1 {
            if segment_segment_distance_squared(
                ordered[i],
                ordered[i + 1],
                ordered[j],
                ordered[j + 1],
            ) < 1.0e-18
            {
                return Err("nonlocal centerline contact is unsupported".into());
            }
        }
    }
    Ok((ordered, width))
}

/// Electrical attachment is overlap of the actual emitted round endpoint cap
/// with a distinct front-copper pad, not containment of the centerline endpoint.
/// Imported nanometre coordinates may round outside a pad without disconnecting
/// its copper. No coordinate snapping or additional contact tolerance is used.
fn pad_connected(model: &KiCadRoutingModel, points: &[[f64; 2]], width: f64) -> bool {
    if model.terminal_pads.len() != 2
        || !width.is_finite()
        || width <= 0.0
        || points.len() < 2
        || model.terminal_pads.iter().any(|p| !p.layers[0])
    {
        return false;
    }
    let a = points[0];
    let b = points[points.len() - 1];
    let p = &model.terminal_pads;
    (p[0].geometry.contains(a, width / 2.0)
        && p[1].geometry.contains(b, width / 2.0)
        && !p[1].geometry.contains(a, width / 2.0)
        && !p[0].geometry.contains(b, width / 2.0))
        || (p[1].geometry.contains(a, width / 2.0)
            && p[0].geometry.contains(b, width / 2.0)
            && !p[0].geometry.contains(a, width / 2.0)
            && !p[1].geometry.contains(b, width / 2.0))
}

fn subdivide(points: &[[f64; 2]], spacing: f64, maximum: usize) -> Result<Vec<[f64; 2]>, String> {
    let mut output = vec![points[0]];
    for pair in points.windows(2) {
        let pieces = (distance_squared(pair[0], pair[1]).sqrt() / spacing)
            .ceil()
            .max(1.0) as usize;
        if pieces > maximum.saturating_sub(output.len()) {
            return Err("joint point allowance exceeded".into());
        }
        for i in 1..=pieces {
            output.push(if i == pieces {
                pair[1]
            } else {
                let t = i as f64 / pieces as f64;
                [
                    pair[0][0] + t * (pair[1][0] - pair[0][0]),
                    pair[0][1] + t * (pair[1][1] - pair[0][1]),
                ]
            });
        }
    }
    Ok(output)
}

/// Rectangular boards retain the existing solver-boundary behavior. For other
/// outlines, the full capsule swept by each initial segment must already clear
/// every exact boundary edge. If both endpoints remain within the admitted
/// displacement bound, every interpolated point of the moving segment remains
/// inside this capsule. The real outline and final native checks are unchanged.
fn outline_allows_motion(
    outline: &BoardOutline,
    points: &[[f64; 2]],
    width: f64,
    edge_clearance: f64,
    s: &KiCadJointSolverConfig,
) -> bool {
    let b = outline.bounds;
    if outline.points.len() == 4
        && outline
            .points
            .iter()
            .all(|p| (p[0] == b[0] || p[0] == b[2]) && (p[1] == b[1] || p[1] == b[3]))
    {
        return true;
    }
    let margin = width / 2.0 + edge_clearance + s.constraint_guard_mm + s.maximum_point_motion_mm;
    let scale = points
        .iter()
        .flatten()
        .chain(outline.points.iter().flatten())
        .fold(margin.max(1.0), |scale, coordinate| {
            scale.max(coordinate.abs())
        });
    let guard = 16.0 * f64::from(f32::EPSILON) * scale;
    points.len() >= 2
        && margin.is_finite()
        && margin >= 0.0
        && points
            .windows(2)
            .all(|pair| outline.contains_segment_with_clearance(pair[0], pair[1], margin + guard))
}

fn prepare_chain(
    pcb: &Expr,
    mut candidate: KiCadRouteCandidate,
    config: &KiCadJointRelaxationConfig,
    remaining_points: usize,
) -> Result<Chain, String> {
    if candidate.supplemental_segments.len() >= remaining_points {
        return Err(
            "joint materialized point allowance exceeded before geometry preparation".into(),
        );
    }
    let (points, width) = simple_chain(&candidate)?;
    let points = subdivide(
        &points,
        config.solver.subdivision_length_mm,
        remaining_points,
    )?;
    let report = route_obstructions::inspect(pcb, &candidate, &config.routing)?;
    if matches!(report.status, KiCadRouteObstructionStatus::Unsupported) {
        return Err(report
            .unsupported_reason
            .unwrap_or("unsupported board geometry".into()));
    }
    let mut rules = connection_rules::resolve(&config.routing, &candidate.connection)?;
    if width + 1.0e-9 < rules.trace_width_mm {
        return Err("materialized width is below supplied routing width".into());
    }
    // Preserve actual copper width even where a retained trace exceeds preference.
    rules.trace_width_mm = width;
    let model = KiCadRoutingModel::from_pcb_with_config(pcb, &candidate.connection, &rules)?;
    if !pad_connected(&model, &points, width) {
        return Err("simple chain must join exactly two distinct pads with front copper".into());
    }
    if !outline_allows_motion(
        &model.outline,
        &points,
        width,
        rules.edge_clearance_mm,
        &config.solver,
    ) {
        return Err(
            "joint motion envelope is not certified inside the nonrectangular outline".into(),
        );
    }
    candidate.config = rules;
    Ok(Chain {
        clearance: candidate.config.clearance_mm,
        candidate,
        points,
        width,
        model,
    })
}

/// Reject oversized selected copper before the importer or quadratic graph work.
fn input_segment_count(pcb: &Expr, net: &str) -> usize {
    let names = partial_ripup::net_names(pcb);
    pcb.children()
        .iter()
        .filter(|item| {
            item.head() == Some("segment")
                && node_net(item).is_some_and(|raw| {
                    normalize_net(names.get(raw).map_or(raw, String::as_str)) == net
                })
        })
        .count()
}

fn preflight_input_points(
    pcb: &Expr,
    target: &KiCadRouteCandidate,
    config: &KiCadJointRelaxationConfig,
) -> Result<(), String> {
    let mut used = target
        .supplemental_segments
        .len()
        .checked_add(1)
        .ok_or("joint point count overflow")?;
    if used > config.solver.maximum_points {
        return Err("joint total materialized point allowance exceeded".into());
    }
    for net in &config.movable_connections {
        let count = input_segment_count(pcb, net);
        used = used
            .checked_add(count)
            .and_then(|n| n.checked_add(1))
            .ok_or("joint point count overflow")?;
        if used > config.solver.maximum_points {
            return Err("joint total materialized point allowance exceeded".into());
        }
    }
    Ok(())
}

/// Every point of a segment stays within the initial segment's motion envelope
/// when both endpoints respect the displacement bound. Keep touching envelopes
/// and a floating-point guard; final admission still checks the complete board
/// and rejects any observed displacement beyond the declared bound.
fn obstacle_within_motion_envelope(
    chain: &Chain,
    obstacle: &CopperObstacle,
    s: &KiCadJointSolverConfig,
) -> bool {
    let obstacle_bounds = obstacle.geometry.aabb();
    let margin = chain.width / 2.0
        + obstacle.clearance(chain.clearance)
        + s.constraint_guard_mm
        + s.maximum_point_motion_mm;
    let scale = chain
        .points
        .iter()
        .flatten()
        .chain(obstacle_bounds.minimum.iter())
        .chain(obstacle_bounds.maximum.iter())
        .fold(margin.max(1.0), |scale, coordinate| {
            scale.max(coordinate.abs())
        });
    let guard = 16.0 * f64::from(f32::EPSILON) * scale;
    chain.points.windows(2).any(|pair| {
        GeometryAabb::around_segment(pair[0], pair[1], margin + guard).intersects(obstacle_bounds)
    })
}

fn request(chains: &[Chain], s: &KiCadJointSolverConfig) -> Result<CopperRepairRequest, String> {
    if chains.iter().map(|c| c.points.len()).sum::<usize>() > s.maximum_points {
        return Err("joint total point allowance exceeded".into());
    }
    let selected: BTreeSet<_> = chains
        .iter()
        .map(|c| c.candidate.connection.as_str())
        .collect();
    let mut polylines = Vec::new();
    let mut bodies = Vec::new();
    let mut separations = Vec::new();
    let mut body_separations = Vec::new();
    for (i, chain) in chains.iter().enumerate() {
        polylines.push(CopperPolylineInput {
            id: format!("net-{i}"),
            points: chain
                .points
                .iter()
                .map(|p| engine_vec2(*p, "joint vertex"))
                .collect::<Result<_, _>>()?,
            width: engine_f32(chain.width, "joint width")?,
            tension_weight: 1.0,
            mobility: CopperPolylineMobility::Interior,
        });
        for (j, other) in chains.iter().enumerate().take(i) {
            separations.push(CopperSeparationPair {
                first: format!("net-{i}"),
                second: format!("net-{j}"),
                clearance: engine_f32(
                    chain.clearance.max(other.clearance) + s.constraint_guard_mm,
                    "joint clearance",
                )?,
            });
        }
        for (k, obstacle) in chain.model.obstacles.iter().enumerate() {
            if !obstacle.layers[0] || !obstacle.blocks_tracks {
                continue;
            }
            if obstacle.kind == KiCadViaLocalRerouteBlockerKind::ForeignNetCopper
                && obstacle
                    .net
                    .as_deref()
                    .is_some_and(|n| selected.contains(normalize_net(n)))
            {
                continue;
            }
            if !obstacle_within_motion_envelope(chain, obstacle, s) {
                continue;
            }
            let id = format!("fixed-{i}-{k}");
            let clearance = engine_f32(
                obstacle.clearance(chain.clearance) + s.constraint_guard_mm,
                "fixed clearance",
            )?;
            let shape = match &obstacle.geometry {
                ObstacleGeometry::Circle { center, radius } => Some((*center, *center, *radius)),
                ObstacleGeometry::Segment { start, end, radius } => Some((*start, *end, *radius)),
                ObstacleGeometry::Rectangle {
                    center,
                    half_size,
                    angle_degrees,
                } => {
                    bodies.push(CopperBodyInput {
                        id: id.clone(),
                        position: engine_vec2(*center, "fixed center")?,
                        angle_radians: engine_f32(angle_degrees.to_radians(), "fixed angle")?,
                        size: engine_vec2([half_size[0] * 2.0, half_size[1] * 2.0], "fixed size")?,
                        mobility: Mobility::FIXED,
                    });
                    body_separations.push(CopperSegmentBodyPair {
                        polyline: format!("net-{i}"),
                        body: id.clone(),
                        clearance,
                    });
                    None
                }
                _ => {
                    return Err(
                        "joint solver supports only circular/capsule/rectangular fixed obstacles"
                            .into(),
                    );
                }
            };
            if let Some((a, b, r)) = shape {
                polylines.push(CopperPolylineInput {
                    id: id.clone(),
                    points: vec![engine_vec2(a, "fixed start")?, engine_vec2(b, "fixed end")?],
                    width: engine_f32(r * 2.0, "fixed width")?,
                    tension_weight: 0.0,
                    mobility: CopperPolylineMobility::Fixed,
                });
                separations.push(CopperSeparationPair {
                    first: format!("net-{i}"),
                    second: id,
                    clearance,
                });
            }
        }
    }
    if polylines.iter().map(|p| p.points.len()).sum::<usize>() > s.maximum_points {
        return Err("joint movable plus fixed point allowance exceeded".into());
    }
    let bounds = chains[0].model.bounds;
    let inset = chains
        .iter()
        .map(|c| c.width / 2.0 + c.candidate.config.edge_clearance_mm + s.constraint_guard_mm)
        .fold(0.0, f64::max);
    Ok(CopperRepairRequest {
        bounds: Bounds::new(
            engine_vec2([bounds[0] + inset, bounds[1] + inset], "joint minimum")?,
            engine_vec2([bounds[2] - inset, bounds[3] - inset], "joint maximum")?,
        ),
        bodies,
        polylines,
        separations,
        body_separations,
        shared_points: Vec::new(),
        attachments: Vec::new(),
        body_body_separations: Vec::new(),
        maximum_segment_pairs: s.maximum_constraint_pairs,
    })
}

fn candidates(
    chains: &[Chain],
    solved: &BTreeMap<String, Vec<Vec2>>,
) -> Result<Vec<KiCadRouteCandidate>, String> {
    chains
        .iter()
        .enumerate()
        .map(|(i, chain)| {
            let vertices = solved
                .get(&format!("net-{i}"))
                .ok_or("engine omitted joint net")?;
            if vertices.len() != chain.points.len() {
                return Err("engine changed joint chain point count".into());
            }
            let mut points: Vec<[f64; 2]> = vertices
                .iter()
                .map(|p| [f64::from(p.x), f64::from(p.y)])
                .collect();
            // Retain original fixed endpoints rather than roundtrip them through f32.
            points[0] = chain.points[0];
            *points.last_mut().unwrap() = *chain.points.last().unwrap();
            if points.iter().flatten().any(|v| !v.is_finite()) {
                return Err("engine produced nonfinite joint geometry".into());
            }
            let mut candidate = chain.candidate.clone();
            candidate.router = "joint-simple-front-chain-projected-tension-v1".into();
            candidate.terminals = vec![points[0], *points.last().unwrap()];
            candidate.branches = vec![KiCadRouteBranch {
                start_terminal: points[0],
                finish_terminal: *points.last().unwrap(),
                cost: 0,
                expansions: 0,
                path: points
                    .iter()
                    .map(|at| KiCadRoutePoint {
                        at: *at,
                        layer: "F.Cu".into(),
                    })
                    .collect(),
            }];
            candidate.supplemental_segments = points
                .windows(2)
                .filter(|p| p[0] != p[1])
                .map(|p| SupplementalSegment {
                    connection: candidate.connection.clone(),
                    start: p[0],
                    end: p[1],
                    width: chain.width,
                    layer: "F.Cu".into(),
                })
                .collect();
            candidate.supplemental_vias.clear();
            Ok(candidate)
        })
        .collect()
}

fn write_board(
    source: &Path,
    board_id: &str,
    directory: &Path,
    candidates: &[KiCadRouteCandidate],
) -> Result<(), String> {
    copy_directory_tree(source, directory)?;
    // A provisional frame must not inherit verification of different copper.
    for name in ["drc.json", "erc.json", "verification.json", "preview.json"] {
        remove_if_present(&directory.join(name))?;
    }
    let board = directory.join(format!("{board_id}.kicad_pcb"));
    for (i, candidate) in candidates.iter().enumerate() {
        let path = directory.join(format!("candidate-{i:02}.json"));
        write_typed_json(&path, candidate)?;
        apply_route_candidate(&board, &path, &board)?;
    }
    // Cheap combined copper SVG, using the same renderer as native verification.
    verification_preview::render(&board, directory)
}

fn publish(output: &Path, result: &KiCadJointRelaxationResult) -> Result<(), String> {
    let temporary = output.join(".joint-relaxation.next.json");
    write_typed_json(&temporary, result)?;
    fs::rename(&temporary, output.join("joint-relaxation.json")).map_err(|e| e.to_string())
}

fn selection_admissible(
    result: &KiCadJointRelaxationResult,
    geometry_clear: bool,
    native_ok: bool,
    maximum_motion: f64,
) -> bool {
    geometry_clear
        && native_ok
        && result.fixed_design_preserved
        && result.source_unchanged
        && !result.per_net_pad_connectivity.is_empty()
        && result.per_net_pad_connectivity.values().all(|v| *v)
        && result.maximum_point_motion_mm.is_finite()
        && result.maximum_point_motion_mm <= maximum_motion + 1.0e-6
}

/// Input remains immutable. A complete selection manifest is the commit point;
/// its selected directory always names a fully materialized retained board.
pub fn relax_kicad_joint_routes(
    source_directory: &Path,
    board_id: &str,
    target_candidate_path: &Path,
    config: &KiCadJointRelaxationConfig,
    output: &Path,
) -> Result<KiCadJointRelaxationResult, String> {
    if !matches!(
        Path::new(board_id)
            .components()
            .collect::<Vec<_>>()
            .as_slice(),
        [std::path::Component::Normal(_)]
    ) {
        return Err("joint board-id must be a single filename stem".into());
    }
    validate_solver(&config.solver)?;
    validate_grid_route_config(&config.routing)?;
    if config.routing.connection_rules.is_empty() {
        return Err("joint adapter requires explicit per-net routing rules".into());
    }
    if config.movable_connections.len() > config.solver.maximum_movable_connections {
        return Err("joint neighbor allowance exceeded".into());
    }
    if output.exists() {
        return Err("joint output must be a fresh directory".into());
    }
    let source_directory = fs::canonicalize(source_directory).map_err(|e| e.to_string())?;
    let absolute_output = env::current_dir().map_err(|e| e.to_string())?.join(output);
    let output_name = absolute_output
        .file_name()
        .ok_or("joint output needs a directory name")?;
    let output_parent = fs::canonicalize(absolute_output.parent().unwrap_or(Path::new(".")))
        .map_err(|e| e.to_string())?;
    if output_parent.starts_with(&source_directory) {
        return Err("joint output cannot be inside its source".into());
    }
    let absolute_output = output_parent.join(output_name);
    let output = absolute_output.as_path();
    let target = read_route_candidate(target_candidate_path)?;
    validate_selection(config, &target.connection)?;
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    let source_hash = file_sha256(&source_board)?;
    let source_erc = kicad_erc_input_sha256(&source_directory)?;
    let pcb = parse(&fs::read_to_string(&source_board).map_err(|e| e.to_string())?)?;
    fs::create_dir_all(output).map_err(|e| e.to_string())?;
    let baseline = output.join("baseline");
    copy_directory_tree(&source_directory, &baseline)?;
    let baseline_native = verify_materialized_rung(&baseline, board_id)?;
    let baseline_drc = read_json(&baseline.join("drc.json"))?;
    let mut result = KiCadJointRelaxationResult { schema_version:1,status:KiCadJointRelaxationStatus::Running,reason:None,
        source_board_sha256:source_hash.clone(),target_candidate_sha256:file_sha256(target_candidate_path)?,config:config.clone(),
        selected_directory:baseline.clone(),baseline_native:baseline_native.clone(),final_native:None,candidate_directory:None,
        frame_directories:Vec::new(),frames:0,segment_pairs:0,segment_body_pairs:0,maximum_point_motion_mm:0.0,
        per_net_pad_connectivity:BTreeMap::new(),fixed_design_preserved:false,source_unchanged:true,
        contract:"One bounded joint solve of explicit simple front-only chains; emitted widths and native-compiled clearances; exact fixed endpoints/poses/rules; no via/layer changes, no automatic neighbors, no deferred restoration; final joint geometry and per-net pad connectivity plus native admission before atomic selection; baseline remains available on failure/interruption.".into() };
    publish(output, &result)?;
    if baseline_native.drc_design_violations != 0
        || baseline_native.erc_violations != 0
        || baseline_native.schematic_parity_issues != 0
    {
        result.status = KiCadJointRelaxationStatus::Rejected;
        result.reason = Some("source DRC/ERC/parity preflight failed; this first slice also rejects annotation findings".into());
        publish(output, &result)?;
        return Ok(result);
    }
    let expected_opens =
        partial_ripup::expected_opens(&source_board, &target.connection, &baseline_native)?;
    let prepared = (|| {
        preflight_input_points(&pcb, &target, config)?;
        let mut remaining = config.solver.maximum_points;
        let first = prepare_chain(&pcb, target, config, remaining)?;
        remaining -= first.points.len();
        let mut chains = vec![first];
        for net in &config.movable_connections {
            if input_segment_count(&pcb, net) >= remaining {
                return Err(
                    "joint remaining materialized point allowance exceeded before import".into(),
                );
            }
            let chain = prepare_chain(
                &pcb,
                import_route_candidate_from_pcb(&source_board, net)?,
                config,
                remaining,
            )?;
            remaining -= chain.points.len();
            chains.push(chain);
        }
        let request = request(&chains, &config.solver)?;
        let compiled = compile_copper_repair(&request)?;
        Ok::<_, String>((chains, compiled))
    })();
    let (chains, mut compiled) = match prepared {
        Ok(value) => value,
        Err(reason) => {
            result.status = KiCadJointRelaxationStatus::Unsupported;
            result.reason = Some(reason);
            publish(output, &result)?;
            return Ok(result);
        }
    };
    result.segment_pairs = compiled.segment_pairs;
    result.segment_body_pairs = compiled.segment_body_pairs;
    let s = &config.solver;
    let solver = SolverConfig {
        timestep: engine_f32(s.timestep, "joint timestep")?,
        projection_iterations: s.projection_iterations,
        field_width: 8,
        field_height: 8,
        field_smoothing_steps: 0,
        target_density: 0.0,
        field_strength: 0.0,
        maximum_field_step: 0.2,
        trace_tension_strength: engine_f32(s.trace_tension_strength, "joint tension")?,
        maximum_trace_tension_step: engine_f32(
            s.maximum_trace_tension_step_mm,
            "joint maximum tension step",
        )?,
    };
    let mut backend = CpuReferenceBackend::new();
    let mut frame_metrics = Vec::new();
    for step in 0..=s.steps {
        if step > 0 {
            let frame = backend.step(&mut compiled.world, &solver, (step - 1) as u64);
            result.frames += 1;
            frame_metrics.push(serde_json::json!({"step":step,"maximum_constraint_residual_mm":frame.metrics.max_constraint_residual,
                "maximum_frame_displacement_mm":frame.metrics.max_displacement,"constraint_projections":frame.metrics.constraint_projections}));
        }
        let solved: BTreeMap<_, _> = compiled.polylines().into_iter().collect();
        let proposals = candidates(&chains, &solved)?;
        for (chain, proposal) in chains.iter().zip(&proposals) {
            for (a, b) in chain.points.iter().zip(&proposal.branches[0].path) {
                result.maximum_point_motion_mm = result
                    .maximum_point_motion_mm
                    .max(distance_squared(*a, b.at).sqrt());
            }
        }
        if step % s.render_every_steps == 0 || step == s.steps {
            let directory = output.join(format!("frame-{step:04}"));
            write_board(&baseline, board_id, &directory, &proposals)?;
            result.frame_directories.push(directory);
            write_typed_json(&output.join("solver-frames.json"), &frame_metrics)?;
            publish(output, &result)?;
        }
    }
    let solved: BTreeMap<_, _> = compiled.polylines().into_iter().collect();
    let proposals = candidates(&chains, &solved)?;
    let final_directory = output.join("candidate");
    write_board(&baseline, board_id, &final_directory, &proposals)?;
    result.candidate_directory = Some(final_directory.clone());
    let final_board = final_directory.join(format!("{board_id}.kicad_pcb"));
    let final_pcb = parse(&fs::read_to_string(&final_board).map_err(|e| e.to_string())?)?;
    let mut geometry_clear = true;
    for (i, (chain, candidate)) in chains.iter().zip(&proposals).enumerate() {
        let connected = simple_chain(candidate).is_ok_and(|(points, width)| {
            width == chain.width
                && pad_connected(&chain.model, &points, width)
                && points.first() == chain.points.first()
                && points.last() == chain.points.last()
        });
        result
            .per_net_pad_connectivity
            .insert(candidate.connection.clone(), connected);
        let report = route_obstructions::inspect(&final_pcb, candidate, &config.routing)?;
        geometry_clear &= matches!(report.status, KiCadRouteObstructionStatus::Analyzed)
            && report.contacts.is_empty();
        write_typed_json(
            &final_directory.join(format!("geometry-{i:02}.json")),
            &report,
        )?;
    }
    let changed: Vec<_> = chains
        .iter()
        .map(|c| c.candidate.connection.as_str())
        .collect();
    result.fixed_design_preserved =
        partial_ripup::preserved_connections(&baseline, &final_directory, board_id, &changed)?;
    let native = verify_materialized_rung(&final_directory, board_id)?;
    let native_ok = partial_ripup::final_admissible(
        &native,
        &read_json(&final_directory.join("drc.json"))?,
        &baseline_drc,
        expected_opens,
        false,
    )?;
    result.final_native = Some(native);
    result.source_unchanged = file_sha256(&source_board)? == source_hash
        && kicad_erc_input_sha256(&source_directory)? == source_erc;
    if selection_admissible(
        &result,
        geometry_clear,
        native_ok,
        s.maximum_point_motion_mm,
    ) {
        result.status = KiCadJointRelaxationStatus::Admitted;
        result.selected_directory = final_directory;
    } else {
        result.status = KiCadJointRelaxationStatus::Rejected;
        result.reason = Some(format!(
            "joint admission failed: geometry={geometry_clear}, native={native_ok}, fixed={}, source={}, pad_connectivity={:?}, motion={:.6}/{:.6}mm",
            result.fixed_design_preserved,
            result.source_unchanged,
            result.per_net_pad_connectivity,
            result.maximum_point_motion_mm,
            s.maximum_point_motion_mm
        ));
    }
    publish(output, &result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(net: &str, points: &[[f64; 2]], width: f64) -> KiCadRouteCandidate {
        serde_json::from_value(serde_json::json!({
            "schema_version":2,"connection":net,"router":"fixture","config":{},
            "terminals":[points[0],points[points.len()-1]],"grid_origin":[0,0],"grid_size":[1,1],
            "cost":0,"expansions":0,"branches":[],"supplemental_vias":[],
            "supplemental_segments":points.windows(2).map(|p|serde_json::json!({
                "connection":net,"start":p[0],"end":p[1],"width":width,"layer":"F.Cu"
            })).collect::<Vec<_>>()
        }))
        .unwrap()
    }

    fn preset() -> KiCadJointSolverConfig {
        KiCadJointSolverConfig {
            steps: 64,
            timestep: 0.05,
            projection_iterations: 8,
            trace_tension_strength: 4.0,
            maximum_trace_tension_step_mm: 0.1,
            constraint_guard_mm: 0.005,
            maximum_point_motion_mm: 4.0,
            subdivision_length_mm: 1.0,
            maximum_constraint_pairs: 20_000,
            maximum_points: 256,
            maximum_movable_connections: 2,
            render_every_steps: 16,
        }
    }

    fn model(points: &[[f64; 2]]) -> KiCadRoutingModel {
        KiCadRoutingModel {
            bounds: [-1.0, -1.0, 6.0, 6.0],
            outline: BoardOutline {
                bounds: [-1.0, -1.0, 6.0, 6.0],
                points: vec![[-1.0, -1.0], [6.0, -1.0], [6.0, 6.0], [-1.0, 6.0]],
            },
            terminals: vec![points[0], *points.last().unwrap()],
            target_holes: Vec::new(),
            obstacles: Vec::new(),
            terminal_pads: [points[0], *points.last().unwrap()]
                .into_iter()
                .enumerate()
                .map(|(i, at)| KiCadTerminalPad {
                    at,
                    footprint: format!("X{i}"),
                    local_position: [0.0, 0.0],
                    layers: [true, false],
                    plated_through_hole: false,
                    geometry: ObstacleGeometry::Circle {
                        center: at,
                        radius: 0.3,
                    },
                })
                .collect(),
        }
    }

    fn chain(net: &str, points: &[[f64; 2]], width: f64, clearance: f64) -> Chain {
        Chain {
            candidate: candidate(net, points, width),
            points: points.to_vec(),
            width,
            clearance,
            model: model(points),
        }
    }

    fn fixed_circle(at: [f64; 2]) -> CopperObstacle {
        CopperObstacle {
            source_uuid: None,
            layers: [true, false],
            blocks_tracks: true,
            blocks_vias: true,
            local_clearance_mm: 0.0,
            geometry: ObstacleGeometry::Circle {
                center: at,
                radius: 0.1,
            },
            kind: KiCadViaLocalRerouteBlockerKind::FootprintPad,
            object: "fixed test pad".into(),
            net: Some("FIXED".into()),
            footprint: Some("FIXED".into()),
            component_movable: false,
        }
    }

    #[test]
    fn remote_context_does_not_consume_local_solver_point_allowance() {
        let mut c = chain("A", &[[0.0, 0.0], [2.0, 0.0], [4.0, 0.0]], 0.2, 0.25);
        for i in 0..160 {
            c.model
                .obstacles
                .push(fixed_circle([100.0 + i as f64, 100.0]));
        }
        let local = request(std::slice::from_ref(&c), &preset()).unwrap();
        assert_eq!(local.polylines.len(), 1);
        assert!(compile_copper_repair(&local).is_ok());
        // Local density still consumes the exact same work allowance.
        for obstacle in &mut c.model.obstacles {
            obstacle.geometry = fixed_circle([2.0, 2.0]).geometry;
        }
        assert!(
            request(&[c], &preset())
                .unwrap_err()
                .contains("point allowance")
        );
    }

    #[test]
    fn motion_envelope_keeps_future_contacts_and_local_clearance() {
        let c = chain("A", &[[0.0, 0.0], [2.0, 0.0], [4.0, 0.0]], 0.2, 0.25);
        let mut obstacle = fixed_circle([2.0, 4.4]);
        assert!(
            !obstacle
                .geometry
                .intersects_segment([0.0, 0.0], [4.0, 0.0], 0.35)
        );
        assert!(obstacle_within_motion_envelope(&c, &obstacle, &preset()));
        obstacle.geometry = fixed_circle([2.0, 4.6]).geometry;
        assert!(!obstacle_within_motion_envelope(&c, &obstacle, &preset()));
        obstacle.local_clearance_mm = 0.7;
        assert!(obstacle_within_motion_envelope(&c, &obstacle, &preset()));
    }

    #[test]
    fn excluded_obstacles_remain_clear_for_bounded_endpoint_moves() {
        let c = chain("A", &[[0.0, 0.0], [4.0, 1.0]], 0.4, 0.25);
        let s = preset();
        for x in [-6.0, 0.0, 4.0, 10.0] {
            for y in [-6.0, 0.0, 6.0] {
                for geometry in [
                    ObstacleGeometry::Circle {
                        center: [x, y],
                        radius: 0.5,
                    },
                    ObstacleGeometry::Segment {
                        start: [x, y],
                        end: [x + 1.0, y + 1.0],
                        radius: 0.3,
                    },
                    ObstacleGeometry::Rectangle {
                        center: [x, y],
                        half_size: [0.5, 1.0],
                        angle_degrees: 37.0,
                    },
                ] {
                    let mut obstacle = fixed_circle([x, y]);
                    obstacle.geometry = geometry;
                    if obstacle_within_motion_envelope(&c, &obstacle, &s) {
                        continue;
                    }
                    // Independently move both ends throughout the allowed disk;
                    // a remote obstacle must never enter the swept segment.
                    for da in [
                        [0.0, 0.0],
                        [4.0, 0.0],
                        [-4.0, 0.0],
                        [0.0, 4.0],
                        [0.0, -4.0],
                        [2.8, 2.8],
                    ] {
                        for db in [
                            [0.0, 0.0],
                            [4.0, 0.0],
                            [-4.0, 0.0],
                            [0.0, 4.0],
                            [0.0, -4.0],
                            [-2.8, -2.8],
                        ] {
                            assert!(!obstacle.geometry.intersects_segment(
                                [c.points[0][0] + da[0], c.points[0][1] + da[1]],
                                [c.points[1][0] + db[0], c.points[1][1] + db[1]],
                                c.width / 2.0
                                    + obstacle.clearance(c.clearance)
                                    + s.constraint_guard_mm,
                            ));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn materialized_graph_rejects_hidden_spurs_mixed_width_and_layers() {
        let c = candidate("A", &[[0.0, 0.0], [2.0, 0.0], [4.0, 0.0]], 0.3);
        let mut shuffled = c.clone();
        shuffled.supplemental_segments.reverse();
        assert_eq!(
            simple_chain(&shuffled).unwrap(),
            (vec![[0.0, 0.0], [2.0, 0.0], [4.0, 0.0]], 0.3)
        );
        let mut spur = c.clone();
        spur.supplemental_segments.push(SupplementalSegment {
            connection: "A".into(),
            start: [2.0, 0.0],
            end: [2.0, 1.0],
            width: 0.3,
            layer: "F.Cu".into(),
        });
        assert!(simple_chain(&spur).is_err());
        let mut disconnected = c.clone();
        disconnected
            .supplemental_segments
            .push(SupplementalSegment {
                connection: "A".into(),
                start: [2.0, 2.0],
                end: [3.0, 2.0],
                width: 0.3,
                layer: "F.Cu".into(),
            });
        assert!(simple_chain(&disconnected).is_err());
        let mut mixed = c.clone();
        mixed.supplemental_segments[0].width = 0.4;
        assert!(simple_chain(&mixed).is_err());
        mixed = c.clone();
        mixed.supplemental_segments[0].layer = "B.Cu".into();
        assert!(simple_chain(&mixed).is_err());
        mixed = c;
        mixed.supplemental_segments[0].start[0] = f64::NAN;
        assert!(simple_chain(&mixed).is_err());
    }

    #[test]
    fn adjacent_backtracking_is_rejected_in_both_orders_and_signs() {
        for points in [
            vec![[0.0, 0.0], [2.0, 0.0], [1.0, 0.0]],
            vec![[0.0, 0.0], [-2.0, 0.0], [-1.0, 0.0]],
            vec![[0.0, 0.0], [2.0, 2.0], [1.0, 1.0]],
        ] {
            assert!(
                simple_chain(&candidate("A", &points, 0.2))
                    .unwrap_err()
                    .contains("backtracking")
            );
            let reversed: Vec<_> = points.iter().rev().copied().collect();
            let mut c = candidate("A", &reversed, 0.2);
            c.supplemental_segments.reverse();
            assert!(simple_chain(&c).unwrap_err().contains("backtracking"));
        }
        for points in [
            vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]],
            vec![[2.0, 0.0], [1.0, 0.0], [0.0, 0.0]],
            vec![[0.0, 0.0], [2.0, 0.0], [1.0, 1.0]],
        ] {
            assert!(simple_chain(&candidate("A", &points, 0.2)).is_ok());
        }
    }

    #[test]
    fn materialized_and_subdivided_work_is_bounded_before_geometry() {
        let invalid_board = parse("(not_a_board)").unwrap();
        let config = KiCadJointRelaxationConfig {
            routing: KiCadGridRouteConfig::default(),
            movable_connections: vec![],
            solver: preset(),
        };
        let too_many = candidate("TARGET", &[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]], 0.2);
        let error = prepare_chain(&invalid_board, too_many, &config, 2)
            .err()
            .unwrap();
        assert!(error.contains("materialized point allowance"), "{error}");
        let too_long = candidate("TARGET", &[[0.0, 0.0], [100.0, 0.0]], 0.2);
        let error = prepare_chain(&invalid_board, too_long, &config, 4)
            .err()
            .unwrap();
        assert!(error.contains("point allowance"), "{error}");
        let board =
            parse(r#"(kicad_pcb (net 1 "/A") (segment (net 1)) (segment (net 1)))"#).unwrap();
        let mut config = config;
        config.movable_connections = vec!["A".into()];
        config.solver.maximum_points = 4;
        let target = candidate("TARGET", &[[0.0, 0.0], [1.0, 0.0]], 0.2);
        assert!(
            preflight_input_points(&board, &target, &config).is_err(),
            "2 target + 3 neighbor points exceed total4"
        );
        config.solver.maximum_points = 5;
        assert!(preflight_input_points(&board, &target, &config).is_ok());
    }

    #[test]
    #[ignore = "requires the explicitly selected retained native fixture directory"]
    fn retained_native_proposals_pass_tightened_guards_without_rerouting() {
        let root = PathBuf::from(
            env::var("PCB_MAKER_JOINT_RETAINED_FIXTURE_ROOT").expect("set retained fixture root"),
        );
        let inputs = read_json(&root.join("fixture-inputs.json")).unwrap();
        for (name, spec) in inputs["cases"].as_object().unwrap() {
            let directory = root.join(name);
            let config: KiCadJointRelaxationConfig =
                serde_json::from_value(read_json(&directory.join("config.json")).unwrap()).unwrap();
            let source = PathBuf::from(spec["source"].as_str().unwrap());
            let board = source.join(format!("{name}.kicad_pcb"));
            assert_eq!(
                file_sha256(&board).unwrap(),
                spec["native_source_sha256"].as_str().unwrap()
            );
            let pcb = parse(&fs::read_to_string(&board).unwrap()).unwrap();
            let target = read_route_candidate(&directory.join("target-candidate.json")).unwrap();
            preflight_input_points(&pcb, &target, &config).unwrap();
            let mut remaining = config.solver.maximum_points;
            let mut originals = vec![target];
            originals.extend(
                config
                    .movable_connections
                    .iter()
                    .map(|net| import_route_candidate_from_pcb(&board, net).unwrap()),
            );
            let chains: Vec<_> = originals
                .into_iter()
                .map(|c| {
                    let chain = prepare_chain(&pcb, c, &config, remaining).unwrap();
                    remaining -= chain.points.len();
                    chain
                })
                .collect();
            request(&chains, &config.solver).unwrap();
            let attempt = directory.join("attempt");
            let receipt = read_json(&attempt.join("joint-relaxation.json")).unwrap();
            assert_eq!(receipt["status"], "admitted");
            let candidate_directory =
                PathBuf::from(receipt["selected_directory"].as_str().unwrap());
            let candidate_board = candidate_directory.join(format!("{name}.kicad_pcb"));
            let before = file_sha256(&candidate_board).unwrap();
            let final_pcb = parse(&fs::read_to_string(&candidate_board).unwrap()).unwrap();
            for (i, chain) in chains.iter().enumerate() {
                let c = read_route_candidate(
                    &candidate_directory.join(format!("candidate-{i:02}.json")),
                )
                .unwrap();
                let (points, width) = simple_chain(&c).unwrap();
                assert_eq!(width, chain.width);
                assert!(pad_connected(&chain.model, &points, width));
                let query = route_obstructions::inspect(&final_pcb, &c, &config.routing).unwrap();
                assert!(matches!(
                    query.status,
                    KiCadRouteObstructionStatus::Analyzed
                ));
                assert!(query.contacts.is_empty());
            }
            let changed: Vec<_> = chains
                .iter()
                .map(|c| c.candidate.connection.as_str())
                .collect();
            assert!(
                partial_ripup::preserved_connections(
                    &attempt.join("baseline"),
                    &candidate_directory,
                    name,
                    &changed
                )
                .unwrap()
            );
            let native: VerificationReport = serde_json::from_value(
                read_json(&candidate_directory.join("verification.json")).unwrap(),
            )
            .unwrap();
            assert!(native.complete);
            assert_eq!(file_sha256(&candidate_board).unwrap(), before);
            println!(
                "retained {name}: tightened input/chain/geometry/preservation guards pass; PCB {before} unchanged"
            );
        }
    }

    #[test]
    fn nonrectangular_outline_certifies_whole_motion_capsules() {
        let outline = BoardOutline::new(vec![
            [0.0, 0.0],
            [20.0, 0.0],
            [20.0, 20.0],
            [12.0, 20.0],
            [12.0, 12.0],
            [8.0, 12.0],
            [8.0, 20.0],
            [0.0, 20.0],
        ])
        .unwrap();
        let points = [[5.0, 5.0], [15.0, 5.0]];
        assert!(outline_allows_motion(
            &outline,
            &points,
            0.2,
            0.25,
            &preset()
        ));
        assert!(!outline_allows_motion(
            &outline,
            &points,
            2.0,
            0.25,
            &preset()
        ));
        assert!(!outline_allows_motion(
            &outline,
            &points,
            0.2,
            1.0,
            &preset()
        ));
        let mut larger = preset();
        larger.maximum_point_motion_mm = 5.0;
        assert!(!outline_allows_motion(
            &outline, &points, 0.2, 0.25, &larger
        ));
        // Centerline is inside, but permitted motion reaches the concavity.
        assert!(!outline_allows_motion(
            &outline,
            &[[5.0, 10.0], [15.0, 10.0]],
            0.2,
            0.25,
            &preset()
        ));
        // Inside endpoints cannot certify a segment that crosses the notch.
        let crossing = [[5.0, 16.0], [15.0, 16.0]];
        assert!(
            crossing
                .iter()
                .all(|p| outline.contains_point_with_clearance(*p, 0.0))
        );
        assert!(!outline_allows_motion(
            &outline,
            &crossing,
            0.2,
            0.25,
            &preset()
        ));
        // Require slack beyond the nominal capsule radius for f32 conversion.
        assert!(!outline_allows_motion(
            &outline,
            &[[5.0, 4.355], [15.0, 4.355]],
            0.2,
            0.25,
            &preset()
        ));
    }

    #[test]
    fn outline_certificate_keeps_rectangular_behavior_and_exact_slanted_edges() {
        let rectangle = BoardOutline::rectangle([0.0, 0.0, 20.0, 20.0]).unwrap();
        assert!(outline_allows_motion(
            &rectangle,
            &[[0.5, 0.5], [19.5, 0.5]],
            0.2,
            0.25,
            &preset()
        ));
        let rotated =
            BoardOutline::new(vec![[0.0, 10.0], [10.0, 0.0], [20.0, 10.0], [10.0, 20.0]]).unwrap();
        assert!(outline_allows_motion(
            &rotated,
            &[[8.0, 10.0], [12.0, 10.0]],
            0.2,
            0.25,
            &preset()
        ));
        assert!(!outline_allows_motion(
            &rotated,
            &[[1.0, 10.0], [3.0, 10.0]],
            0.2,
            0.25,
            &preset()
        ));
    }

    #[test]
    fn electrical_endpoints_require_distinct_front_pad_contacts() {
        let points = [[0.0, 0.0], [4.0, 0.0]];
        let mut m = model(&points);
        assert!(pad_connected(&m, &points, 0.2));
        m.terminal_pads[1].layers = [false, true];
        assert!(!pad_connected(&m, &points, 0.2));
        m = model(&points);
        m.terminal_pads[1].plated_through_hole = true;
        m.terminal_pads[1].layers = [true, true];
        assert!(pad_connected(&m, &points, 0.2));
        m.terminal_pads[0].plated_through_hole = true;
        m.terminal_pads[0].layers = [true, true];
        assert!(pad_connected(&m, &points, 0.2));
        m.terminal_pads[1].layers = [false, true];
        assert!(!pad_connected(&m, &points, 0.2));
        m = model(&points);
        m.terminal_pads.push(m.terminal_pads[0].clone());
        assert!(!pad_connected(&m, &points, 0.2));
        assert!(!pad_connected(
            &model(&points),
            &[[0.0, 0.0], [3.0, 0.0]],
            0.2
        ));
    }

    #[test]
    fn endpoint_caps_require_strict_copper_overlap_and_unambiguous_pads() {
        let m = model(&[[0.0, 0.0], [4.0, 0.0]]);
        let outside_centers = [[0.35, 0.0], [3.65, 0.0]];
        assert!(
            !m.terminal_pads[0]
                .geometry
                .contains(outside_centers[0], 0.0)
        );
        assert!(pad_connected(&m, &outside_centers, 0.2));
        // Coordinate-rounding scale outside a pad remains real copper overlap.
        assert!(pad_connected(&m, &[[0.300000162, 0.0], [4.0, 0.0]], 0.2));
        assert!(!pad_connected(
            &m,
            &[[0.300000162, 0.0], [4.0, 0.0]],
            1.0e-8
        ));
        assert!(!pad_connected(&m, &[[0.41, 0.0], [4.0, 0.0]], 0.2));
        assert!(!pad_connected(&m, &[[0.4, 0.0], [4.0, 0.0]], 0.2));
        assert!(!pad_connected(&m, &[[0.0, 0.0], [4.0, 0.0]], 8.0));
        assert!(!pad_connected(&m, &outside_centers, 0.0));
        assert!(!pad_connected(&m, &outside_centers, f64::NAN));
        let mut corner = m;
        corner.terminal_pads[0].geometry = ObstacleGeometry::Rectangle {
            center: [0.0, 0.0],
            half_size: [0.3, 0.3],
            angle_degrees: 0.0,
        };
        assert!(pad_connected(&corner, &[[0.37, 0.37], [4.0, 0.0]], 0.2));
        // Rounded endpoint caps must not use a square-expanded rectangle.
        assert!(!pad_connected(&corner, &[[0.38, 0.38], [4.0, 0.0]], 0.2));
    }

    #[test]
    fn joint_pair_clearances_preserve_widths_and_foreign_pads() {
        let mut a = chain("A", &[[0.0, 0.0], [2.0, 0.0], [4.0, 0.0]], 0.2, 0.25);
        let b = chain("B", &[[0.0, 2.0], [2.0, 2.0], [4.0, 2.0]], 0.4, 0.35);
        let mut obstacle = CopperObstacle {
            source_uuid: Some("fixed-pad".into()),
            layers: [true, false],
            blocks_tracks: true,
            blocks_vias: true,
            local_clearance_mm: 0.7,
            geometry: ObstacleGeometry::Circle {
                center: [2.0, 4.0],
                radius: 0.3,
            },
            kind: KiCadViaLocalRerouteBlockerKind::FootprintPad,
            object: "X".into(),
            net: Some("B".into()),
            footprint: Some("X".into()),
            component_movable: false,
        };
        a.model.obstacles.push(obstacle.clone());
        obstacle.kind = KiCadViaLocalRerouteBlockerKind::ForeignNetCopper;
        obstacle.source_uuid = Some("moving-copper".into());
        a.model.obstacles.push(obstacle);
        let r = request(&[a, b], &preset()).unwrap();
        assert_eq!(
            r.polylines
                .iter()
                .filter(|p| p.mobility == CopperPolylineMobility::Interior)
                .map(|p| p.width)
                .collect::<Vec<_>>(),
            vec![0.2, 0.4]
        );
        assert_eq!(
            r.polylines.len(),
            3,
            "foreign selected copper is movable, its pad remains fixed"
        );
        assert!(
            r.separations
                .iter()
                .any(|p| (p.clearance - 0.355).abs() < 1.0e-6)
        );
        assert!(
            r.separations
                .iter()
                .any(|p| (p.clearance - 0.705).abs() < 1.0e-6)
        );
        let compiled = compile_copper_repair(&r).unwrap();
        assert!(
            compiled
                .world
                .segment_clearance
                .minimum
                .iter()
                .any(|v| (*v - 1.105).abs() < 1.0e-6),
            "zero-length fixed-circle segment keeps its radius plus target radius and local clearance"
        );
        let circle = r
            .polylines
            .iter()
            .find(|p| p.mobility == CopperPolylineMobility::Fixed)
            .unwrap();
        assert_eq!(circle.points[0], circle.points[1]);
        assert_eq!(circle.width, 0.6);
        let analytic = ObstacleGeometry::Circle {
            center: [2.0, 4.0],
            radius: 0.3,
        };
        assert!(analytic.intersects_segment([1.0, 3.0], [3.0, 3.0], 0.1 + 0.705));
        assert!(!analytic.intersects_segment([1.0, 2.8], [3.0, 2.8], 0.1 + 0.705));
    }

    #[test]
    fn output_retains_native_endpoints_and_actual_width() {
        let c = chain(
            "A",
            &[[0.123456789, 0.0], [2.0, 1.0], [4.123456789, 0.0]],
            0.3,
            0.25,
        );
        let points = vec![
            Vec2::new(0.5, 0.5),
            Vec2::new(2.0, 0.5),
            Vec2::new(4.5, 0.5),
        ];
        let output = candidates(
            std::slice::from_ref(&c),
            &BTreeMap::from([("net-0".into(), points)]),
        )
        .unwrap();
        assert_eq!(output[0].branches[0].path[0].at, c.points[0]);
        assert_eq!(output[0].branches[0].path[2].at, c.points[2]);
        assert!(
            output[0]
                .supplemental_segments
                .iter()
                .all(|s| s.width == 0.3)
        );
    }

    #[test]
    fn selected_net_rules_and_target_identity_are_explicit() {
        let rule = KiCadConnectionRoutingRules {
            trace_width_mm: 0.2,
            clearance_mm: 0.25,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        };
        let mut config = KiCadJointRelaxationConfig {
            routing: KiCadGridRouteConfig::default(),
            movable_connections: vec!["A".into()],
            solver: preset(),
        };
        config
            .routing
            .connection_rules
            .insert("TARGET".into(), rule.clone());
        assert!(
            validate_selection(&config, "TARGET").is_err(),
            "missing selected neighbor must not use defaults"
        );
        config.routing.connection_rules.insert("A".into(), rule);
        assert!(validate_selection(&config, "TARGET").is_ok());
        assert!(validate_selection(&config, "/TARGET").is_err());
        assert!(validate_selection(&config, "").is_err());
        config.movable_connections.push("TARGET".into());
        assert!(validate_selection(&config, "TARGET").is_err());
    }

    #[test]
    fn admission_cannot_trade_neighbor_connectivity_or_fixed_geometry_for_target() {
        let native:VerificationReport=serde_json::from_value(serde_json::json!({"board_id":"test","complete":true,
            "erc_violations":0,"drc_design_violations":0,"schematic_parity_issues":0,"selected_net_unconnected_items":0,
            "intentional_no_connect_groups":0,"library_metadata_warnings":0})).unwrap();
        let mut result = KiCadJointRelaxationResult {
            schema_version: 1,
            status: KiCadJointRelaxationStatus::Running,
            reason: None,
            source_board_sha256: String::new(),
            target_candidate_sha256: String::new(),
            config: KiCadJointRelaxationConfig {
                routing: KiCadGridRouteConfig::default(),
                movable_connections: vec!["A".into()],
                solver: preset(),
            },
            selected_directory: PathBuf::from("baseline"),
            baseline_native: native.clone(),
            final_native: Some(native),
            candidate_directory: None,
            frame_directories: Vec::new(),
            frames: 0,
            segment_pairs: 0,
            segment_body_pairs: 0,
            maximum_point_motion_mm: 1.0,
            per_net_pad_connectivity: BTreeMap::from([("TARGET".into(), true), ("A".into(), true)]),
            fixed_design_preserved: true,
            source_unchanged: true,
            contract: String::new(),
        };
        assert!(selection_admissible(&result, true, true, 4.0));
        result.per_net_pad_connectivity.insert("A".into(), false);
        assert!(!selection_admissible(&result, true, true, 4.0));
        result.per_net_pad_connectivity.insert("A".into(), true);
        result.fixed_design_preserved = false;
        assert!(!selection_admissible(&result, true, true, 4.0));
        result.fixed_design_preserved = true;
        result.maximum_point_motion_mm = 4.1;
        assert!(!selection_admissible(&result, true, true, 4.0));
    }
}
