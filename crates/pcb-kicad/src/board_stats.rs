// Copyright (C) 2026 Toit contributors.

//! Measurements read from a board: statistics, physical copper, outline
//! area, the routable connections and placement and copper digests.

use super::*;

/// Cheap structural measurements read directly from a KiCad board. These are
/// suitable for benchmark provenance and cold-input checks. The length is the
/// sum of stored straight segments, not canonical physical-copper union.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct KiCadBoardStatistics {
    pub footprints: usize,
    /// Distinct non-empty net names referenced anywhere in the board. This
    /// supports both ordinary KiCad `(net id "name")` forms and pcb-maker's
    /// name-addressed `(net "name")` forms.
    pub total_named_nets_including_no_connects: usize,
    pub segments: usize,
    pub arcs: usize,
    pub vias: usize,
    /// All top-level KiCad zone objects, including non-copper rule areas.
    pub zones: usize,
    /// Zones that describe copper pours rather than rule-area keepouts.
    #[serde(default)]
    pub copper_zones: usize,
    /// Non-copper zone objects carrying a top-level `(keepout ...)` form.
    #[serde(default)]
    pub rule_areas: usize,
    /// Top-level graphic objects placed directly on a copper layer. Footprint
    /// copper primitives and pads are fixed component geometry, not counted
    /// here.
    #[serde(default)]
    pub copper_graphics: usize,
    pub stored_segment_length_mm: f64,
    #[serde(default)]
    pub physical_copper: KiCadPhysicalCopperStatistics,
    /// Order-independent digest of top-level tracks, arcs, vias, and zones,
    /// excluding KiCad object UUIDs/timestamps. Raw board hashes remain useful
    /// provenance, while this digest identifies equivalent copper decisions.
    pub copper_geometry_sha256: String,
    /// Order-independent reference/position/rotation digest for every
    /// footprint. This proves two route-only competitors received the same
    /// fixed placement without depending on KiCad UUIDs or file order.
    pub component_placement_sha256: String,
    /// The same pose digest after coordinates and angles are rounded to the
    /// 0.0001 mm/degree precision used by the Specctra exchange path.
    pub component_placement_100nm_sha256: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct KiCadPhysicalCopperStatistics {
    pub schema_version: u32,
    /// Union length of straight same-net tracks after splitting every contact
    /// and deduplicating collinear overlap.
    pub canonical_straight_track_length_mm: f64,
    /// Circular centerline length of serialized KiCad arc tracks.
    pub arc_track_length_mm: f64,
    /// Straight physical union plus arc centerlines. This is an exact
    /// centerline union only when `centerline_union_exact` is true.
    pub physical_centerline_length_mm: f64,
    pub centerline_union_exact: bool,
    pub overlapping_straight_track_length_mm: f64,
    /// Sum of canonical straight edge length times maximum covering width,
    /// plus serialized arc length times width. It excludes end caps and does
    /// not subtract crossing or zone overlap.
    pub centerline_weighted_track_area_mm2: f64,
    pub via_projected_outer_area_mm2: f64,
    pub via_drill_area_mm2: f64,
    pub layer_weighted_via_annulus_area_mm2: f64,
    /// Shoelace area of refilled `filled_polygon` contours.
    pub filled_zone_area_mm2: f64,
    /// Track area + layer-weighted via annuli + filled zone polygons. This is
    /// a primitive sum, not a planar union.
    pub primitive_copper_area_mm2: f64,
    pub board_outline_area_mm2: Option<f64>,
    pub used_copper_layers: usize,
    pub layers: Vec<KiCadCopperLayerStatistics>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct KiCadCopperLayerStatistics {
    pub layer: String,
    pub stored_straight_track_length_mm: f64,
    pub canonical_straight_track_length_mm: f64,
    pub arc_track_length_mm: f64,
    pub physical_centerline_length_mm: f64,
    pub centerline_weighted_track_area_mm2: f64,
    pub vias_spanning_layer: usize,
    pub via_annulus_area_mm2: f64,
    pub filled_zone_area_mm2: f64,
    pub primitive_copper_area_mm2: f64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KiCadConnectionSummary {
    pub connection: String,
    pub distinct_pad_centers: usize,
    /// Omitted when identical to geometric centers, including historical journals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub electrical_terminal_count: Option<usize>,
}

pub fn inspect_kicad_board(path: &Path) -> Result<KiCadBoardStatistics, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let pcb =
        parse(&source).map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    if pcb.head() != Some("kicad_pcb") {
        return Err(format!("{} is not a kicad_pcb document", path.display()));
    }
    board_statistics(&pcb)
}

/// Returns named electrical connections with at least two electrical pad
/// contacts, in KiCad net-declaration order. This is the route inventory for a
/// full-connectivity cold board; net IDs are deliberately not exposed as
/// semantic connection names.
pub fn inspect_kicad_routable_connections(
    path: &Path,
) -> Result<Vec<KiCadConnectionSummary>, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let pcb =
        parse(&source).map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    if pcb.head() != Some("kicad_pcb") {
        return Err(format!("{} is not a kicad_pcb document", path.display()));
    }
    let mut connection_order = Vec::new();
    let mut seen = BTreeSet::new();
    for net in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("net"))
    {
        if let Some(name) = net_name_from_form(net).map(normalize_net)
            && !name.is_empty()
            && !name.bytes().all(|byte| byte.is_ascii_digit())
            && !name.starts_with("unconnected-(")
            && seen.insert(name.to_string())
        {
            connection_order.push(name.to_string());
        }
    }
    let mut centers = BTreeMap::<String, BTreeMap<Point, ([bool; 2], bool)>>::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let footprint_at = form_at(footprint)?;
        for pad in footprint
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
        {
            let Some(connection) = node_net(pad).map(normalize_net).filter(|name| {
                !name.is_empty()
                    && !name.bytes().all(|byte| byte.is_ascii_digit())
                    && !name.starts_with("unconnected-(")
            }) else {
                continue;
            };
            if seen.insert(connection.to_string()) {
                connection_order.push(connection.to_string());
            }
            let pad_at = form_at(pad)?;
            let offset = rotate_vector([pad_at[0], pad_at[1]], -footprint_at[2]);
            let center = [footprint_at[0] + offset[0], footprint_at[1] + offset[1]];
            let group = centers
                .entry(connection.to_string())
                .or_default()
                .entry(Point(
                    (center[0] * 1_000_000.0).round() as i64,
                    (center[1] * 1_000_000.0).round() as i64,
                ))
                .or_insert(([false; 2], false));
            let layers = pad_copper_layers(pad);
            group.0[0] |= layers[0];
            group.0[1] |= layers[1];
            group.1 |= layers == [true, true] && electrical_terminals::plated_pad(pad);
        }
    }
    Ok(connection_order
        .into_iter()
        .filter_map(|connection| {
            let distinct_pad_centers = centers.get(&connection).map_or(0, BTreeMap::len);
            let electrical_terminal_count: usize = centers
                .get(&connection)
                .into_iter()
                .flat_map(|groups| groups.values())
                .map(|(layers, plated)| {
                    electrical_terminals::contact_layers(*layers, *plated).len()
                })
                .sum();
            (electrical_terminal_count >= 2).then_some(KiCadConnectionSummary {
                connection,
                distinct_pad_centers,
                electrical_terminal_count: (electrical_terminal_count != distinct_pad_centers)
                    .then_some(electrical_terminal_count),
            })
        })
        .collect())
}

