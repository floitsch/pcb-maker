// Copyright (C) 2026 Toit contributors.

//! Local evidence for terminal/grid access. This does not run a router or
//! claim native validity: it exposes the same parsed copper and rule geometry.
use super::*;

fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn difference(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn edge_cuts(a: [f64; 2], direction: [f64; 2], p: [f64; 2], q: [f64; 2], cuts: &mut Vec<f64>) {
    let edge = difference(q, p);
    let offset = difference(p, a);
    let denominator = cross(direction, edge);
    if denominator.abs() > 1e-15 {
        let t = cross(offset, edge) / denominator;
        let u = cross(offset, direction) / denominator;
        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
            cuts.push(t);
        }
    } else if cross(offset, direction).abs() <= 1e-15 {
        let norm = direction[0] * direction[0] + direction[1] * direction[1];
        for point in [p, q] {
            let v = difference(point, a);
            let t = (v[0] * direction[0] + v[1] * direction[1]) / norm;
            if (0.0..=1.0).contains(&t) {
                cuts.push(t);
            }
        }
    }
}

fn boundary_cuts(
    geometry: &ObstacleGeometry,
    a: [f64; 2],
    direction: [f64; 2],
    cuts: &mut Vec<f64>,
) {
    match geometry {
        ObstacleGeometry::Circle { center, radius } => {
            let v = difference(a, *center);
            let aa = direction[0] * direction[0] + direction[1] * direction[1];
            let bb = 2.0 * (v[0] * direction[0] + v[1] * direction[1]);
            let cc = v[0] * v[0] + v[1] * v[1] - radius * radius;
            let discriminant = bb * bb - 4.0 * aa * cc;
            if discriminant >= 0.0 {
                for t in [
                    (-bb - discriminant.sqrt()) / (2.0 * aa),
                    (-bb + discriminant.sqrt()) / (2.0 * aa),
                ] {
                    if (0.0..=1.0).contains(&t) {
                        cuts.push(t);
                    }
                }
            }
        }
        ObstacleGeometry::Rectangle {
            center,
            half_size,
            angle_degrees,
        } => {
            let [x, y] = *half_size;
            let corners = [[-x, -y], [x, -y], [x, y], [-x, y]].map(|point| {
                let v = rotate_vector(point, *angle_degrees);
                [center[0] + v[0], center[1] + v[1]]
            });
            for i in 0..4 {
                edge_cuts(a, direction, corners[i], corners[(i + 1) % 4], cuts);
            }
        }
        ObstacleGeometry::Segment { start, end, radius } => {
            for center in [*start, *end] {
                boundary_cuts(
                    &ObstacleGeometry::Circle {
                        center,
                        radius: *radius,
                    },
                    a,
                    direction,
                    cuts,
                );
            }
            let delta = difference(*end, *start);
            let length = (delta[0] * delta[0] + delta[1] * delta[1]).sqrt();
            if length > 0.0 {
                boundary_cuts(
                    &ObstacleGeometry::Rectangle {
                        center: [(start[0] + end[0]) / 2.0, (start[1] + end[1]) / 2.0],
                        half_size: [length / 2.0, *radius],
                        angle_degrees: delta[1].atan2(delta[0]).to_degrees(),
                    },
                    a,
                    direction,
                    cuts,
                );
            }
        }
        ObstacleGeometry::Polygon { points } => {
            for i in 0..points.len() {
                edge_cuts(
                    a,
                    direction,
                    points[i],
                    points[(i + 1) % points.len()],
                    cuts,
                );
            }
        }
        ObstacleGeometry::Union { parts } => {
            for part in parts {
                boundary_cuts(part, a, direction, cuts);
            }
        }
    }
}

