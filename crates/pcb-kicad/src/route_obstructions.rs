// Copyright (C) 2026 Toit contributors.

//! Advisory attribution of a candidate's materialized copper, without rasterization.
use super::*;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadRouteObstructionStatus {
    Analyzed,
    Unsupported,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadRouteObstructionContact {
    pub candidate_kind: String,
    pub candidate_index: usize,
    pub layers: Vec<String>,
    pub obstacle_kind: String,
    pub obstacle_index: Option<usize>,
    pub obstacle_uuid: Option<String>,
    pub object: String,
    pub net: Option<String>,
    pub footprint: Option<String>,
    /// Minimum nonnegative copper gap; overlap has gap zero. None for outside copper.
    pub gap_mm: Option<f64>,
    pub required_clearance_mm: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadRouteObstructionReport {
    pub schema_version: u32,
    pub status: KiCadRouteObstructionStatus,
    pub unsupported_reason: Option<String>,
    pub board_sha256: String,
    pub candidate_sha256: String,
    /// SHA256 of serde's normalized supplied config, including defaults; not raw file bytes.
    pub supplied_config_sha256: String,
    pub connection: String,
    pub target_clearance_mm: f64,
    pub candidate_segments: usize,
    pub candidate_vias: usize,
    pub obstacles_considered: usize,
    pub exact_tests: usize,
    /// Only retained tracks/vias, never fixed foreign pads, are included here.
    pub foreign_routed_nets: Vec<String>,
    pub contacts: Vec<KiCadRouteObstructionContact>,
    pub contract: String,
}

/// Inspect actual supplemental copper against the current board. Rules are
/// explicit because imported candidates may contain inferred default clearances.
/// This does not mutate inputs, route, or grant native/connectivity admission.
pub fn inspect_kicad_route_obstructions(
    board: &Path,
    candidate_path: &Path,
    config: &KiCadGridRouteConfig,
) -> Result<KiCadRouteObstructionReport, String> {
    let source = fs::read_to_string(board).map_err(|e| e.to_string())?;
    let pcb = parse(&source)?;
    let candidate = read_route_candidate(candidate_path)?;
    let mut result = inspect(&pcb, &candidate, config)?;
    result.board_sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
    result.candidate_sha256 = file_sha256(candidate_path)?;
    Ok(result)
}

fn unsupported_geometry(pcb: &Expr, candidate: &KiCadRouteCandidate) -> Option<String> {
    if !candidate.footprint_placements.is_empty() || !candidate.reference_placements.is_empty() {
        return Some("candidate placement changes are outside this current-board query".into());
    }
    if candidate
        .supplemental_segments
        .iter()
        .any(|s| copper_layer_index(&s.layer).is_none())
        || candidate
            .supplemental_vias
            .iter()
            .any(|v| v.layers != ["F.Cu", "B.Cu"])
    {
        return Some("candidate copper must use outer-layer tracks and through vias".into());
    }
    unsupported_board_geometry(pcb)
}

pub(super) fn unsupported_board_geometry(pcb: &Expr) -> Option<String> {
    if board_copper_layers(pcb)
        .iter()
        .any(|layer| copper_layer_index(layer).is_none())
    {
        return Some("only two outer copper layers are supported".into());
    }
    for item in pcb.children() {
        if (item.head() == Some("zone") && !is_rule_area(item))
            || is_top_level_copper_graphic(item)
            || item.head() == Some("arc")
        {
            return Some("copper zones, arcs and copper graphics are not attributed".into());
        }
        if item.head() == Some("via") {
            if unsupported_padstack(item) {
                return Some("per-layer or removed-layer via padstacks are not attributed".into());
            }
            let layers = item.child("layers").map(|x| {
                x.children()
                    .iter()
                    .skip(1)
                    .filter_map(Expr::atom)
                    .collect::<Vec<_>>()
            });
            if layers.as_deref() != Some(&["F.Cu", "B.Cu"][..]) {
                return Some("retained vias must span F.Cu and B.Cu".into());
            }
        }
        if item.head() != Some("footprint") {
            continue;
        }
        for child in item.children() {
            if child.head() != Some("pad") {
                if form_atom(child, "layer", 1).is_some_and(|l| l.ends_with(".Cu")) {
                    return Some("footprint copper graphics are not attributed".into());
                }
                continue;
            }
            let shape = child.children().get(3).and_then(Expr::atom);
            if unsupported_padstack(child) {
                return Some("per-layer or removed-layer pad geometry is not attributed".into());
            }
            if !matches!(
                shape,
                Some("circle" | "oval" | "rect" | "roundrect" | "custom")
            ) {
                return Some(format!("pad shape {shape:?} has no exact query lowering"));
            }
            if shape == Some("roundrect") {
                let checked = form_xy(child, "size")
                    .and_then(|size| roundrect_pad::geometry(child, [0.0; 2], size, 0.0));
                if let Err(reason) = checked {
                    return Some(reason);
                }
            }
            if shape == Some("circle")
                && form_xy(child, "size").is_ok_and(|size| size[0] != size[1])
            {
                return Some(
                    "asymmetric circle-pad dimensions have no exact query lowering".into(),
                );
            }
            if shape == Some("custom")
                && child
                    .child("options")
                    .and_then(|o| form_atom(o, "anchor", 1))
                    != Some("rect")
            {
                return Some("custom pad query requires an explicit rectangular anchor".into());
            }
        }
    }
    None
}

fn unsupported_padstack(item: &Expr) -> bool {
    item.child("padstack").is_some()
        || item
            .child("remove_unused_layers")
            .is_some_and(|setting| setting.children().get(1).and_then(Expr::atom) != Some("no"))
}

pub(super) fn inspect(
    pcb: &Expr,
    candidate: &KiCadRouteCandidate,
    supplied: &KiCadGridRouteConfig,
) -> Result<KiCadRouteObstructionReport, String> {
    validate_grid_route_config(supplied)?;
    let config = connection_rules::resolve(supplied, &candidate.connection)?;
    let mut report = KiCadRouteObstructionReport {
        schema_version: 1,
        status: KiCadRouteObstructionStatus::Analyzed,
        unsupported_reason: None,
        board_sha256: String::new(),
        candidate_sha256: String::new(),
        supplied_config_sha256: format!("{:x}", Sha256::digest(serde_json::to_vec(supplied).map_err(|e| e.to_string())?)),
        connection: candidate.connection.clone(),
        target_clearance_mm: config.clearance_mm,
        candidate_segments: candidate.supplemental_segments.len(),
        candidate_vias: candidate.supplemental_vias.len(),
        obstacles_considered: 0,
        exact_tests: 0,
        foreign_routed_nets: Vec::new(),
        contacts: Vec::new(),
        contract: "Advisory copper/track-keepout/via-keepout/board-edge attribution only; explicit supplied rules, actual materialized widths/sizes and exact supported obstacle geometry. Fixed pads are not removable routes. No drill-spacing, custom-rule, connectivity or native-admission claim; empty contacts do not prove a routable or complete board.".into(),
    };
    if let Some(reason) = unsupported_geometry(pcb, candidate) {
        report.status = KiCadRouteObstructionStatus::Unsupported;
        report.unsupported_reason = Some(reason);
        return Ok(report);
    }
    let model = match KiCadRoutingModel::from_pcb_with_config(pcb, &candidate.connection, &config) {
        Ok(model) => model,
        Err(reason) => {
            report.status = KiCadRouteObstructionStatus::Unsupported;
            report.unsupported_reason = Some(reason);
            return Ok(report);
        }
    };
    report.obstacles_considered = model.obstacles.len();
    for (index, segment) in candidate.supplemental_segments.iter().enumerate() {
        let layer =
            copper_layer_index(&segment.layer).ok_or("unsupported candidate segment layer")?;
        if segment.connection != candidate.connection
            || !segment.width.is_finite()
            || segment.width <= 0.0
            || segment
                .start
                .iter()
                .chain(segment.end.iter())
                .any(|x| !x.is_finite())
        {
            return Err("candidate has invalid materialized segment geometry".into());
        }
        inspect_primitive(
            &mut report,
            &model,
            &config,
            "segment",
            index,
            segment.start,
            segment.end,
            segment.width / 2.0,
            layer_mask(layer),
            CopperQueryKind::Trace,
        );
    }
    for (index, via) in candidate.supplemental_vias.iter().enumerate() {
        if via.connection != candidate.connection
            || via.layers != ["F.Cu", "B.Cu"]
            || !via.size.is_finite()
            || via.size <= 0.0
            || !via.drill.is_finite()
            || via.drill <= 0.0
            || via.drill >= via.size
            || via.at.iter().any(|x| !x.is_finite())
        {
            return Err("candidate has invalid or unsupported through-via geometry".into());
        }
        inspect_primitive(
            &mut report,
            &model,
            &config,
            "via",
            index,
            via.at,
            via.at,
            via.size / 2.0,
            [true, true],
            CopperQueryKind::Via,
        );
    }
    report.foreign_routed_nets = report
        .contacts
        .iter()
        .filter(|contact| contact.obstacle_kind == "foreign_net_copper")
        .filter_map(|contact| contact.net.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn inspect_primitive(
    report: &mut KiCadRouteObstructionReport,
    model: &KiCadRoutingModel,
    config: &KiCadGridRouteConfig,
    kind: &str,
    index: usize,
    start: [f64; 2],
    end: [f64; 2],
    radius: f64,
    layers: [bool; 2],
    query: CopperQueryKind,
) {
    let layer_names = |mask: [bool; 2]| {
        ["F.Cu", "B.Cu"]
            .into_iter()
            .enumerate()
            .filter(|(i, _)| mask[*i])
            .map(|(_, l)| l.to_string())
            .collect::<Vec<_>>()
    };
    if !model
        .outline
        .contains_segment_with_clearance(start, end, radius + config.edge_clearance_mm)
    {
        let within = model
            .outline
            .contains_segment_with_clearance(start, end, 0.0);
        let gap = model
            .outline
            .edges()
            .map(|(a, b)| segment_segment_distance_squared(start, end, a, b).sqrt())
            .fold(f64::INFINITY, f64::min)
            - radius;
        report.contacts.push(KiCadRouteObstructionContact {
            candidate_kind: kind.into(),
            candidate_index: index,
            layers: layer_names(layers),
            obstacle_kind: "board_edge".into(),
            obstacle_index: None,
            obstacle_uuid: None,
            object: "Edge.Cuts".into(),
            net: None,
            footprint: None,
            gap_mm: within.then_some(gap.max(0.0)),
            required_clearance_mm: config.edge_clearance_mm,
        });
    }
    for (obstacle_index, obstacle) in model.obstacles.iter().enumerate() {
        let shared = [
            layers[0] && obstacle.layers[0],
            layers[1] && obstacle.layers[1],
        ];
        if !obstacle.blocks(query) || !shared.iter().any(|x| *x) {
            continue;
        }
        let clearance = obstacle.clearance(config.clearance_mm);
        if !obstacle
            .geometry
            .aabb()
            .intersects(GeometryAabb::around_segment(start, end, radius + clearance))
        {
            continue;
        }
        report.exact_tests += 1;
        if !obstacle
            .geometry
            .intersects_segment(start, end, radius + clearance)
        {
            continue;
        }
        let obstacle_kind = match obstacle.kind {
            KiCadViaLocalRerouteBlockerKind::FootprintPad => "footprint_pad",
            KiCadViaLocalRerouteBlockerKind::ForeignNetCopper => "foreign_net_copper",
            KiCadViaLocalRerouteBlockerKind::RuleArea => "rule_area",
        };
        report.contacts.push(KiCadRouteObstructionContact {
            candidate_kind: kind.into(),
            candidate_index: index,
            layers: layer_names(shared),
            obstacle_kind: obstacle_kind.into(),
            obstacle_index: Some(obstacle_index),
            obstacle_uuid: obstacle.source_uuid.clone(),
            object: obstacle.object.clone(),
            net: obstacle.net.clone(),
            footprint: obstacle.footprint.clone(),
            gap_mm: Some((centerline_gap(&obstacle.geometry, start, end) - radius).max(0.0)),
            required_clearance_mm: clearance,
        });
    }
}

/// Exact nonnegative distance to the supported filled obstacle, before the
/// candidate's own radius. Uses the same distance primitives as collision tests.
fn centerline_gap(geometry: &ObstacleGeometry, start: [f64; 2], end: [f64; 2]) -> f64 {
    let polygon_gap = |points: &[[f64; 2]]| {
        if point_in_polygon(start, points) || point_in_polygon(end, points) {
            return 0.0;
        }
        points
            .iter()
            .copied()
            .zip(points.iter().copied().cycle().skip(1))
            .take(points.len())
            .map(|(a, b)| segment_segment_distance_squared(start, end, a, b).sqrt())
            .fold(f64::INFINITY, f64::min)
    };
    match geometry {
        ObstacleGeometry::Circle { center, radius } => {
            (point_segment_distance_squared(*center, start, end).sqrt() - radius).max(0.0)
        }
        ObstacleGeometry::Segment {
            start: a,
            end: b,
            radius,
        } => (segment_segment_distance_squared(start, end, *a, *b).sqrt() - radius).max(0.0),
        ObstacleGeometry::Rectangle {
            center,
            half_size,
            angle_degrees,
        } => {
            let corners = [
                [-half_size[0], -half_size[1]],
                [half_size[0], -half_size[1]],
                [half_size[0], half_size[1]],
                [-half_size[0], half_size[1]],
            ]
            .map(|point| {
                let p = rotate_vector(point, *angle_degrees);
                [center[0] + p[0], center[1] + p[1]]
            });
            polygon_gap(&corners)
        }
        ObstacleGeometry::Polygon { points } => polygon_gap(points),
        ObstacleGeometry::Union { parts } => parts
            .iter()
            .map(|part| centerline_gap(part, start, end))
            .fold(f64::INFINITY, f64::min),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(extra: &str) -> Expr {
        parse(&format!(r#"(kicad_pcb
            (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
            (footprint "A" (at 2 2) (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "/TARGET")))
            (footprint "B" (at 18 2) (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "/TARGET")))
            (segment (start 2 2) (end 18 2) (width 0.2) (layer "F.Cu") (net "TARGET") (uuid "same-net"))
            (segment (start 10 5) (end 10 15) (width 0.4) (layer "F.Cu") (net "FRONT") (uuid "front-track"))
            (segment (start 10 5) (end 10 15) (width 0.4) (layer "B.Cu") (net "BACK") (uuid "back-track"))
            {extra})"#)).unwrap()
    }

    fn candidate(pcb: &Expr) -> KiCadRouteCandidate {
        let mut candidate = import_route_candidate_from_expr(pcb, "TARGET").unwrap();
        candidate.config.clearance_mm = 0.01;
        candidate.config.trace_width_mm = 9.0;
        // Keep logical branches unchanged: only the actual applied copper is queried.
        candidate.supplemental_segments = vec![SupplementalSegment {
            connection: "TARGET".into(),
            start: [10.65, 6.0],
            end: [10.65, 14.0],
            width: 0.2,
            layer: "F.Cu".into(),
        }];
        candidate
    }

    fn rules(clearance: f64) -> KiCadConnectionRoutingRules {
        KiCadConnectionRoutingRules {
            trace_width_mm: 0.2,
            clearance_mm: clearance,
            via_size_mm: 0.6,
            via_drill_mm: 0.3,
        }
    }

    #[test]
    fn actual_materialized_width_layer_uuid_and_foreign_class_gap() {
        let pcb = fixture("");
        let candidate = candidate(&pcb);
        let mut config = KiCadGridRouteConfig {
            clearance_mm: 0.3,
            edge_clearance_mm: 0.0,
            ..Default::default()
        };
        assert!(
            inspect(&pcb, &candidate, &config)
                .unwrap()
                .contacts
                .is_empty()
        );
        config.connection_rules.insert("TARGET".into(), rules(0.2));
        config.connection_rules.insert("FRONT".into(), rules(0.4));
        config.connection_rules.insert("BACK".into(), rules(0.2));
        let report = inspect(&pcb, &candidate, &config).unwrap();
        assert_eq!(report.foreign_routed_nets, ["FRONT"]);
        assert_eq!(report.contacts.len(), 1);
        let hit = &report.contacts[0];
        assert_eq!(hit.obstacle_uuid.as_deref(), Some("front-track"));
        assert_eq!(hit.required_clearance_mm, 0.4);
        assert!((hit.gap_mm.unwrap() - 0.35).abs() < 1e-10);
        assert_eq!(hit.layers, ["F.Cu"]);
    }

    #[test]
    fn fixed_pad_and_via_only_keepout_do_not_become_removable_traces() {
        let pcb = fixture(
            r#"
            (footprint "P" (property "Reference" "P1") (at 11.25 10)
              (pad "1" smd circle (at 0 0) (size 0.2 0.2) (layers "F.Cu") (net "PAD") (clearance 0.6) (uuid "pad-item")))
            (zone (net 0) (layers "F.Cu" "B.Cu") (name "via-only") (uuid "keepout-item")
              (keepout (tracks allowed) (vias not_allowed))
              (polygon (pts (xy 10.5 9) (xy 10.8 9) (xy 10.8 11) (xy 10.5 11))))"#,
        );
        let mut candidate = candidate(&pcb);
        let config = KiCadGridRouteConfig {
            clearance_mm: 0.2,
            edge_clearance_mm: 0.0,
            ..Default::default()
        };
        let report = inspect(&pcb, &candidate, &config).unwrap();
        assert!(report.foreign_routed_nets.is_empty());
        assert_eq!(report.contacts.len(), 1);
        assert_eq!(report.contacts[0].obstacle_kind, "footprint_pad");
        assert_eq!(
            report.contacts[0].obstacle_uuid.as_deref(),
            Some("pad-item")
        );
        assert!((report.contacts[0].gap_mm.unwrap() - 0.4).abs() < 1e-10);
        assert_eq!(report.contacts[0].required_clearance_mm, 0.6);
        candidate.supplemental_segments.clear();
        candidate.supplemental_vias = vec![SupplementalVia {
            connection: "TARGET".into(),
            at: [10.65, 10.0],
            size: 0.6,
            drill: 0.3,
            layers: ["F.Cu".into(), "B.Cu".into()],
        }];
        let report = inspect(&pcb, &candidate, &config).unwrap();
        assert!(
            report
                .contacts
                .iter()
                .any(|h| h.obstacle_kind == "rule_area"
                    && h.obstacle_uuid.as_deref() == Some("keepout-item"))
        );
        assert_eq!(report.foreign_routed_nets, ["BACK", "FRONT"]);
    }

    #[test]
    fn unsupported_geometry_and_malformed_copper_never_report_clear() {
        let pcb = fixture("");
        let mut candidate = candidate(&pcb);
        let config = KiCadGridRouteConfig::default();
        let arcs = fixture(
            r#"(arc (start 1 1) (mid 2 2) (end 3 1) (width 0.2) (layer "F.Cu") (net "ARC"))"#,
        );
        let report = inspect(&arcs, &candidate, &config).unwrap();
        assert!(matches!(
            report.status,
            KiCadRouteObstructionStatus::Unsupported
        ));
        assert!(report.unsupported_reason.unwrap().contains("arcs"));
        candidate.supplemental_segments[0].layer = "In1.Cu".into();
        assert!(matches!(
            inspect(&pcb, &candidate, &config).unwrap().status,
            KiCadRouteObstructionStatus::Unsupported
        ));
        candidate.supplemental_segments[0].layer = "F.Cu".into();
        candidate.supplemental_segments[0].width = f64::NAN;
        assert!(inspect(&pcb, &candidate, &config).is_err());
    }

    #[test]
    fn per_layer_padstacks_and_removed_layers_are_explicitly_unsupported() {
        let pcb = fixture("");
        let candidate = candidate(&pcb);
        let config = KiCadGridRouteConfig::default();
        for extra in [
            r#"(via (at 5 5) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net "V") (padstack (layer "B.Cu" (size 1.2))))"#,
            r#"(footprint "STACK" (at 5 5) (pad "1" thru_hole circle (at 0 0) (size 1 1) (drill 0.5) (layers "*.Cu") (net "P") (padstack (layer "B.Cu" (size 2 2)))))"#,
            r#"(via (at 5 5) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net "V") (remove_unused_layers))"#,
        ] {
            let report = inspect(&fixture(extra), &candidate, &config).unwrap();
            assert!(matches!(
                report.status,
                KiCadRouteObstructionStatus::Unsupported
            ));
            assert!(report.unsupported_reason.unwrap().contains("per-layer"));
        }
        let unchanged = fixture(
            r#"(via (at 5 5) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net "V") (remove_unused_layers no))"#,
        );
        assert!(matches!(
            inspect(&unchanged, &candidate, &config).unwrap().status,
            KiCadRouteObstructionStatus::Analyzed
        ));
    }

    #[test]
    fn exact_gap_agrees_with_collision_threshold_for_rotated_and_compound_shapes() {
        let rectangle = ObstacleGeometry::Rectangle {
            center: [5.0, 5.0],
            half_size: [2.0, 1.0],
            angle_degrees: 30.0,
        };
        let polygon = ObstacleGeometry::Polygon {
            points: vec![[2.0, 2.0], [4.0, 2.0], [3.0, 4.0]],
        };
        let compound = ObstacleGeometry::Union {
            parts: vec![rectangle.clone(), polygon.clone()],
        };
        for geometry in [rectangle, polygon, compound] {
            let a = [8.0, 7.0];
            let b = [8.0, 9.0];
            let gap = centerline_gap(&geometry, a, b);
            assert!(gap > 0.01);
            assert!(!geometry.intersects_segment(a, b, gap - 1e-6));
            assert!(geometry.intersects_segment(a, b, gap + 1e-6));
        }
    }
    #[test]
    fn roundrect_corner_gap_retains_layer_and_local_clearance() {
        let pcb = fixture(
            r#"(footprint "P" (property "Reference" "P1") (at 5 5 90)
            (pad "1" smd roundrect (at 0 0 90) (size 2 2) (layers "F.Cu")
             (roundrect_rratio 0.25) (net "PAD") (clearance 0.04) (uuid "rounded-pad")))"#,
        );
        let mut candidate = candidate(&pcb);
        candidate.supplemental_segments[0].start = [5.95, 5.95];
        candidate.supplemental_segments[0].end = [6.05, 6.05];
        let config = KiCadGridRouteConfig {
            clearance_mm: 0.02,
            edge_clearance_mm: 0.0,
            ..Default::default()
        };
        let report = inspect(&pcb, &candidate, &config).unwrap();
        assert!(matches!(
            report.status,
            KiCadRouteObstructionStatus::Analyzed
        ));
        assert_eq!(report.contacts.len(), 1);
        let hit = &report.contacts[0];
        assert_eq!(hit.obstacle_uuid.as_deref(), Some("rounded-pad"));
        assert!((hit.gap_mm.unwrap() - (0.45_f64.hypot(0.45) - 0.5 - 0.1)).abs() < 1e-9);
        assert_eq!(hit.required_clearance_mm, 0.04);
        candidate.supplemental_segments[0].layer = "B.Cu".into();
        assert!(
            inspect(&pcb, &candidate, &config)
                .unwrap()
                .contacts
                .is_empty()
        );
    }

    #[test]
    fn malformed_or_chamfered_roundrect_query_is_explicitly_unsupported() {
        let original = fixture("");
        let candidate = candidate(&original);
        for fields in [
            "(roundrect_rratio NaN)",
            "(roundrect_rratio 0.6)",
            "(roundrect_rratio 0.25) (chamfer top_left)",
            "",
        ] {
            let pcb = fixture(&format!(
                "(footprint \"P\" (at 5 5) (pad \"1\" smd roundrect (at 0 0) (size 2 2) (layers \"F.Cu\") (net \"PAD\") {fields}))"
            ));
            assert!(matches!(
                inspect(&pcb, &candidate, &KiCadGridRouteConfig::default())
                    .unwrap()
                    .status,
                KiCadRouteObstructionStatus::Unsupported
            ));
        }
    }
}