pub(crate) fn board_statistics(pcb: &Expr) -> Result<KiCadBoardStatistics, String> {
    let mut stored_segment_length_mm = 0.0;
    for item in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("segment"))
    {
        let start = form_xy(item, "start")?;
        let end = form_xy(item, "end")?;
        stored_segment_length_mm += (end[0] - start[0]).hypot(end[1] - start[1]);
    }
    let mut named_nets = BTreeSet::new();
    collect_named_nets(pcb, &mut named_nets);
    let zones = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone"))
        .collect::<Vec<_>>();
    let rule_areas = zones.iter().filter(|zone| is_rule_area(zone)).count();
    Ok(KiCadBoardStatistics {
        footprints: count_head(pcb, "footprint"),
        total_named_nets_including_no_connects: named_nets.len(),
        segments: count_head(pcb, "segment"),
        arcs: count_head(pcb, "arc"),
        vias: count_head(pcb, "via"),
        zones: zones.len(),
        copper_zones: zones.len() - rule_areas,
        rule_areas,
        copper_graphics: pcb
            .children()
            .iter()
            .filter(|item| is_top_level_copper_graphic(item))
            .count(),
        stored_segment_length_mm,
        physical_copper: physical_copper_statistics(pcb, stored_segment_length_mm)?,
        copper_geometry_sha256: copper_geometry_sha256(pcb),
        component_placement_sha256: component_placement_sha256(pcb)?,
        component_placement_100nm_sha256: component_placement_quantized_sha256(pcb)?,
    })
}

pub(crate) fn is_rule_area(zone: &Expr) -> bool {
    zone.head() == Some("zone") && zone.child("keepout").is_some()
}