/// Check every constant-membership interval of the segment, including unions
/// and concave polygons. Endpoint membership alone would bridge notches/islands.
fn segment_inside(geometry: &ObstacleGeometry, a: [f64; 2], b: [f64; 2]) -> bool {
    if !geometry.contains(a, 0.0) || !geometry.contains(b, 0.0) {
        return false;
    }
    if distance_squared(a, b) <= 1e-24 {
        return true;
    }
    let direction = difference(b, a);
    let mut cuts = vec![0.0, 1.0];
    boundary_cuts(geometry, a, direction, &mut cuts);
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();
    let inside = |t| geometry.contains([a[0] + t * direction[0], a[1] + t * direction[1]], 0.0);
    cuts.iter().all(|&t| inside(t))
        && cuts
            .windows(2)
            .all(|pair| inside((pair[0] + pair[1]) / 2.0))
}

/// Recover a failed anchor snap using copper already connected to that anchor.
/// Each option is an unblocked center on the permitted pad layer. The anchor
/// connector stays inside existing pad copper and is removed by FirstPadContact;
/// it is deliberately not treated as a new full-width trace.
pub(super) fn copper_options(
    model: &KiCadRoutingModel,
    terminal: &ElectricalTerminal,
    origin: [f64; 2],
    size: [usize; 2],
    config: &KiCadGridRouteConfig,
    blocked: impl Fn(GridPosition) -> bool,
) -> Result<Vec<GridPosition>, String> {
    let anchor = terminal.at;
    let preferred = preferred_terminal_layer(terminal.layers).ok_or("terminal has no supported layer")?;
    let permitted = if terminal.layers == [true, true] {
        vec![preferred, 1 - preferred]
    } else {
        vec![preferred]
    };
    let mut options = Vec::new();
    for layer in permitted {
        let geometry = terminal_copper_geometry_at(model, anchor, layer)
            .ok_or("missing terminal copper geometry")?;
        let bounds = geometry.aabb();
        let low = [
            ((bounds.minimum[0] - origin[0]) / config.resolution_mm)
                .ceil()
                .max(0.0) as usize,
            ((bounds.minimum[1] - origin[1]) / config.resolution_mm)
                .ceil()
                .max(0.0) as usize,
        ];
        let high = [
            ((bounds.maximum[0] - origin[0]) / config.resolution_mm).floor() as isize,
            ((bounds.maximum[1] - origin[1]) / config.resolution_mm).floor() as isize,
        ];
        if high[0] < 0 || high[1] < 0 {
            continue;
        }
        let high = [
            (high[0] as usize).min(size[0] - 1),
            (high[1] as usize).min(size[1] - 1),
        ];
        if low[0] > high[0] || low[1] > high[1] {
            continue;
        }
        if (high[0] - low[0] + 1).saturating_mul(high[1] - low[1] + 1) > 65536 {
            return Err("terminal copper access scan exceeds 65536 cells per layer".into());
        }
        for y in low[1]..=high[1] {
            for x in low[0]..=high[0] {
                let point = GridPosition { x, y, layer };
                if !blocked(point)
                    && segment_inside(
                        &geometry,
                        anchor,
                        grid_position_at(point, origin, config.resolution_mm),
                    )
                {
                    options.push(point);
                }
            }
        }
    }
    options.sort_by(|a, b| {
        distance_squared(grid_position_at(*a, origin, config.resolution_mm), anchor)
            .total_cmp(&distance_squared(
                grid_position_at(*b, origin, config.resolution_mm),
                anchor,
            ))
            .then(a.layer.cmp(&b.layer))
            .then(a.y.cmp(&b.y))
            .then(a.x.cmp(&b.x))
    });
    Ok(options)
}