pub(crate) fn is_top_level_copper_graphic(item: &Expr) -> bool {
    matches!(
        item.head(),
        Some(
            "gr_text"
                | "gr_text_box"
                | "gr_line"
                | "gr_rect"
                | "gr_arc"
                | "gr_poly"
                | "gr_curve"
                | "dimension"
        )
    ) && form_atom(item, "layer", 1).is_some_and(|layer| layer.ends_with(".Cu"))
}

#[derive(Default)]
struct MeasuredCopperLayer {
    stored_straight_track_length_mm: f64,
    canonical_straight_track_length_mm: f64,
    arc_track_length_mm: f64,
    centerline_weighted_track_area_mm2: f64,
    vias_spanning_layer: usize,
    via_annulus_area_mm2: f64,
    filled_zone_area_mm2: f64,
}

fn physical_copper_statistics(
    pcb: &Expr,
    stored_segment_length_mm: f64,
) -> Result<KiCadPhysicalCopperStatistics, String> {
    let copper_layers = board_copper_layers(pcb);
    let mut layers = copper_layers
        .iter()
        .cloned()
        .map(|layer| (layer, MeasuredCopperLayer::default()))
        .collect::<BTreeMap<_, _>>();
    let mut branches_by_net = BTreeMap::<String, Vec<(KiCadRouteBranch, f64)>>::new();
    for (ordinal, item) in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("segment"))
        .enumerate()
    {
        let start = form_xy(item, "start")?;
        let end = form_xy(item, "end")?;
        let width = form_f64(item, "width", 1)?;
        if !width.is_finite() || width <= 0.0 {
            return Err(format!("segment {ordinal} has invalid geometry"));
        }
        // Real boards carry zero-length segments; they add no copper.
        if start == end {
            continue;
        }
        let layer = form_atom(item, "layer", 1)
            .ok_or_else(|| format!("segment {ordinal} has no layer"))?
            .to_string();
        // Copper on no net (stray or decorative tracks) still counts.
        let net = node_net(item)
            .map(normalize_net)
            .filter(|net| !net.is_empty())
            .unwrap_or("(no net)")
            .to_string();
        let length = distance_squared(start, end).sqrt();
        layers
            .entry(layer.clone())
            .or_default()
            .stored_straight_track_length_mm += length;
        branches_by_net.entry(net).or_default().push((
            KiCadRouteBranch {
                start_terminal: start,
                finish_terminal: end,
                cost: 0,
                expansions: 0,
                path: vec![
                    KiCadRoutePoint {
                        at: start,
                        layer: layer.clone(),
                    },
                    KiCadRoutePoint { at: end, layer },
                ],
            },
            width,
        ));
    }

    let mut canonical_straight_track_length_mm = 0.0;
    let mut centerline_weighted_track_area_mm2 = 0.0;
    for branches in branches_by_net.into_values() {
        let mut route_branches = branches
            .iter()
            .map(|(branch, _)| branch.clone())
            .collect::<Vec<_>>();
        let selected = (0..route_branches.len()).collect::<Vec<_>>();
        normalize_selected_route_graph(&mut route_branches, &selected);
        let mut edges = BTreeMap::<String, ([f64; 2], [f64; 2], String, f64)>::new();
        for (branch_index, branch) in route_branches.iter().enumerate() {
            let width = branches[branch_index].1;
            for pair in branch.path.windows(2) {
                if pair[0].layer != pair[1].layer || pair[0].at == pair[1].at {
                    continue;
                }
                let (start, end) = ordered_points(pair[0].at, pair[1].at);
                let key = format!(
                    "{}:{:.9}:{:.9}:{:.9}:{:.9}",
                    pair[0].layer, start[0], start[1], end[0], end[1]
                );
                edges
                    .entry(key)
                    .and_modify(|edge| edge.3 = edge.3.max(width))
                    .or_insert((start, end, pair[0].layer.clone(), width));
            }
        }
        for (_, (start, end, layer, width)) in edges {
            let length = distance_squared(start, end).sqrt();
            canonical_straight_track_length_mm += length;
            centerline_weighted_track_area_mm2 += length * width;
            let layer = layers.entry(layer).or_default();
            layer.canonical_straight_track_length_mm += length;
            layer.centerline_weighted_track_area_mm2 += length * width;
        }
    }

    let mut arc_track_length_mm = 0.0;
    let mut arc_count = 0;
    for (ordinal, item) in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("arc"))
        .enumerate()
    {
        let length = track_arc_length(item)
            .map_err(|error| format!("failed to measure track arc {ordinal}: {error}"))?;
        let width = form_f64(item, "width", 1)?;
        if !width.is_finite() || width <= 0.0 {
            return Err(format!("track arc {ordinal} has invalid width"));
        }
        let layer = form_atom(item, "layer", 1)
            .ok_or_else(|| format!("track arc {ordinal} has no layer"))?
            .to_string();
        arc_count += 1;
        arc_track_length_mm += length;
        centerline_weighted_track_area_mm2 += length * width;
        let layer = layers.entry(layer).or_default();
        layer.arc_track_length_mm += length;
        layer.centerline_weighted_track_area_mm2 += length * width;
    }

    let mut via_projected_outer_area_mm2 = 0.0;
    let mut via_drill_area_mm2 = 0.0;
    let mut layer_weighted_via_annulus_area_mm2 = 0.0;
    for (ordinal, item) in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("via"))
        .enumerate()
    {
        let diameter = form_f64(item, "size", 1)?;
        let drill = form_f64(item, "drill", 1)?;
        if !diameter.is_finite() || !drill.is_finite() || diameter <= 0.0 || drill <= 0.0 {
            return Err(format!("via {ordinal} has invalid diameter/drill"));
        }
        // Real boards have vias drilled as wide as their pad (KiCad accepts
        // them when the annular-width check is off): no annulus.
        let outer_area = std::f64::consts::PI * (diameter / 2.0).powi(2);
        let drill_area = std::f64::consts::PI * (drill.min(diameter) / 2.0).powi(2);
        let annulus_area = outer_area - drill_area;
        via_projected_outer_area_mm2 += outer_area;
        via_drill_area_mm2 += drill_area;
        let spanned = via_spanned_layers(item, &copper_layers)?;
        layer_weighted_via_annulus_area_mm2 += annulus_area * spanned.len() as f64;
        for layer in spanned {
            let layer = layers.entry(layer).or_default();
            layer.vias_spanning_layer += 1;
            layer.via_annulus_area_mm2 += annulus_area;
        }
    }

    let mut filled_zone_area_mm2 = 0.0;
    for zone in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone"))
    {
        for polygon in zone
            .children()
            .iter()
            .filter(|item| item.head() == Some("filled_polygon"))
        {
            let layer = form_atom(polygon, "layer", 1)
                .or_else(|| form_atom(zone, "layer", 1))
                .ok_or_else(|| "filled zone polygon has no layer".to_string())?
                .to_string();
            let area = polygon_area(polygon)?;
            filled_zone_area_mm2 += area;
            layers.entry(layer).or_default().filled_zone_area_mm2 += area;
        }
    }

    let physical_centerline_length_mm = canonical_straight_track_length_mm + arc_track_length_mm;
    let primitive_copper_area_mm2 = centerline_weighted_track_area_mm2
        + layer_weighted_via_annulus_area_mm2
        + filled_zone_area_mm2;
    let mut limitations = vec![
        "area is a primitive sum: track end caps and overlaps between tracks, vias, and zones are not unioned"
            .into(),
    ];
    if arc_count > 0 {
        limitations.push(
            "arc centerlines are measured exactly but are not unioned against straight or other arc tracks"
                .into(),
        );
    }
    let mut layer_statistics = layers
        .into_iter()
        .map(|(layer, measured)| {
            let physical_centerline_length_mm =
                measured.canonical_straight_track_length_mm + measured.arc_track_length_mm;
            let primitive_copper_area_mm2 = measured.centerline_weighted_track_area_mm2
                + measured.via_annulus_area_mm2
                + measured.filled_zone_area_mm2;
            KiCadCopperLayerStatistics {
                layer,
                stored_straight_track_length_mm: measured.stored_straight_track_length_mm,
                canonical_straight_track_length_mm: measured.canonical_straight_track_length_mm,
                arc_track_length_mm: measured.arc_track_length_mm,
                physical_centerline_length_mm,
                centerline_weighted_track_area_mm2: measured.centerline_weighted_track_area_mm2,
                vias_spanning_layer: measured.vias_spanning_layer,
                via_annulus_area_mm2: measured.via_annulus_area_mm2,
                filled_zone_area_mm2: measured.filled_zone_area_mm2,
                primitive_copper_area_mm2,
            }
        })
        .collect::<Vec<_>>();
    layer_statistics.sort_by(|left, right| {
        let index = |layer: &str| {
            copper_layers
                .iter()
                .position(|candidate| candidate == layer)
                .unwrap_or(usize::MAX)
        };
        index(&left.layer)
            .cmp(&index(&right.layer))
            .then_with(|| left.layer.cmp(&right.layer))
    });
    let used_copper_layers = layer_statistics
        .iter()
        .filter(|layer| {
            layer.physical_centerline_length_mm > 0.0
                || layer.vias_spanning_layer > 0
                || layer.filled_zone_area_mm2 > 0.0
        })
        .count();
    Ok(KiCadPhysicalCopperStatistics {
        schema_version: 1,
        canonical_straight_track_length_mm,
        arc_track_length_mm,
        physical_centerline_length_mm,
        centerline_union_exact: arc_count == 0,
        overlapping_straight_track_length_mm: (stored_segment_length_mm
            - canonical_straight_track_length_mm)
            .max(0.0),
        centerline_weighted_track_area_mm2,
        via_projected_outer_area_mm2,
        via_drill_area_mm2,
        layer_weighted_via_annulus_area_mm2,
        filled_zone_area_mm2,
        primitive_copper_area_mm2,
        board_outline_area_mm2: board_outline_area(pcb)?,
        used_copper_layers,
        layers: layer_statistics,
        limitations,
    })
}

fn ordered_points(left: [f64; 2], right: [f64; 2]) -> ([f64; 2], [f64; 2]) {
    if left[0].total_cmp(&right[0]).is_lt()
        || (left[0] == right[0] && left[1].total_cmp(&right[1]).is_le())
    {
        (left, right)
    } else {
        (right, left)
    }
}

fn board_copper_layers(pcb: &Expr) -> Vec<String> {
    let mut layers = Vec::new();
    let push = |layers: &mut Vec<String>, layer: &str| {
        if layer.ends_with(".Cu") && !layers.iter().any(|existing| existing == layer) {
            layers.push(layer.to_string());
        }
    };
    if let Some(stack) = pcb.child("layers") {
        for entry in stack.children().iter().skip(1) {
            if let Some(layer) = entry.children().get(1).and_then(Expr::atom) {
                push(&mut layers, layer);
            }
        }
    }
    for item in pcb.children() {
        if matches!(item.head(), Some("segment" | "arc" | "zone"))
            && let Some(layer) = form_atom(item, "layer", 1)
        {
            push(&mut layers, layer);
        }
        if item.head() == Some("via")
            && let Some(via_layers) = item.child("layers")
        {
            for layer in via_layers.children().iter().skip(1).filter_map(Expr::atom) {
                if layer != "*.Cu" {
                    push(&mut layers, layer);
                }
            }
        }
    }
    layers
}