pub fn inspect_kicad_terminal_access(
    path: &Path,
    connection: &str,
    config: &KiCadGridRouteConfig,
) -> Result<serde_json::Value, String> {
    let source = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let config = connection_rules::resolve(config, connection)?;
    validate_grid_route_config(&config)?;
    let model = KiCadRoutingModel::from_pcb_with_config(&parse(&source)?, connection, &config)?;
    let radius = config.trace_width_mm / 2.0;
    let origin = [
        model.bounds[0] + config.edge_clearance_mm + radius,
        model.bounds[1] + config.edge_clearance_mm + radius,
    ];
    let (origin, alignment) = grid_alignment::aligned_origin(&model, &config, origin);
    let maximum = [
        model.bounds[2] - config.edge_clearance_mm - radius,
        model.bounds[3] - config.edge_clearance_mm - radius,
    ];
    if origin[0] >= maximum[0] || origin[1] >= maximum[1] {
        return Err("board has no routing area after applying edge clearance".into());
    }
    let size = [
        (((maximum[0] - origin[0]) / config.resolution_mm).floor() as usize) + 1,
        (((maximum[1] - origin[1]) / config.resolution_mm).floor() as usize) + 1,
    ];
    let mut terminals = Vec::new();
    for terminal in model.electrical_terminals() {
        let anchor = terminal.at;
        let preferred =
            preferred_terminal_layer(terminal.layers).ok_or("terminal has no supported layer")?;
        let permitted: Vec<_> = if terminal.layers == [true, true] {
            vec![preferred, 1 - preferred]
        } else {
            vec![preferred]
        };
        let footprint: Vec<_> = model
            .terminal_pads
            .iter()
            .filter(|pad| distance_squared(pad.at, anchor) <= 1e-12)
            .map(|pad| pad.footprint.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for layer in permitted {
            let geometry = terminal_copper_geometry_at(&model, anchor, layer)
                .ok_or("missing terminal geometry")?;
            let bounds = geometry.aabb();
            let padding = 1.0_f64.max(config.resolution_mm * 2.0);
            let viewport = GeometryAabb {
                minimum: [bounds.minimum[0] - padding, bounds.minimum[1] - padding],
                maximum: [bounds.maximum[0] + padding, bounds.maximum[1] + padding],
            };
            let low = [
                ((viewport.minimum[0] - origin[0]) / config.resolution_mm)
                    .ceil()
                    .max(0.0) as usize,
                ((viewport.minimum[1] - origin[1]) / config.resolution_mm)
                    .ceil()
                    .max(0.0) as usize,
            ];
            let high = [
                ((viewport.maximum[0] - origin[0]) / config.resolution_mm)
                    .floor()
                    .max(0.0) as usize,
                ((viewport.maximum[1] - origin[1]) / config.resolution_mm)
                    .floor()
                    .max(0.0) as usize,
            ];
            let high = [high[0].min(size[0] - 1), high[1].min(size[1] - 1)];
            let samples = high[0]
                .saturating_sub(low[0])
                .saturating_add(1)
                .saturating_mul(high[1].saturating_sub(low[1]).saturating_add(1));
            if samples > 65536 {
                return Err("terminal diagnostic exceeds 65536 cells per layer; use a coarser diagnostic grid".into());
            }
            let nearby: Vec<_> = model
                .obstacles
                .iter()
                .enumerate()
                .filter(|(_, obstacle)| {
                    let bounds = obstacle.geometry.aabb();
                    let inflate = radius + obstacle.clearance(config.clearance_mm);
                    let inflated = GeometryAabb {
                        minimum: [bounds.minimum[0] - inflate, bounds.minimum[1] - inflate],
                        maximum: [bounds.maximum[0] + inflate, bounds.maximum[1] + inflate],
                    };
                    obstacle.layers[layer]
                        && obstacle.blocks_tracks
                        && inflated.intersects(viewport)
                })
                .collect();
            let mut cells = Vec::new();
            for y in low[1]..=high[1] {
                for x in low[0]..=high[0] {
                    let at = [
                        origin[0] + x as f64 * config.resolution_mm,
                        origin[1] + y as f64 * config.resolution_mm,
                    ];
                    let blockers: Vec<_> = model
                        .obstacles
                        .iter()
                        .enumerate()
                        .filter(|(_, obstacle)| {
                            obstacle.layers[layer]
                                && obstacle.blocks_tracks
                                && obstacle
                                    .geometry
                                    .contains(at, radius + obstacle.clearance(config.clearance_mm))
                        })
                        .map(|(index, obstacle)| {
                            format!(
                                "#{index} {} net={}",
                                obstacle.object,
                                obstacle.net.as_deref().unwrap_or("none")
                            )
                        })
                        .collect();
                    let outside = !model
                        .outline
                        .contains_point_with_clearance(at, radius + config.edge_clearance_mm);
                    cells.push(serde_json::json!({"grid":[x,y],"at":at,"inside_pad":geometry.contains(at,0.0),
                    "blocked":outside || !blockers.is_empty(),"outside_outline":outside,"blockers":blockers,
                    "anchor_connector_clear":route_segment_is_clear(anchor,at,layer,&model,&config),
                    "anchor_segment_inside_pad":segment_inside(&geometry,anchor,at)}));
                }
            }
            let snapped = snapped_grid_position(anchor, origin, &config, size[0], size[1]);
            let nearest = snapped.as_ref().ok().map(|p| {
                serde_json::json!({"grid":[p.x,p.y],
                "at":grid_position_at(*p,origin,config.resolution_mm)})
            });
            terminals.push(serde_json::json!({"anchor":anchor,"footprints":footprint,"layer":copper_layer_name(layer),
                "geometry":geometry,"viewport":{"min":viewport.minimum,"max":viewport.maximum},
                "nearest":nearest,"nearest_error":snapped.err(),"cells":cells,
                "obstacles":nearby.into_iter().map(|(index,obstacle)| serde_json::json!({"index":index,"object":obstacle.object,
                    "net":obstacle.net,"footprint":obstacle.footprint,"geometry":obstacle.geometry,
                    "clearance_mm":obstacle.clearance(config.clearance_mm)})).collect::<Vec<_>>()}));
        }
    }
    Ok(
        serde_json::json!({"schema_version":1,"connection":connection,"config":config,"origin":origin,
        "grid_size":size,"grid_alignment":alignment,"source_sha256":format!("{:x}",Sha256::digest(source.as_bytes())),
        "scope":"Parsed native copper/rule geometry at local grid centers, with straight pad-internal continuity checks; no path search or native admission.",
        "terminals":terminals}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_pad_routes_from_copper_when_its_anchor_snap_is_blocked() {
        let source = r#"(kicad_pcb
          (gr_rect (start 0 0) (end 10 10) (layer "Edge.Cuts"))
          (footprint "JP1" (at 0 0)
            (pad "1" smd custom (at 1.603 3.234) (size 0.3 0.3) (layers "B.Cu") (net "/VCC")
              (options (clearance outline) (anchor rect))
              (primitives (gr_poly (pts (xy 1 0) (xy 0.5 -0.75) (xy -0.5 -0.75) (xy -0.5 0.75) (xy 0.5 0.75)) (width 0) (fill yes))))
            (pad "2" smd custom (at 3.053 3.234) (size 0.3 0.3) (layers "B.Cu") (net "/SIGNAL")
              (options (clearance outline) (anchor rect))
              (primitives (gr_poly (pts (xy 0.5 -0.75) (xy -0.65 -0.75) (xy -0.15 0) (xy -0.65 0.75) (xy 0.5 0.75)) (width 0) (fill yes)))))
          (footprint "OUT" (at 8 8)
            (pad "1" smd circle (at 0 0) (size 1 1) (layers "B.Cu") (net "/SIGNAL"))))"#;
        let path = env::temp_dir().join(format!(
            "pcb-terminal-access-{}.kicad_pcb",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, source).unwrap();
        let mut config = KiCadGridRouteConfig {
            resolution_mm: 0.5,
            trace_width_mm: 0.5,
            clearance_mm: 0.25,
            edge_clearance_mm: 0.17,
            max_expansions: 100_000,
            terminal_contact_policy: KiCadTerminalContactPolicy::PadAnchor,
            ..Default::default()
        };
        assert!(
            route_materialized_connection(&path, "SIGNAL", &config)
                .unwrap_err()
                .to_string()
                .contains("occupied")
        );
        config.terminal_contact_policy = KiCadTerminalContactPolicy::FirstPadContact;
        let candidate = route_materialized_connection(&path, "SIGNAL", &config).unwrap();
        assert!(candidate.supplemental_vias.is_empty());
        assert!(!candidate.supplemental_segments.is_empty());
        assert!(
            candidate
                .supplemental_segments
                .iter()
                .all(|segment| segment.layer == "B.Cu")
        );
        assert!(
            candidate
                .branches
                .iter()
                .flat_map(|branch| &branch.path)
                .all(|point| point.at != [3.053, 3.234])
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn copper_continuity_rejects_chords_across_notches_and_islands() {
        let notch = ObstacleGeometry::Polygon {
            points: vec![
                [0.0, 0.0],
                [4.0, 0.0],
                [4.0, 4.0],
                [3.0, 4.0],
                [3.0, 1.0],
                [1.0, 1.0],
                [1.0, 4.0],
                [0.0, 4.0],
            ],
        };
        assert!(segment_inside(&notch, [0.5, 0.5], [3.5, 0.5]));
        assert!(!segment_inside(&notch, [0.5, 3.0], [3.5, 3.0]));
        let islands = ObstacleGeometry::Union {
            parts: vec![
                ObstacleGeometry::Circle {
                    center: [0.0, 0.0],
                    radius: 1.0,
                },
                ObstacleGeometry::Circle {
                    center: [3.0, 0.0],
                    radius: 1.0,
                },
            ],
        };
        assert!(!segment_inside(&islands, [0.0, 0.0], [3.0, 0.0]));
        let touching = ObstacleGeometry::Union {
            parts: vec![
                ObstacleGeometry::Rectangle {
                    center: [0.0, 0.0],
                    half_size: [1.0, 1.0],
                    angle_degrees: 0.0,
                },
                ObstacleGeometry::Rectangle {
                    center: [2.0, 0.0],
                    half_size: [1.0, 1.0],
                    angle_degrees: 0.0,
                },
            ],
        };
        assert!(segment_inside(&touching, [0.0, 0.0], [2.0, 0.0]));
    }

    #[test]
    fn rotated_rectangles_capsules_and_union_seams_keep_internal_paths() {
        let rect = ObstacleGeometry::Rectangle {
            center: [3.0, 4.0],
            half_size: [2.0, 0.5],
            angle_degrees: 90.0,
        };
        assert!(segment_inside(&rect, [3.0, 3.0], [3.0, 5.0]));
        assert!(!segment_inside(&rect, [2.0, 4.0], [4.0, 4.0]));
        let capsule = ObstacleGeometry::Segment {
            start: [0.0, 0.0],
            end: [2.0, 0.0],
            radius: 0.5,
        };
        assert!(segment_inside(&capsule, [-0.4, 0.0], [2.4, 0.0]));
        assert!(!segment_inside(&capsule, [0.0, 0.6], [2.0, 0.6]));
    }

    fn model() -> KiCadRoutingModel {
        KiCadRoutingModel::from_pcb(&parse(r#"(kicad_pcb
            (gr_rect (start 0 0) (end 10 10) (layer "Edge.Cuts"))
            (footprint "A" (at 2.1 2.1) (pad "1" smd rect (at 0 0) (size 1.8 1.8) (layers "B.Cu") (net "/SIGNAL")))
            (footprint "B" (at 8 8) (pad "1" smd circle (at 0 0) (size 2 2) (layers "B.Cu") (net "/SIGNAL"))))"#).unwrap(),"SIGNAL").unwrap()
    }

    #[test]
    fn pad_options_preserve_layers_and_reject_blocked_or_disconnected_points() {
        let model = model();
        let config = KiCadGridRouteConfig {
            resolution_mm: 1.0,
            ..Default::default()
        };
        let options = copper_options(&model, &model.electrical_terminals()[0], [0.0, 0.0], [10, 10], &config, |p| {
            p.x == 2 && p.y == 2
        })
        .unwrap();
        assert!(!options.is_empty());
        assert!(
            options
                .iter()
                .all(|p| p.layer == 1 && (p.x != 2 || p.y != 2))
        );
        assert!(
            copper_options(&model, &model.electrical_terminals()[0], [0.0, 0.0], [10, 10], &config, |_| true)
                .unwrap()
                .is_empty()
        );
        let geometry = terminal_copper_geometry_at(&model, [2.1, 2.1], 1).unwrap();
        assert!(options.iter().all(|p| segment_inside(
            &geometry,
            [2.1, 2.1],
            grid_position_at(*p, [0.0, 0.0], 1.0)
        )));
    }
}