fn via_spanned_layers(item: &Expr, copper_layers: &[String]) -> Result<Vec<String>, String> {
    let declared = item
        .child("layers")
        .ok_or_else(|| "via has no layers".to_string())?
        .children()
        .iter()
        .skip(1)
        .filter_map(Expr::atom)
        .collect::<Vec<_>>();
    if declared.contains(&"*.Cu") {
        if copper_layers.is_empty() {
            return Err("through via uses *.Cu but the board has no copper stack".into());
        }
        return Ok(copper_layers.to_vec());
    }
    if declared.len() == 2 {
        let start = copper_layers
            .iter()
            .position(|layer| layer == declared[0])
            .ok_or_else(|| format!("via references unknown copper layer {:?}", declared[0]))?;
        let end = copper_layers
            .iter()
            .position(|layer| layer == declared[1])
            .ok_or_else(|| format!("via references unknown copper layer {:?}", declared[1]))?;
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        return Ok(copper_layers[start..=end].to_vec());
    }
    if declared.is_empty() {
        return Err("via has an empty layer span".into());
    }
    declared
        .into_iter()
        .map(|layer| {
            copper_layers
                .iter()
                .find(|candidate| candidate.as_str() == layer)
                .cloned()
                .ok_or_else(|| format!("via references unknown copper layer {layer:?}"))
        })
        .collect()
}

fn track_arc_length(item: &Expr) -> Result<f64, String> {
    let start = form_xy(item, "start")?;
    let middle = form_xy(item, "mid")?;
    let end = form_xy(item, "end")?;
    let denominator = 2.0
        * (start[0] * (middle[1] - end[1])
            + middle[0] * (end[1] - start[1])
            + end[0] * (start[1] - middle[1]));
    if !denominator.is_finite() {
        return Err("track arc has non-finite points".into());
    }
    if denominator.abs() <= 1.0e-12 {
        // A degenerate arc (its three points on a line, as some boards
        // carry) is the straight track KiCad draws for it.
        return Ok(distance_squared(start, middle).sqrt() + distance_squared(middle, end).sqrt());
    }
    let start_norm = start[0] * start[0] + start[1] * start[1];
    let middle_norm = middle[0] * middle[0] + middle[1] * middle[1];
    let end_norm = end[0] * end[0] + end[1] * end[1];
    let center = [
        (start_norm * (middle[1] - end[1])
            + middle_norm * (end[1] - start[1])
            + end_norm * (start[1] - middle[1]))
            / denominator,
        (start_norm * (end[0] - middle[0])
            + middle_norm * (start[0] - end[0])
            + end_norm * (middle[0] - start[0]))
            / denominator,
    ];
    let radius = distance_squared(start, center).sqrt();
    if !radius.is_finite() || radius <= 0.0 {
        return Err("arc has invalid radius".into());
    }
    let angle = |left: [f64; 2], right: [f64; 2]| {
        let left = [left[0] - center[0], left[1] - center[1]];
        let right = [right[0] - center[0], right[1] - center[1]];
        ((left[0] * right[0] + left[1] * right[1]) / (radius * radius))
            .clamp(-1.0, 1.0)
            .acos()
    };
    Ok(radius * (angle(start, middle) + angle(middle, end)))
}

fn polygon_area(polygon: &Expr) -> Result<f64, String> {
    if polygon.child("pts").is_none() {
        return Err("filled polygon has no pts".into());
    }
    let points = outline::pts_points(polygon)?;
    if points.len() < 3 {
        return Err("filled polygon has fewer than three points".into());
    }
    let twice_area = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(left, right)| left[0] * right[1] - right[0] * left[1])
        .sum::<f64>();
    Ok(twice_area.abs() / 2.0)
}

pub(crate) fn board_outline_area(pcb: &Expr) -> Result<Option<f64>, String> {
    Ok(board_outline(pcb)?.map(|outline| outline.area()))
}

#[cfg(test)]
pub(crate) fn rectangular_board_outline_area(pcb: &Expr) -> Result<Option<f64>, String> {
    let Some(outline) = board_outline(pcb)? else {
        return Ok(None);
    };
    let bounds_area =
        (outline.bounds[2] - outline.bounds[0]) * (outline.bounds[3] - outline.bounds[1]);
    Ok(((outline.area() - bounds_area).abs() <= 1.0e-6).then_some(bounds_area))
}

#[cfg(test)]
pub(crate) fn rectangular_board_bounds(pcb: &Expr) -> Result<Option<[f64; 4]>, String> {
    let Some(outline) = board_outline(pcb)? else {
        return Ok(None);
    };
    let bounds_area =
        (outline.bounds[2] - outline.bounds[0]) * (outline.bounds[3] - outline.bounds[1]);
    Ok(((outline.area() - bounds_area).abs() <= 1.0e-6).then_some(outline.bounds))
}

fn component_placement_records(pcb: &Expr) -> Result<Vec<(String, [f64; 3])>, String> {
    let mut records = Vec::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let declared_reference =
            footprint_reference(footprint).filter(|reference| !reference.is_empty());
        let footprint_name = footprint
            .children()
            .get(1)
            .and_then(Expr::atom)
            .unwrap_or("unnamed-footprint");
        let diagnostic_name = declared_reference.as_deref().unwrap_or(footprint_name);
        let at = footprint
            .child("at")
            .ok_or_else(|| format!("footprint {diagnostic_name:?} has no at form"))?;
        let fields = at.children();
        let number = |index: usize, label: &str| {
            fields
                .get(index)
                .and_then(Expr::atom)
                .ok_or_else(|| format!("footprint {diagnostic_name:?} has no {label}"))?
                .parse::<f64>()
                .map_err(|error| {
                    format!("footprint {diagnostic_name:?} has invalid {label}: {error}")
                })
        };
        let x = number(1, "x position")?;
        let y = number(2, "y position")?;
        let rotation = fields
            .get(3)
            .and_then(Expr::atom)
            .unwrap_or("0")
            .parse::<f64>()
            .map_err(|error| {
                format!("footprint {diagnostic_name:?} has invalid rotation: {error}")
            })?
            .rem_euclid(360.0);
        let reference = declared_reference
            .unwrap_or_else(|| format!("@anonymous:{footprint_name}:{x:.9}:{y:.9}:{rotation:.9}"));
        records.push((reference, [x, y, rotation]));
    }
    records.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(records)
}

fn component_placement_sha256(pcb: &Expr) -> Result<String, String> {
    let records = component_placement_records(pcb)?;
    let mut hash = Sha256::new();
    for (reference, pose) in records {
        hash.update((reference.len() as u64).to_le_bytes());
        hash.update(reference.as_bytes());
        for value in pose {
            let bits = if value == 0.0 {
                0.0_f64.to_bits()
            } else {
                value.to_bits()
            };
            hash.update(bits.to_le_bytes());
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn component_placement_quantized_sha256(pcb: &Expr) -> Result<String, String> {
    let records = component_placement_records(pcb)?;
    let mut hash = Sha256::new();
    for (reference, pose) in records {
        hash.update((reference.len() as u64).to_le_bytes());
        hash.update(reference.as_bytes());
        for value in pose {
            let quantized = (value * 10_000.0).round() as i64;
            hash.update(quantized.to_le_bytes());
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn copper_geometry_sha256(pcb: &Expr) -> String {
    let mut records: Vec<_> = pcb
        .children()
        .iter()
        .filter(|item| {
            matches!(item.head(), Some("segment" | "arc" | "via" | "zone"))
                || is_top_level_copper_graphic(item)
        })
        .filter_map(without_volatile_identity)
        .map(|item| encode(&item))
        .collect();
    records.sort();
    let mut hash = Sha256::new();
    for record in records {
        hash.update((record.len() as u64).to_le_bytes());
        hash.update(record.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

fn without_volatile_identity(expression: &Expr) -> Option<Expr> {
    if matches!(expression.head(), Some("uuid" | "tstamp")) {
        return None;
    }
    Some(match expression {
        Expr::Atom(atom) => Expr::Atom(atom.clone()),
        Expr::List(items) => {
            Expr::List(items.iter().filter_map(without_volatile_identity).collect())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_statistics_find_declared_and_name_addressed_nets() {
        let pcb = parse(
            r#"(kicad_pcb
                (net 0 "")
                (net 1 "SIGNAL")
                (footprint "test" (at 0 0)
                  (property "Reference" "J1")
                  (pad "1" smd rect (at 0 0) (size 1 1)
                    (layers "F.Cu") (net 1 "SIGNAL"))
                  (pad "2" smd rect (at 0 1) (size 1 1)
                    (layers "F.Cu") (net "unconnected-(J1-Pad2)")))
                (segment (start 0 0) (end 3 4) (width 0.25)
                  (layer "F.Cu") (net 1))
                (via (at 3 4) (size 0.7) (drill 0.3)
                  (layers "F.Cu" "B.Cu") (net "SIGNAL")))"#,
        )
        .unwrap();

        let statistics = board_statistics(&pcb).unwrap();
        assert_eq!(statistics.footprints, 1);
        assert_eq!(statistics.total_named_nets_including_no_connects, 2);
        assert_eq!(statistics.segments, 1);
        assert_eq!(statistics.vias, 1);
        assert_eq!(statistics.stored_segment_length_mm, 5.0);
        assert_eq!(
            statistics
                .physical_copper
                .canonical_straight_track_length_mm,
            5.0
        );
        assert_eq!(statistics.physical_copper.used_copper_layers, 2);
        assert!(statistics.physical_copper.centerline_union_exact);
        assert!(
            (statistics
                .physical_copper
                .centerline_weighted_track_area_mm2
                - 1.25)
                .abs()
                < 1.0e-12
        );
        assert_eq!(statistics.copper_geometry_sha256.len(), 64);
        assert_eq!(statistics.component_placement_sha256.len(), 64);
        assert_eq!(statistics.component_placement_100nm_sha256.len(), 64);
    }

    #[test]
    fn ordinary_numeric_kicad_nets_expose_names_and_routable_pad_counts() {
        let numeric = parse(r#"(segment (net 7 "SIGNAL"))"#).unwrap();
        assert_eq!(node_net(&numeric), Some("SIGNAL"));

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-connection-inventory-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let board = directory.join("board.kicad_pcb");
        fs::write(
            &board,
            r#"(kicad_pcb
                (net 0 "")
                (net 2 "SECOND")
                (net 1 "FIRST")
                (footprint "a" (at 10 20 90)
                  (property "Reference" "A")
                  (pad "1" thru_hole circle (at 1 0) (size 1 1)
                    (drill 0.5) (layers "*.Cu" "*.Mask") (net 2 "SECOND"))
                  (pad "2" thru_hole circle (at 1 0) (size 1 1)
                    (drill 0.5) (layers "*.Cu" "*.Mask") (net 2 "SECOND"))
                  (pad "3" thru_hole circle (at 0 0) (size 1 1)
                    (drill 0.5) (layers "*.Cu" "*.Mask") (net 1 "FIRST"))
                  (pad "4" thru_hole circle (at 2 0) (size 1 1)
                    (drill 0.5) (layers "*.Cu" "*.Mask") (net "THIRD")))
                (footprint "b" (at 30 40)
                  (property "Reference" "B")
                  (pad "1" thru_hole circle (at 0 0) (size 1 1)
                    (drill 0.5) (layers "*.Cu" "*.Mask") (net 2 "SECOND"))
                  (pad "2" thru_hole circle (at 3 0) (size 1 1)
                    (drill 0.5) (layers "*.Cu" "*.Mask") (net "THIRD"))))"#,
        )
        .unwrap();

        assert_eq!(
            inspect_kicad_routable_connections(&board).unwrap(),
            vec![
                KiCadConnectionSummary {
                    connection: "SECOND".into(),
                    distinct_pad_centers: 2,
                    electrical_terminal_count: None,
                },
                KiCadConnectionSummary {
                    connection: "THIRD".into(),
                    distinct_pad_centers: 2,
                    electrical_terminal_count: None,
                }
            ]
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn physical_copper_metrics_deduplicate_tracks_and_measure_layers() {
        let pcb = parse(
            r#"(kicad_pcb
                (layers
                  (0 "F.Cu" signal)
                  (31 "B.Cu" signal))
                (gr_rect (start 0 0) (end 10 5) (layer "Edge.Cuts"))
                (segment (start 0 0) (end 4 0) (width 0.2)
                  (layer "F.Cu") (net "SIGNAL"))
                (segment (start 0 0) (end 2 0) (width 0.4)
                  (layer "F.Cu") (net "SIGNAL"))
                (arc (start 1 0) (mid 0.7071067811865476 0.7071067811865476)
                  (end 0 1) (width 0.2) (layer "B.Cu") (net "ARC"))
                (via (at 2 0) (size 0.7) (drill 0.3)
                  (layers "F.Cu" "B.Cu") (net "SIGNAL"))
                (zone (net "PLANE") (layer "B.Cu")
                  (filled_polygon (layer "B.Cu")
                    (pts (xy 5 1) (xy 7 1) (xy 7 3) (xy 5 3)))))"#,
        )
        .unwrap();

        let statistics = board_statistics(&pcb).unwrap();
        let physical = &statistics.physical_copper;

        assert!((physical.canonical_straight_track_length_mm - 4.0).abs() < 1.0e-12);
        assert!((physical.overlapping_straight_track_length_mm - 2.0).abs() < 1.0e-12);
        assert!((physical.arc_track_length_mm - std::f64::consts::FRAC_PI_2).abs() < 1.0e-9);
        assert!(!physical.centerline_union_exact);
        assert!(
            (physical.centerline_weighted_track_area_mm2 - (1.2 + 0.1 * std::f64::consts::PI))
                .abs()
                < 1.0e-9
        );
        assert!((physical.filled_zone_area_mm2 - 4.0).abs() < 1.0e-12);
        assert_eq!(physical.board_outline_area_mm2, Some(50.0));
        assert_eq!(physical.used_copper_layers, 2);
        assert_eq!(physical.layers.len(), 2);
        assert_eq!(physical.layers[0].layer, "F.Cu");
        assert_eq!(physical.layers[0].vias_spanning_layer, 1);
        assert_eq!(physical.layers[1].layer, "B.Cu");
        assert_eq!(physical.layers[1].vias_spanning_layer, 1);
        assert!((physical.layers[1].filled_zone_area_mm2 - 4.0).abs() < 1.0e-12);
    }

    #[test]
    fn copper_geometry_digest_ignores_object_identity_and_order() {
        let first = parse(
            r#"(kicad_pcb
                (segment (start 0 0) (end 1 0) (width 0.25)
                  (layer "F.Cu") (net "SIGNAL") (uuid "first-segment"))
                (via (at 1 0) (size 0.7) (drill 0.3)
                  (layers "F.Cu" "B.Cu") (net "SIGNAL") (uuid "first-via")))"#,
        )
        .unwrap();
        let second = parse(
            r#"(kicad_pcb
                (via (at 1 0) (size 0.7) (drill 0.3)
                  (layers "F.Cu" "B.Cu") (net "SIGNAL") (uuid "other-via"))
                (segment (start 0 0) (end 1 0) (width 0.25)
                  (layer "F.Cu") (net "SIGNAL") (uuid "other-segment")))"#,
        )
        .unwrap();

        assert_eq!(
            board_statistics(&first).unwrap().copper_geometry_sha256,
            board_statistics(&second).unwrap().copper_geometry_sha256
        );
    }

    #[test]
    fn component_placement_digest_ignores_order_but_detects_motion() {
        let board = |a_x: f64, reverse: bool| {
            let a = format!(
                r#"(footprint "test" (at {a_x} 2 15)
                    (property "Reference" "A") (uuid "a"))"#
            );
            let b = r#"(footprint "test" (at 3 4 90)
                    (property "Reference" "B") (uuid "b"))"#;
            parse(&format!(
                "(kicad_pcb {} {})",
                if reverse { b } else { &a },
                if reverse { &a } else { b }
            ))
            .unwrap()
        };
        let first = board(1.0, false);
        let reordered = board(1.0, true);
        let moved = board(1.1, false);

        let digest = |pcb| board_statistics(pcb).unwrap().component_placement_sha256;
        assert_eq!(digest(&first), digest(&reordered));
        assert_ne!(digest(&first), digest(&moved));
    }
}
