// Copyright (C) 2026 Toit contributors.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

mod connection_rules;
pub use connection_rules::KiCadConnectionRoutingRules;
mod local_clearance;
mod native_report;
mod necks;
pub use necks::{NeckRepair, repair_kicad_necks};
mod check_work;
mod drc_cache;
mod kicad_config;
pub use drc_cache::{DrcCacheCounts, drc_cache_counts};
mod electrical_terminals;
#[cfg(test)]
mod geometry_queries;
mod roundrect_pad;
mod silkscreen;
mod verification_preview;
use native_report::{ReportKind, remove_if_present, run_kicad_report, validate_report};
pub use native_report::{erc_design_issues, is_erc_library_warning, is_library_metadata_warning};
pub use silkscreen::repair_kicad_silkscreen;
mod board_layout;
mod congestion_rank;
mod congestion_samples;
pub use board_layout::{
    KiCadBoardLayoutConfig, KiCadBoardLayoutMove, KiCadBoardLayoutResult, KiCadOutlineTrial,
    KiCadSwappable, layout_kicad_board,
};
pub use congestion_rank::{KiCadCongestionScore, score_congestion_sample, score_kicad_congestion};
pub use congestion_samples::{
    KiCadCongestionJob, KiCadCongestionSampleReport, KiCadCongestionVariant, KiCadPerturbation,
    export_congestion_samples,
};
mod board_placer;
mod canonical_order;
pub use board_placer::{
    KiCadBoardPlacerConfig, KiCadBoardPlacerResult, KiCadPlacedFootprint, place_kicad_board,
};
mod board_router;
mod verdict;
pub use verdict::{
    DrcTally, EXIT_COMPLETE, EXIT_CUT, EXIT_INCOMPLETE, EXIT_INTERNAL, EXIT_USAGE, KiCadReference,
    KiCadRunFacts, KiCadRunStatus, KiCadVerdict, decide, error_verdict, input_error, is_cosmetic,
    is_input_error, judge, kicad_drc_report, read_drc_tally, reference_findings, room_action,
    route_facts, split_findings, tally_drc, wall_cuts,
};
mod describe;
mod flip;
mod net_classes;
pub use net_classes::KiCadNetClassRequest;
mod quality;
pub use quality::{KiCadQualityReport, score_kicad_board};
mod cost;
pub use cost::{KiCadCost, KiCadFabQuote};
mod labels;
pub use labels::KiCadLabelReport;
mod fab_export;
pub use fab_export::{KiCadFabExport, export_kicad_fab};
mod board_footprints;
mod key_matrix;
pub use key_matrix::{KiCadStarterHalf, KiCadStarterReport};
mod netlist_import;
pub use describe::{KiCadBoardDescription, KiCadSwapCandidate, describe_kicad_board};
pub use netlist_import::{KiCadNetlistImport, import_kicad_netlist, import_kicad_netlist_from};
mod placement_constraints;
mod placement_room;
pub use placement_constraints::{
    KiCadConstraintStatus, KiCadConstraintsSource, KiCadPlacementConstraints,
    read_placement_constraints,
};
pub use placement_room::{KiCadRoom, TARGET_UTILIZATION, noted_room};
mod outline;
mod pin_swap;
pub use pin_swap::{
    KiCadPinChange, KiCadPinSwapConfig, KiCadPinSwapEstimate, KiCadPinSwapResult, KiCadPinSwapSpec,
    read_pin_swap_spec, swap_kicad_pins, swap_kicad_pins_in_place,
};
mod shrink;
pub use shrink::{KiCadShrinkReport, shrink_kicad_board};
mod project_rules;
pub use board_router::{
    KiCadBoardRouterConfig, KiCadBoardRouterNet, KiCadBoardRouterResult, KiCadBoardRouterViolation,
    KiCadPourMode, bench_kicad_search, dump_kicad_pour_map, route_kicad_board,
    write_kicad_board_without_tracks,
};
pub use project_rules::resolve_project_rules;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SupplementalSegment {
    pub connection: String,
    pub start: [f64; 2],
    pub end: [f64; 2],
    pub width: f64,
    pub layer: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SupplementalVia {
    pub connection: String,
    pub at: [f64; 2],
    pub size: f64,
    pub drill: f64,
    pub layers: [String; 2],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadRouteBranch {
    pub start_terminal: [f64; 2],
    pub finish_terminal: [f64; 2],
    pub cost: u32,
    pub expansions: u32,
    pub path: Vec<KiCadRoutePoint>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadRoutePoint {
    pub at: [f64; 2],
    pub layer: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Expr {
    Atom(String),
    List(Vec<Expr>),
}

impl Expr {
    fn head(&self) -> Option<&str> {
        match self {
            Self::List(items) => items.first().and_then(Self::atom),
            Self::Atom(_) => None,
        }
    }

    fn atom(&self) -> Option<&str> {
        match self {
            Self::Atom(atom) => Some(unquote(atom)),
            Self::List(_) => None,
        }
    }

    fn child(&self, head: &str) -> Option<&Expr> {
        match self {
            Self::List(items) => items.iter().find(|item| item.head() == Some(head)),
            Self::Atom(_) => None,
        }
    }

    fn children(&self) -> &[Expr] {
        match self {
            Self::List(items) => items,
            Self::Atom(_) => &[],
        }
    }
}

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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct KiCadCopperStripReport {
    pub schema_version: u32,
    pub removed_segments: usize,
    pub removed_arcs: usize,
    pub removed_vias: usize,
    pub removed_copper_zones: usize,
    #[serde(default)]
    pub removed_copper_graphics: usize,
    pub retained_rule_areas: usize,
    pub before: KiCadBoardStatistics,
    pub after: KiCadBoardStatistics,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KiCadConnectionSummary {
    pub connection: String,
    pub distinct_pad_centers: usize,
    /// Omitted when identical to geometric centers, including historical journals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub electrical_terminal_count: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KiCadFootprintLinkUpdate {
    #[serde(default)]
    pub schematic_path: String,
    pub reference: String,
    pub schematic_before: String,
    pub board_link: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KiCadSchematicLinkNormalization {
    pub schematic_path: String,
    pub before_sha256: String,
    pub after_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KiCadFootprintLinkNormalizationReport {
    pub schema_version: u32,
    pub board_id: String,
    pub updated_references: Vec<KiCadFootprintLinkUpdate>,
    pub schematic_before_sha256: String,
    pub schematic_after_sha256: String,
    pub board_sha256: String,
    pub copper_geometry_sha256: String,
    pub component_placement_sha256: String,
    #[serde(default)]
    pub schematic_files: Vec<KiCadSchematicLinkNormalization>,
}

/// Synchronizes footprint links throughout the referenced schematic hierarchy
/// to the embedded footprint links in an already materialized board. This is
/// intended for copied, pinned legacy projects whose library nicknames were
/// renamed after the board was laid out. It changes no board geometry,
/// placement, net, or copper decision.
pub fn normalize_kicad_footprint_links(
    directory: &Path,
    board_id: &str,
) -> Result<KiCadFootprintLinkNormalizationReport, String> {
    let board_path = directory.join(format!("{board_id}.kicad_pcb"));
    let schematic_path = directory.join(format!("{board_id}.kicad_sch"));
    let board_source = fs::read_to_string(&board_path)
        .map_err(|error| format!("failed to read {}: {error}", board_path.display()))?;
    let board = parse(&board_source)
        .map_err(|error| format!("failed to parse {}: {error}", board_path.display()))?;
    let board_statistics = board_statistics(&board)?;
    let mut board_links = BTreeMap::new();
    for footprint in board
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let Some(reference) = footprint_reference(footprint)
            .filter(|value| !value.is_empty() && !value.contains('*') && !value.starts_with('#'))
        else {
            continue;
        };
        let link = footprint
            .children()
            .get(1)
            .and_then(Expr::atom)
            .ok_or_else(|| format!("board footprint {reference:?} has no library link"))?
            .to_string();
        if let Some(previous) = board_links.insert(reference.clone(), link.clone())
            && previous != link
        {
            return Err(format!(
                "board has conflicting footprint links for reference {reference:?}"
            ));
        }
    }

    let project_root = fs::canonicalize(directory)
        .map_err(|error| format!("failed to resolve {}: {error}", directory.display()))?;
    let root_schematic = fs::canonicalize(&schematic_path)
        .map_err(|error| format!("failed to resolve {}: {error}", schematic_path.display()))?;
    let mut pending = VecDeque::from([root_schematic.clone()]);
    let mut visited = BTreeSet::new();
    let mut updates = BTreeMap::new();
    let mut schematic_files = Vec::new();
    while let Some(current_path) = pending.pop_front() {
        if !current_path.starts_with(&project_root) {
            return Err(format!(
                "referenced schematic {} escapes project {}",
                current_path.display(),
                project_root.display()
            ));
        }
        if !visited.insert(current_path.clone()) {
            continue;
        }
        let relative = current_path
            .strip_prefix(&project_root)
            .expect("project containment was checked")
            .to_string_lossy()
            .into_owned();
        let before_sha256 = file_sha256(&current_path)?;
        let schematic_source = fs::read_to_string(&current_path)
            .map_err(|error| format!("failed to read {}: {error}", current_path.display()))?;
        let mut schematic = parse(&schematic_source)
            .map_err(|error| format!("failed to parse {}: {error}", current_path.display()))?;
        let Expr::List(items) = &mut schematic else {
            return Err(format!("{} root is not a list", current_path.display()));
        };
        if root_head(items) != Some("kicad_sch") {
            return Err(format!(
                "{} is not a kicad_sch document",
                current_path.display()
            ));
        }
        for sheet_file in items
            .iter()
            .filter(|item| item.head() == Some("sheet"))
            .filter_map(|sheet| schematic_property(sheet, "Sheetfile"))
        {
            let child = Path::new(&sheet_file);
            if child.is_absolute() {
                return Err(format!(
                    "referenced schematic path {sheet_file:?} is absolute"
                ));
            }
            let child_path =
                fs::canonicalize(current_path.parent().unwrap().join(child)).map_err(|error| {
                    format!(
                        "failed to resolve child schematic {sheet_file:?} from {}: {error}",
                        current_path.display()
                    )
                })?;
            if !child_path.starts_with(&project_root) {
                return Err(format!(
                    "referenced schematic {} escapes project {}",
                    child_path.display(),
                    project_root.display()
                ));
            }
            pending.push_back(child_path);
        }
        let mut changed = false;
        for symbol in items
            .iter_mut()
            .filter(|item| item.head() == Some("symbol"))
        {
            let Some(reference) = schematic_property(symbol, "Reference") else {
                continue;
            };
            let Some(board_link) = board_links.get(&reference) else {
                continue;
            };
            let Some(before) = schematic_property(symbol, "Footprint") else {
                continue;
            };
            if before == *board_link {
                continue;
            }
            let Expr::List(symbol_items) = symbol else {
                unreachable!("symbol form must be a list")
            };
            let property = symbol_items
                .iter_mut()
                .find(|item| {
                    item.head() == Some("property")
                        && item.children().get(1).and_then(Expr::atom) == Some("Footprint")
                })
                .expect("Footprint property was read above");
            let Expr::List(property_items) = property else {
                unreachable!("property form must be a list")
            };
            property_items[2] = Expr::Atom(quote(board_link));
            changed = true;
            updates.insert(
                (relative.clone(), reference.clone()),
                KiCadFootprintLinkUpdate {
                    schematic_path: relative.clone(),
                    reference,
                    schematic_before: before,
                    board_link: board_link.clone(),
                },
            );
        }
        if changed {
            fs::write(&current_path, format!("{}\n", encode(&schematic))).map_err(|error| {
                format!(
                    "failed to write normalized {}: {error}",
                    current_path.display()
                )
            })?;
        }
        schematic_files.push(KiCadSchematicLinkNormalization {
            schematic_path: relative,
            before_sha256,
            after_sha256: file_sha256(&current_path)?,
        });
    }
    let root_audit = schematic_files
        .iter()
        .find(|audit| root_schematic == project_root.join(&audit.schematic_path))
        .expect("root schematic is always audited");
    Ok(KiCadFootprintLinkNormalizationReport {
        schema_version: 2,
        board_id: board_id.to_string(),
        updated_references: updates.into_values().collect(),
        schematic_before_sha256: root_audit.before_sha256.clone(),
        schematic_after_sha256: root_audit.after_sha256.clone(),
        board_sha256: file_sha256(&board_path)?,
        copper_geometry_sha256: board_statistics.copper_geometry_sha256,
        component_placement_sha256: board_statistics.component_placement_sha256,
        schematic_files,
    })
}

fn schematic_property(node: &Expr, name: &str) -> Option<String> {
    node.children().iter().find_map(|item| {
        (item.head() == Some("property") && item.children().get(1)?.atom()? == name)
            .then(|| item.children().get(2)?.atom().map(str::to_owned))
            .flatten()
    })
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

/// Rewrites a KiCad board without inherited routed copper while retaining the
/// placement, pads, net declarations, board outline, non-copper graphics, and
/// rule-area keepouts. This is the cold-start adapter for reproducible routing
/// studies; it removes copper pours and top-level copper graphics because both
/// are inherited layout decisions.
pub fn write_kicad_board_without_copper(
    source: &Path,
    destination: &Path,
) -> Result<KiCadCopperStripReport, String> {
    let source_text = fs::read_to_string(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?;
    let mut pcb = parse(&source_text)
        .map_err(|error| format!("failed to parse {}: {error}", source.display()))?;
    if pcb.head() != Some("kicad_pcb") {
        return Err(format!("{} is not a kicad_pcb document", source.display()));
    }
    let before = board_statistics(&pcb)?;
    let Expr::List(items) = &mut pcb else {
        unreachable!("kicad_pcb root must be a list")
    };
    items.retain(|item| match item.head() {
        Some("segment" | "arc" | "via") => false,
        Some("zone") => is_rule_area(item),
        _ => !is_top_level_copper_graphic(item),
    });
    let after = board_statistics(&pcb)?;
    if before.component_placement_sha256 != after.component_placement_sha256
        || before.component_placement_100nm_sha256 != after.component_placement_100nm_sha256
    {
        return Err("cold-board rewrite changed component placement".into());
    }
    if after.segments != 0
        || after.arcs != 0
        || after.vias != 0
        || after.copper_zones != 0
        || after.copper_graphics != 0
    {
        return Err("cold-board rewrite retained routed copper".into());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    fs::write(destination, format!("{}\n", encode(&pcb)))
        .map_err(|error| format!("failed to write {}: {error}", destination.display()))?;
    Ok(KiCadCopperStripReport {
        schema_version: 2,
        removed_segments: before.segments,
        removed_arcs: before.arcs,
        removed_vias: before.vias,
        removed_copper_zones: before.copper_zones,
        removed_copper_graphics: before.copper_graphics,
        retained_rule_areas: after.rule_areas,
        before,
        after,
    })
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

fn board_statistics(pcb: &Expr) -> Result<KiCadBoardStatistics, String> {
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

fn is_rule_area(zone: &Expr) -> bool {
    zone.head() == Some("zone") && zone.child("keepout").is_some()
}

fn is_top_level_copper_graphic(item: &Expr) -> bool {
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

fn board_outline_area(pcb: &Expr) -> Result<Option<f64>, String> {
    Ok(board_outline(pcb)?.map(|outline| outline.area()))
}

#[cfg(test)]
fn rectangular_board_outline_area(pcb: &Expr) -> Result<Option<f64>, String> {
    let Some(outline) = board_outline(pcb)? else {
        return Ok(None);
    };
    let bounds_area =
        (outline.bounds[2] - outline.bounds[0]) * (outline.bounds[3] - outline.bounds[1]);
    Ok(((outline.area() - bounds_area).abs() <= 1.0e-6).then_some(bounds_area))
}

#[cfg(test)]
fn rectangular_board_bounds(pcb: &Expr) -> Result<Option<[f64; 4]>, String> {
    let Some(outline) = board_outline(pcb)? else {
        return Ok(None);
    };
    let bounds_area =
        (outline.bounds[2] - outline.bounds[0]) * (outline.bounds[3] - outline.bounds[1]);
    Ok(((outline.area() - bounds_area).abs() <= 1.0e-6).then_some(outline.bounds))
}

fn board_outline(pcb: &Expr) -> Result<Option<BoardOutline>, String> {
    let edge_shapes = pcb
        .children()
        .iter()
        .filter(|item| {
            matches!(
                item.head(),
                Some("gr_rect" | "gr_line" | "gr_arc" | "gr_poly" | "gr_curve")
            ) && form_atom(item, "layer", 1) == Some("Edge.Cuts")
        })
        .collect::<Vec<_>>();
    if edge_shapes.len() == 1 && edge_shapes[0].head() == Some("gr_rect") {
        let start = form_xy(edge_shapes[0], "start")?;
        let end = form_xy(edge_shapes[0], "end")?;
        return Ok(Some(BoardOutline::rectangle([
            start[0].min(end[0]),
            start[1].min(end[1]),
            start[0].max(end[0]),
            start[1].max(end[1]),
        ])?));
    }
    if edge_shapes.len() < 4
        || edge_shapes
            .iter()
            .any(|shape| shape.head() != Some("gr_line"))
    {
        return Ok(None);
    }
    let mut segments = Vec::with_capacity(edge_shapes.len());
    for shape in edge_shapes {
        let start = form_xy(shape, "start")?;
        let end = form_xy(shape, "end")?;
        if distance_squared(start, end) <= 1.0e-18 {
            return Ok(None);
        }
        segments.push((start, end));
    }
    let point = |coordinate: [f64; 2]| {
        Point(
            (coordinate[0] * 1_000_000.0).round() as i64,
            (coordinate[1] * 1_000_000.0).round() as i64,
        )
    };
    let segments = segments
        .into_iter()
        .map(|(start, end)| (point(start), point(end)))
        .collect::<Vec<_>>();
    let mut adjacency: BTreeMap<Point, Vec<Point>> = BTreeMap::new();
    for (start, end) in segments {
        adjacency.entry(start).or_default().push(end);
        adjacency.entry(end).or_default().push(start);
    }
    if adjacency.len() < 3 || adjacency.values().any(|neighbors| neighbors.len() != 2) {
        return Ok(None);
    }
    let start = *adjacency
        .keys()
        .next()
        .expect("non-empty outline adjacency");
    let mut previous = None;
    let mut current = start;
    let mut points = Vec::with_capacity(adjacency.len());
    loop {
        points.push([
            current.0 as f64 / 1_000_000.0,
            current.1 as f64 / 1_000_000.0,
        ]);
        let neighbors = &adjacency[&current];
        let next = if previous == Some(neighbors[0]) {
            neighbors[1]
        } else {
            neighbors[0]
        };
        previous = Some(current);
        current = next;
        if current == start {
            break;
        }
        if points.len() >= adjacency.len() {
            return Ok(None);
        }
    }
    if points.len() != adjacency.len() {
        return Ok(None);
    }
    Ok(Some(BoardOutline::new(points)?))
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

fn collect_named_nets(root: &Expr, named_nets: &mut BTreeSet<String>) {
    if root.head() == Some("net")
        && let Some(name) = net_name_from_form(root)
        && !name.is_empty()
        && name.parse::<u32>().is_err()
    {
        named_nets.insert(normalize_net(name).to_string());
    }
    for child in root.children() {
        collect_named_nets(child, named_nets);
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct VerificationReport {
    pub board_id: String,
    pub complete: bool,
    pub erc_violations: usize,
    pub drc_design_violations: usize,
    pub schematic_parity_issues: usize,
    pub selected_net_unconnected_items: usize,
    pub intentional_no_connect_groups: usize,
    pub library_metadata_warnings: usize,
    /// ERC's library warnings by type (`is_erc_library_warning`), not in
    /// `erc_violations`.
    #[serde(default)]
    pub erc_library_warnings: BTreeMap<String, usize>,
}

pub const KICAD_VERIFICATION_CACHE_ENV: &str = "PCB_MAKER_KICAD_VERIFICATION_CACHE";
pub const KICAD_VERIFICATION_CACHE_EVENTS_ENV: &str = "PCB_MAKER_KICAD_VERIFICATION_CACHE_EVENTS";
const KICAD_VERIFICATION_CACHE_SCHEMA_VERSION: u32 = 1;
const KICAD_VERIFICATION_POLICY: &str = "verify-materialized-rung-v2:erc-without-library-warnings:drc-all:schematic-parity:intentional-no-connect-groups";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadVerificationCacheStatus {
    Hit,
    Miss,
    Bypassed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadVerificationCacheTrace {
    pub schema_version: u32,
    pub status: KiCadVerificationCacheStatus,
    pub key: Option<String>,
    pub input_sha256: Option<String>,
    pub kicad_cli_version: String,
    pub verifier_sha256: String,
    pub policy: String,
    pub refill_zones: bool,
    pub cache_root: PathBuf,
    pub cache_entry: Option<PathBuf>,
    pub elapsed_micros: u64,
    pub detail: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct KiCadVerificationCacheEntry {
    schema_version: u32,
    key: String,
    input_sha256: String,
    kicad_cli_version: String,
    verifier_sha256: String,
    policy: String,
    refill_zones: bool,
    erc_sha256: String,
    drc_sha256: String,
    report: VerificationReport,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KiCadLayerRenderConfig {
    pub width: u32,
    pub height: u32,
    pub quality: String,
    pub background: String,
    /// Include component models and markings. Comparisons default to hiding
    /// these so they cannot obscure copper; pads and board geometry remain.
    pub show_components: bool,
    /// Add outline-only silkscreen geometry to a disposable render input so
    /// native 3D renders expose non-physical rule-area constraints.
    pub show_rule_areas: bool,
    /// Label and outline copper zones on the disposable render input. This is
    /// opt-in because plane-heavy production boards would otherwise be noisy.
    pub show_copper_zones: bool,
}

impl Default for KiCadLayerRenderConfig {
    fn default() -> Self {
        Self {
            width: 1200,
            height: 750,
            quality: "basic".into(),
            background: "opaque".into(),
            show_components: false,
            show_rule_areas: true,
            show_copper_zones: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadLayerRenderReport {
    pub front: PathBuf,
    pub back: PathBuf,
    pub config: KiCadLayerRenderConfig,
    pub rule_area_overlays: usize,
    pub copper_zone_overlays: usize,
}

/// Renders deterministic top and bottom journal images through KiCad's native
/// board renderer. These images are inspection evidence, not a replacement
/// for ERC, DRC, parity, or connectivity verification.
pub fn render_board_layers(
    pcb_path: &Path,
    output_prefix: &Path,
    config: &KiCadLayerRenderConfig,
) -> Result<KiCadLayerRenderReport, String> {
    validate_layer_render_config(config)?;
    if !pcb_path.is_file() {
        return Err(format!("KiCad board {} does not exist", pcb_path.display()));
    }
    if let Some(parent) = output_prefix
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create render directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let front = PathBuf::from(format!("{}-front.png", output_prefix.display()));
    let back = PathBuf::from(format!("{}-back.png", output_prefix.display()));
    let render_input = PathBuf::from(format!(
        "{}-inspection-{}-{}.kicad_pcb",
        output_prefix.display(),
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("system clock precedes Unix epoch: {error}"))?
            .as_nanos()
    ));
    let overlay_counts =
        if !config.show_components || config.show_rule_areas || config.show_copper_zones {
            prepare_layer_render_board(pcb_path, &render_input, config)?
        } else {
            KiCadLayerRenderOverlayCounts::default()
        };
    let overlay_count = overlay_counts.rule_areas + overlay_counts.copper_zones;
    let disposable_input = !config.show_components || overlay_count != 0;
    let rendered_pcb = if !disposable_input {
        pcb_path
    } else {
        render_input.as_path()
    };
    let render_result = run_kicad_render(rendered_pcb, &front, "top", config)
        .and_then(|()| run_kicad_render(rendered_pcb, &back, "bottom", config));
    if disposable_input {
        let cleanup_result = fs::remove_file(&render_input).map_err(|error| {
            format!(
                "failed to remove disposable render input {}: {error}",
                render_input.display()
            )
        });
        render_result?;
        cleanup_result?;
    } else {
        render_result?;
    }
    Ok(KiCadLayerRenderReport {
        front,
        back,
        config: config.clone(),
        rule_area_overlays: overlay_counts.rule_areas,
        copper_zone_overlays: overlay_counts.copper_zones,
    })
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct KiCadLayerRenderOverlayCounts {
    rule_areas: usize,
    copper_zones: usize,
}

fn prepare_layer_render_board(
    source: &Path,
    destination: &Path,
    config: &KiCadLayerRenderConfig,
) -> Result<KiCadLayerRenderOverlayCounts, String> {
    let source_text = fs::read_to_string(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?;
    let mut pcb = parse(&source_text)
        .map_err(|error| format!("failed to parse {}: {error}", source.display()))?;
    if pcb.head() != Some("kicad_pcb") {
        return Err(format!("{} is not a kicad_pcb document", source.display()));
    }
    let mut overlays = Vec::new();
    let mut counts = KiCadLayerRenderOverlayCounts::default();
    for (zone_index, zone) in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone"))
        .enumerate()
    {
        let rule_area = is_rule_area(zone);
        if (rule_area && !config.show_rule_areas) || (!rule_area && !config.show_copper_zones) {
            continue;
        }
        let Ok(layer_mask) = rule_area_layer_mask(zone) else {
            continue;
        };
        let Ok(points) = rule_area_polygon_points(zone) else {
            continue;
        };
        for (layer_index, enabled) in layer_mask.into_iter().enumerate() {
            if enabled {
                let layer = if layer_index == 0 {
                    "F.SilkS"
                } else {
                    "B.SilkS"
                };
                overlays.push(zone_render_outline_expr(
                    &points, layer, zone_index, rule_area,
                ));
                if rule_area {
                    counts.rule_areas += 1;
                } else {
                    overlays.push(copper_zone_render_label_expr(
                        &points,
                        layer,
                        zone_index,
                        node_net(zone)
                            .or_else(|| form_atom(zone, "net_name", 1))
                            .unwrap_or("unnamed"),
                    ));
                    counts.copper_zones += 1;
                }
            }
        }
    }
    if overlays.is_empty() && config.show_components {
        return Ok(counts);
    }
    let Expr::List(items) = &mut pcb else {
        unreachable!("kicad_pcb root must be a list")
    };
    if !config.show_components {
        // Only the disposable visualization copy loses graphics. Footprints,
        // pads, copper, holes, outline and source project stay intact.
        let is_marking = |item: &Expr| {
            matches!(
                form_atom(item, "layer", 1),
                Some("F.SilkS" | "B.SilkS" | "F.Fab" | "B.Fab")
            )
        };
        items.retain(|item| !is_marking(item));
        for item in items.iter_mut() {
            if matches!(item.head(), Some("footprint" | "module")) {
                let Expr::List(children) = item else {
                    unreachable!()
                };
                children.retain(|child| child.head() != Some("model") && !is_marking(child));
            }
        }
    }
    items.extend(overlays);
    fs::write(destination, format!("{}\n", encode(&pcb))).map_err(|error| {
        format!(
            "failed to write disposable render input {}: {error}",
            destination.display()
        )
    })?;
    Ok(counts)
}

fn zone_render_outline_expr(
    points: &[[f64; 2]],
    layer: &str,
    index: usize,
    rule_area: bool,
) -> Expr {
    let mut point_items = vec![Expr::Atom("pts".into())];
    point_items.extend(
        points
            .iter()
            .copied()
            .map(|point| coordinate_form("xy", point)),
    );
    Expr::List(vec![
        Expr::Atom("gr_poly".into()),
        Expr::List(point_items),
        Expr::List(vec![
            Expr::Atom("stroke".into()),
            Expr::List(vec![Expr::Atom("width".into()), Expr::Atom("0.4".into())]),
            Expr::List(vec![
                Expr::Atom("type".into()),
                Expr::Atom(if rule_area { "dash" } else { "solid" }.into()),
            ]),
        ]),
        Expr::List(vec![Expr::Atom("fill".into()), Expr::Atom("no".into())]),
        string_form("layer", layer),
        Expr::List(vec![
            Expr::Atom("uuid".into()),
            Expr::Atom(quote(&deterministic_uuid_for(&format!(
                "pcb-maker:zone-render-outline:{rule_area}:{layer}:{index}:{points:?}"
            )))),
        ]),
    ])
}

fn copper_zone_render_label_expr(
    points: &[[f64; 2]],
    layer: &str,
    index: usize,
    net_name: &str,
) -> Expr {
    let centroid = points.iter().fold([0.0, 0.0], |sum, point| {
        [sum[0] + point[0], sum[1] + point[1]]
    });
    let centroid = [
        centroid[0] / points.len() as f64,
        centroid[1] / points.len() as f64,
    ];
    let mut effects = vec![
        Expr::Atom("effects".into()),
        Expr::List(vec![
            Expr::Atom("font".into()),
            Expr::List(vec![
                Expr::Atom("size".into()),
                Expr::Atom("1.2".into()),
                Expr::Atom("1.2".into()),
            ]),
            Expr::List(vec![
                Expr::Atom("thickness".into()),
                Expr::Atom("0.2".into()),
            ]),
        ]),
    ];
    if layer == "B.SilkS" {
        effects.push(Expr::List(vec![
            Expr::Atom("justify".into()),
            Expr::Atom("mirror".into()),
        ]));
    }
    Expr::List(vec![
        Expr::Atom("gr_text".into()),
        Expr::Atom(quote(&format!("COPPER ZONE {net_name}"))),
        coordinate_form("at", centroid),
        string_form("layer", layer),
        Expr::List(vec![
            Expr::Atom("uuid".into()),
            Expr::Atom(quote(&deterministic_uuid_for(&format!(
                "pcb-maker:copper-zone-render-label:{layer}:{index}:{net_name}:{centroid:?}"
            )))),
        ]),
        Expr::List(effects),
    ])
}

fn validate_layer_render_config(config: &KiCadLayerRenderConfig) -> Result<(), String> {
    if !(1..=8192).contains(&config.width) || !(1..=8192).contains(&config.height) {
        return Err("KiCad render width and height must be within 1..=8192".into());
    }
    if !matches!(
        config.quality.as_str(),
        "basic" | "high" | "user" | "job_settings"
    ) {
        return Err(format!(
            "unsupported KiCad render quality {:?}",
            config.quality
        ));
    }
    if !matches!(
        config.background.as_str(),
        "default" | "transparent" | "opaque"
    ) {
        return Err(format!(
            "unsupported KiCad render background {:?}",
            config.background
        ));
    }
    Ok(())
}

fn run_kicad_render(
    pcb_path: &Path,
    output_path: &Path,
    side: &str,
    config: &KiCadLayerRenderConfig,
) -> Result<(), String> {
    let result = Command::new("kicad-cli")
        .args([
            "pcb",
            "render",
            "--output",
            &output_path.to_string_lossy(),
            "--width",
            &config.width.to_string(),
            "--height",
            &config.height.to_string(),
            "--side",
            side,
            "--background",
            &config.background,
            "--quality",
            &config.quality,
            &pcb_path.to_string_lossy(),
        ])
        .output()
        .map_err(|error| format!("failed to run kicad-cli PCB renderer: {error}"))?;
    if !result.status.success() || !output_path.is_file() {
        return Err(format!(
            "kicad-cli failed to render {side} image {} with {}: {}{}",
            output_path.display(),
            result.status,
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    Ok(())
}

pub fn verify_materialized_rung(
    directory: &Path,
    board_id: &str,
) -> Result<VerificationReport, String> {
    verification_preview::capture(directory, board_id, || {
        remove_if_present(&directory.join("verification.json"))?;
        let pcb_path = directory.join(format!("{board_id}.kicad_pcb"));
        let pcb_source = fs::read_to_string(&pcb_path)
            .map_err(|error| format!("failed to read {}: {error}", pcb_path.display()))?;
        let pcb = parse(&pcb_source)?;
        verify_materialized_rung_with_options(directory, board_id, count_head(&pcb, "zone") > 0)
    })
}

fn verify_materialized_rung_with_options(
    directory: &Path,
    board_id: &str,
    refill_zones: bool,
) -> Result<VerificationReport, String> {
    if !directory.join(format!("{board_id}.kicad_sch")).exists() {
        return verify_board_only(directory, board_id, refill_zones);
    }
    if let Some(cache_root) = env::var_os(KICAD_VERIFICATION_CACHE_ENV) {
        verify_materialized_rung_cached(directory, board_id, refill_zones, Path::new(&cache_root))
    } else {
        verify_materialized_rung_uncached(directory, board_id, refill_zones)
    }
}

fn verify_materialized_rung_uncached(
    directory: &Path,
    board_id: &str,
    refill_zones: bool,
) -> Result<VerificationReport, String> {
    remove_if_present(&directory.join("verification.json"))?;
    let schematic = directory.join(format!("{board_id}.kicad_sch"));
    let pcb = directory.join(format!("{board_id}.kicad_pcb"));
    let erc_path = directory.join("erc.json");
    let drc_path = directory.join("drc.json");
    run_kicad_report(
        &[
            "sch",
            "erc",
            "--format",
            "json",
            "--severity-all",
            "--output",
        ],
        &erc_path,
        &schematic,
    )?;
    let mut drc_arguments = vec![
        "pcb",
        "drc",
        "--format",
        "json",
        "--severity-all",
        "--schematic-parity",
    ];
    if refill_zones {
        drc_arguments.push("--refill-zones");
    }
    drc_arguments.push("--output");
    run_kicad_report(&drc_arguments, &drc_path, &pcb)?;

    let erc: serde_json::Value = read_json(&erc_path)?;
    let drc: serde_json::Value = read_json(&drc_path)?;
    write_verification_report(directory, board_id, &erc, &drc)
}

/// A project without a schematic (a bare board, as in board-only benchmark
/// sets): DRC without schematic parity, and an empty ERC report in its place.
fn verify_board_only(
    directory: &Path,
    board_id: &str,
    refill_zones: bool,
) -> Result<VerificationReport, String> {
    remove_if_present(&directory.join("verification.json"))?;
    let pcb = directory.join(format!("{board_id}.kicad_pcb"));
    let drc_path = directory.join("drc.json");
    let mut drc_arguments = vec!["pcb", "drc", "--format", "json", "--severity-all"];
    if refill_zones {
        drc_arguments.push("--refill-zones");
    }
    drc_arguments.push("--output");
    run_kicad_report(&drc_arguments, &drc_path, &pcb)?;
    let drc: serde_json::Value = read_json(&drc_path)?;
    let erc = serde_json::json!({
        "source": format!("{board_id}.kicad_sch"),
        "date": drc["date"].as_str().unwrap_or("unknown"),
        "kicad_version": "none: board-only project, no schematic",
        "coordinate_units": "mm",
        "sheets": [],
    });
    write_typed_json(&directory.join("erc.json"), &erc)?;
    write_verification_report(directory, board_id, &erc, &drc)
}

fn write_verification_report(
    directory: &Path,
    board_id: &str,
    erc: &serde_json::Value,
    drc: &serde_json::Value,
) -> Result<VerificationReport, String> {
    remove_if_present(&directory.join("verification.json"))?;
    let report = assess_verification_reports(board_id, erc, drc)?;
    write_typed_json(&directory.join("verification.json"), &report)?;
    Ok(report)
}

fn assess_verification_reports(
    board_id: &str,
    erc: &serde_json::Value,
    drc: &serde_json::Value,
) -> Result<VerificationReport, String> {
    validate_report(
        erc,
        ReportKind::Erc,
        Path::new(&format!("{board_id}.kicad_sch")),
    )?;
    validate_report(
        drc,
        ReportKind::Drc,
        Path::new(&format!("{board_id}.kicad_pcb")),
    )?;
    let erc_violations = erc_design_issues(erc).len();
    let mut erc_library_warnings = BTreeMap::<String, usize>::new();
    for finding in erc["sheets"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|sheet| sheet["violations"].as_array().into_iter().flatten())
        .filter(|finding| is_erc_library_warning(finding))
    {
        *erc_library_warnings
            .entry(finding["type"].as_str().unwrap_or("").to_string())
            .or_default() += 1;
    }
    let schematic_parity_issues = drc["schematic_parity"].as_array().map_or(0, Vec::len);
    let mut drc_design_violations = 0;
    let mut library_metadata_warnings = 0;
    for violation in drc["violations"].as_array().into_iter().flatten() {
        if is_library_metadata_warning(violation) {
            library_metadata_warnings += 1;
        } else {
            drc_design_violations += 1;
        }
    }
    let mut intentional_no_connect_groups = 0;
    let mut selected_net_unconnected_items = 0;
    for violation in drc["unconnected_items"].as_array().into_iter().flatten() {
        if is_intentional_no_connect_group(violation) {
            intentional_no_connect_groups += 1;
        } else {
            selected_net_unconnected_items += 1;
        }
    }
    let complete = erc_violations == 0
        && drc_design_violations == 0
        && schematic_parity_issues == 0
        && selected_net_unconnected_items == 0;
    let report = VerificationReport {
        board_id: board_id.to_string(),
        complete,
        erc_violations,
        drc_design_violations,
        schematic_parity_issues,
        selected_net_unconnected_items,
        intentional_no_connect_groups,
        library_metadata_warnings,
        erc_library_warnings,
    };
    Ok(report)
}

enum KiCadCacheInput {
    Cacheable(String),
    Bypassed(String),
}

fn verify_materialized_rung_cached(
    directory: &Path,
    board_id: &str,
    refill_zones: bool,
    cache_root: &Path,
) -> Result<VerificationReport, String> {
    remove_if_present(&directory.join("verification.json"))?;
    let started = Instant::now();
    let kicad_cli_version = kicad_cli_version()?;
    let verifier_sha256 = kicad_verifier_implementation_sha256();
    let input_sha256 = match kicad_cache_input_sha256(directory)? {
        KiCadCacheInput::Cacheable(hash) => hash,
        KiCadCacheInput::Bypassed(detail) => {
            let report = verify_materialized_rung_uncached(directory, board_id, refill_zones)?;
            write_kicad_cache_trace(
                directory,
                &KiCadVerificationCacheTrace {
                    schema_version: KICAD_VERIFICATION_CACHE_SCHEMA_VERSION,
                    status: KiCadVerificationCacheStatus::Bypassed,
                    key: None,
                    input_sha256: None,
                    kicad_cli_version,
                    verifier_sha256,
                    policy: KICAD_VERIFICATION_POLICY.into(),
                    refill_zones,
                    cache_root: cache_root.to_path_buf(),
                    cache_entry: None,
                    elapsed_micros: started.elapsed().as_micros() as u64,
                    detail,
                },
            )?;
            return Ok(report);
        }
    };
    let key = {
        let mut hash = Sha256::new();
        hash.update(KICAD_VERIFICATION_CACHE_SCHEMA_VERSION.to_le_bytes());
        hash.update(KICAD_VERIFICATION_POLICY.as_bytes());
        hash.update([u8::from(refill_zones)]);
        hash.update((kicad_cli_version.len() as u64).to_le_bytes());
        hash.update(kicad_cli_version.as_bytes());
        hash.update(verifier_sha256.as_bytes());
        hash.update(input_sha256.as_bytes());
        format!("{:x}", hash.finalize())
    };
    fs::create_dir_all(cache_root).map_err(|error| {
        format!(
            "failed to create KiCad verification cache {}: {error}",
            cache_root.display()
        )
    })?;
    let cache_entry = cache_root.join(&key);
    if cache_entry.is_dir() {
        let entry: KiCadVerificationCacheEntry =
            read_typed_json(&cache_entry.join("cache-entry.json"))?;
        if entry.schema_version != KICAD_VERIFICATION_CACHE_SCHEMA_VERSION
            || entry.key != key
            || entry.input_sha256 != input_sha256
            || entry.kicad_cli_version != kicad_cli_version
            || entry.verifier_sha256 != verifier_sha256
            || entry.policy != KICAD_VERIFICATION_POLICY
            || entry.refill_zones != refill_zones
            || entry.report.board_id != board_id
        {
            return Err(format!(
                "KiCad verification cache entry {} does not match its key",
                cache_entry.display()
            ));
        }
        let cached_erc = cache_entry.join("erc.json");
        let cached_drc = cache_entry.join("drc.json");
        if file_sha256(&cached_erc)? != entry.erc_sha256
            || file_sha256(&cached_drc)? != entry.drc_sha256
        {
            return Err(format!(
                "KiCad verification cache entry {} has corrupted reports",
                cache_entry.display()
            ));
        }
        let assessed = assess_verification_reports(
            board_id,
            &read_json(&cached_erc)?,
            &read_json(&cached_drc)?,
        )?;
        if assessed != entry.report {
            return Err(format!(
                "KiCad verification cache entry {} has an inconsistent assessment",
                cache_entry.display()
            ));
        }
        fs::copy(&cached_erc, directory.join("erc.json"))
            .map_err(|error| format!("failed to restore cached ERC report: {error}"))?;
        fs::copy(&cached_drc, directory.join("drc.json"))
            .map_err(|error| format!("failed to restore cached DRC report: {error}"))?;
        write_typed_json(&directory.join("verification.json"), &entry.report)?;
        write_kicad_cache_trace(
            directory,
            &KiCadVerificationCacheTrace {
                schema_version: KICAD_VERIFICATION_CACHE_SCHEMA_VERSION,
                status: KiCadVerificationCacheStatus::Hit,
                key: Some(key),
                input_sha256: Some(input_sha256),
                kicad_cli_version,
                verifier_sha256,
                policy: KICAD_VERIFICATION_POLICY.into(),
                refill_zones,
                cache_root: cache_root.to_path_buf(),
                cache_entry: Some(cache_entry),
                elapsed_micros: started.elapsed().as_micros() as u64,
                detail: "restored exact ERC, DRC, and interpreted verification reports".into(),
            },
        )?;
        return Ok(entry.report);
    }

    let report = verify_materialized_rung_uncached(directory, board_id, refill_zones)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock precedes UNIX epoch: {error}"))?
        .as_nanos();
    let temporary = cache_root.join(format!(".{key}.{}.{}.tmp", std::process::id(), nonce));
    fs::create_dir(&temporary).map_err(|error| {
        format!(
            "failed to create temporary cache entry {}: {error}",
            temporary.display()
        )
    })?;
    let erc = directory.join("erc.json");
    let drc = directory.join("drc.json");
    fs::copy(&erc, temporary.join("erc.json"))
        .map_err(|error| format!("failed to cache ERC report: {error}"))?;
    fs::copy(&drc, temporary.join("drc.json"))
        .map_err(|error| format!("failed to cache DRC report: {error}"))?;
    fs::copy(
        directory.join("verification.json"),
        temporary.join("verification.json"),
    )
    .map_err(|error| format!("failed to cache verification report: {error}"))?;
    let entry = KiCadVerificationCacheEntry {
        schema_version: KICAD_VERIFICATION_CACHE_SCHEMA_VERSION,
        key: key.clone(),
        input_sha256: input_sha256.clone(),
        kicad_cli_version: kicad_cli_version.clone(),
        verifier_sha256: verifier_sha256.clone(),
        policy: KICAD_VERIFICATION_POLICY.into(),
        refill_zones,
        erc_sha256: file_sha256(&erc)?,
        drc_sha256: file_sha256(&drc)?,
        report: report.clone(),
    };
    write_typed_json(&temporary.join("cache-entry.json"), &entry)?;
    if let Err(error) = fs::rename(&temporary, &cache_entry) {
        if cache_entry.is_dir() {
            fs::remove_dir_all(&temporary).map_err(|cleanup| {
                format!(
                    "cache entry race ({error}) and failed to remove {}: {cleanup}",
                    temporary.display()
                )
            })?;
        } else {
            return Err(format!(
                "failed to publish cache entry {}: {error}",
                cache_entry.display()
            ));
        }
    }
    write_kicad_cache_trace(
        directory,
        &KiCadVerificationCacheTrace {
            schema_version: KICAD_VERIFICATION_CACHE_SCHEMA_VERSION,
            status: KiCadVerificationCacheStatus::Miss,
            key: Some(key),
            input_sha256: Some(input_sha256),
            kicad_cli_version,
            verifier_sha256,
            policy: KICAD_VERIFICATION_POLICY.into(),
            refill_zones,
            cache_root: cache_root.to_path_buf(),
            cache_entry: Some(cache_entry),
            elapsed_micros: started.elapsed().as_micros() as u64,
            detail: "ran native KiCad ERC/DRC and published an immutable cache entry".into(),
        },
    )?;
    Ok(report)
}

/// Identifies the native-verification implementation independently of the
/// router executable that happens to call it.
///
/// The cache intentionally invalidates conservatively after any change to this
/// crate or the resolved Rust dependency graph. Changes confined to placement
/// and routing crates retain native KiCad verification hits when their emitted
/// KiCad inputs are byte-identical.
pub fn kicad_verifier_implementation_sha256() -> String {
    let mut hash = Sha256::new();
    hash.update(b"pcb-maker-kicad-verifier-implementation-v1");
    hash.update(include_bytes!("lib.rs"));
    hash.update(include_bytes!("native_report.rs"));
    hash.update(include_bytes!("../../../Cargo.lock"));
    format!("{:x}", hash.finalize())
}

fn kicad_cli_version() -> Result<String, String> {
    let output = Command::new("kicad-cli")
        .arg("version")
        .output()
        .map_err(|error| format!("failed to identify kicad-cli: {error}"))?;
    if !output.status.success() {
        return Err(format!("kicad-cli version exited with {}", output.status));
    }
    String::from_utf8(output.stdout)
        .map(|version| version.trim().to_string())
        .map_err(|error| format!("kicad-cli version was not UTF-8: {error}"))
}

fn kicad_cache_input_sha256(directory: &Path) -> Result<KiCadCacheInput, String> {
    for table in ["sym-lib-table", "fp-lib-table"] {
        let path = directory.join(table);
        if !path.is_file() {
            continue;
        }
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        for tail in source.split("(uri \"").skip(1) {
            let Some(uri) = tail.split('"').next() else {
                return Ok(KiCadCacheInput::Bypassed(format!(
                    "could not parse library URI in {table}"
                )));
            };
            if !uri.starts_with("${KIPRJMOD}/") {
                return Ok(KiCadCacheInput::Bypassed(format!(
                    "{table} references external library {uri:?}"
                )));
            }
        }
    }
    let mut files = Vec::new();
    collect_kicad_cache_inputs(directory, directory, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    if files.is_empty() {
        return Err(format!(
            "{} contains no KiCad design inputs",
            directory.display()
        ));
    }
    let mut hash = Sha256::new();
    hash.update(b"pcb-maker-kicad-verification-input-v1");
    for (relative, absolute) in files {
        let relative = relative.as_os_str().as_encoded_bytes();
        let contents = fs::read(&absolute)
            .map_err(|error| format!("failed to read {}: {error}", absolute.display()))?;
        hash.update((relative.len() as u64).to_le_bytes());
        hash.update(relative);
        hash.update((contents.len() as u64).to_le_bytes());
        hash.update(contents);
    }
    Ok(KiCadCacheInput::Cacheable(format!("{:x}", hash.finalize())))
}

fn collect_kicad_cache_inputs(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<(), String> {
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("failed to read {}: {error}", directory.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", entry.path().display()))?;
        if file_type.is_symlink() {
            return Err(format!(
                "KiCad verification cache rejects symlink {}",
                entry.path().display()
            ));
        }
        if file_type.is_dir() {
            collect_kicad_cache_inputs(root, &entry.path(), files)?;
            continue;
        }
        if !file_type.is_file() || !is_kicad_cache_input(&entry.path()) {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_path_buf();
        files.push((relative, entry.path()));
    }
    Ok(())
}

fn is_kicad_cache_input(path: &Path) -> bool {
    let name = path.file_name().and_then(|name| name.to_str());
    if matches!(name, Some("sym-lib-table" | "fp-lib-table")) {
        return true;
    }
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("kicad_sch" | "kicad_pcb" | "kicad_pro" | "kicad_sym" | "kicad_mod" | "kicad_dru")
    )
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let contents =
        fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(contents)))
}

fn write_typed_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    fs::write(path, format!("{serialized}\n"))
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn read_typed_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn write_kicad_cache_trace(
    directory: &Path,
    trace: &KiCadVerificationCacheTrace,
) -> Result<(), String> {
    write_typed_json(&directory.join("verification-cache.json"), trace)?;
    if let Some(event_path) = env::var_os(KICAD_VERIFICATION_CACHE_EVENTS_ENV) {
        let event_path = PathBuf::from(event_path);
        if let Some(parent) = event_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create cache event directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let mut events = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&event_path)
            .map_err(|error| {
                format!(
                    "failed to open cache event log {}: {error}",
                    event_path.display()
                )
            })?;
        let serialized = serde_json::to_string(trace).map_err(|error| error.to_string())?;
        writeln!(events, "{serialized}").map_err(|error| {
            format!(
                "failed to append cache event log {}: {error}",
                event_path.display()
            )
        })?;
    }
    Ok(())
}

fn copy_directory_tree(source: &Path, destination: &Path) -> Result<(), String> {
    if destination.exists() {
        return Err(format!(
            "refusing to overwrite KiCad action artifact {}",
            destination.display()
        ));
    }
    fs::create_dir(destination).map_err(|error| {
        format!(
            "failed to create KiCad action artifact {}: {error}",
            destination.display()
        )
    })?;
    // An output directory inside the project is not copied into itself.
    let root = fs::canonicalize(destination).unwrap_or_else(|_| destination.to_path_buf());
    copy_directory_entries(source, destination, &root)
}

fn copy_directory_entries(source: &Path, destination: &Path, root: &Path) -> Result<(), String> {
    for entry in fs::read_dir(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "failed to inspect KiCad artifact {}: {error}",
                entry.path().display()
            )
        })?;
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            if fs::canonicalize(entry.path()).is_ok_and(|path| root.starts_with(path)) {
                continue;
            }
            fs::create_dir(&target).map_err(|error| {
                format!(
                    "failed to create KiCad action artifact {}: {error}",
                    target.display()
                )
            })?;
            copy_directory_entries(&entry.path(), &target, root)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target).map_err(|error| {
                format!(
                    "failed to copy KiCad artifact {} to {}: {error}",
                    entry.path().display(),
                    target.display()
                )
            })?;
        } else {
            return Err(format!(
                "KiCad reference-action source contains unsupported non-file artifact {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

fn set_form_at(node: &mut Expr, at: [f64; 3]) -> Result<(), String> {
    let Expr::List(items) = node else {
        return Err("cannot set at form on an atom".into());
    };
    let Some(Expr::List(values)) = items.iter_mut().find(|item| item.head() == Some("at")) else {
        return Err("node has no at form".into());
    };
    values.truncate(4);
    while values.len() < 4 {
        values.push(Expr::Atom("0".into()));
    }
    values[1] = Expr::Atom(at[0].to_string());
    values[2] = Expr::Atom(at[1].to_string());
    values[3] = Expr::Atom(at[2].to_string());
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct SelectedRouteSegment {
    branch_index: usize,
    segment_index: usize,
    start: [f64; 2],
    end: [f64; 2],
}

const ROUTE_CONTACT_EPSILON_MM: f64 = 1.0e-9;

fn route_cross(left: [f64; 2], right: [f64; 2]) -> f64 {
    left[0] * right[1] - left[1] * right[0]
}

fn route_subtract(left: [f64; 2], right: [f64; 2]) -> [f64; 2] {
    [left[0] - right[0], left[1] - right[1]]
}

fn route_point_on_segment(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> bool {
    let direction = route_subtract(end, start);
    let offset = route_subtract(point, start);
    let length_squared = direction[0] * direction[0] + direction[1] * direction[1];
    if length_squared <= ROUTE_CONTACT_EPSILON_MM * ROUTE_CONTACT_EPSILON_MM {
        return distance_squared(point, start)
            <= ROUTE_CONTACT_EPSILON_MM * ROUTE_CONTACT_EPSILON_MM;
    }
    let scale = length_squared.sqrt().max(1.0);
    if route_cross(direction, offset).abs() > ROUTE_CONTACT_EPSILON_MM * scale {
        return false;
    }
    let projection = offset[0] * direction[0] + offset[1] * direction[1];
    projection >= -ROUTE_CONTACT_EPSILON_MM
        && projection <= length_squared + ROUTE_CONTACT_EPSILON_MM
}

fn route_segment_parameter(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> f64 {
    let direction = route_subtract(end, start);
    let length_squared = direction[0] * direction[0] + direction[1] * direction[1];
    if length_squared == 0.0 {
        0.0
    } else {
        let offset = route_subtract(point, start);
        (offset[0] * direction[0] + offset[1] * direction[1]) / length_squared
    }
}

/// Returns the physical contact points shared by two same-layer segments.
///
/// Collinear overlap is represented by its endpoint vertices. Proper
/// crossings get one computed point, while endpoint/T contacts retain the
/// exact stored endpoint coordinate so later bitwise grouping is stable.
fn route_segment_contacts(
    first_start: [f64; 2],
    first_end: [f64; 2],
    second_start: [f64; 2],
    second_end: [f64; 2],
) -> Vec<[f64; 2]> {
    let mut contacts = Vec::new();
    for point in [first_start, first_end, second_start, second_end] {
        if route_point_on_segment(point, first_start, first_end)
            && route_point_on_segment(point, second_start, second_end)
            && !contacts.iter().any(|existing| {
                distance_squared(*existing, point)
                    <= ROUTE_CONTACT_EPSILON_MM * ROUTE_CONTACT_EPSILON_MM
            })
        {
            contacts.push(point);
        }
    }
    if !contacts.is_empty() {
        return contacts;
    }

    let first_direction = route_subtract(first_end, first_start);
    let second_direction = route_subtract(second_end, second_start);
    let denominator = route_cross(first_direction, second_direction);
    let scale = (distance_squared(first_start, first_end)
        * distance_squared(second_start, second_end))
    .sqrt()
    .max(1.0);
    if denominator.abs() <= ROUTE_CONTACT_EPSILON_MM * scale {
        return contacts;
    }
    let between_starts = route_subtract(second_start, first_start);
    let first_parameter = route_cross(between_starts, second_direction) / denominator;
    let second_parameter = route_cross(between_starts, first_direction) / denominator;
    let contact_range = -ROUTE_CONTACT_EPSILON_MM..=1.0 + ROUTE_CONTACT_EPSILON_MM;
    if contact_range.contains(&first_parameter) && contact_range.contains(&second_parameter) {
        contacts.push([
            first_start[0] + first_parameter * first_direction[0],
            first_start[1] + first_parameter * first_direction[1],
        ]);
    }
    contacts
}

/// Inserts explicit vertices wherever selected same-layer routes physically
/// touch. The operation preserves the initial copper geometry but turns
/// collinear overlap, T contacts, and crossings into a graph that the generic
/// shared-particle compiler can represent without duplicate physical edges.
fn normalize_selected_route_graph(branches: &mut [KiCadRouteBranch], selected: &[usize]) {
    normalize_selected_route_graph_against(branches, selected, selected)
}

fn normalize_selected_route_graph_against(
    branches: &mut [KiCadRouteBranch],
    selected: &[usize],
    context: &[usize],
) {
    let selected_set = selected.iter().copied().collect::<BTreeSet<_>>();
    let original_paths = context
        .iter()
        .map(|&branch_index| (branch_index, branches[branch_index].path.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut segments = Vec::new();
    let mut cuts = BTreeMap::<(usize, usize), Vec<[f64; 2]>>::new();
    for (&branch_index, path) in &original_paths {
        for (segment_index, pair) in path.windows(2).enumerate() {
            if pair[0].layer != pair[1].layer || pair[0].at == pair[1].at {
                continue;
            }
            segments.push(SelectedRouteSegment {
                branch_index,
                segment_index,
                start: pair[0].at,
                end: pair[1].at,
            });
            if selected_set.contains(&branch_index) {
                cuts.insert((branch_index, segment_index), vec![pair[0].at, pair[1].at]);
            }
        }
    }

    for first in 0..segments.len() {
        for second in (first + 1)..segments.len() {
            let first_segment = segments[first];
            let second_segment = segments[second];
            let first_path = &original_paths[&first_segment.branch_index];
            let second_path = &original_paths[&second_segment.branch_index];
            if first_path[first_segment.segment_index].layer
                != second_path[second_segment.segment_index].layer
            {
                continue;
            }
            let contacts = route_segment_contacts(
                first_segment.start,
                first_segment.end,
                second_segment.start,
                second_segment.end,
            );
            // Ordinary adjacent segments only share their existing vertex.
            // A backtrack shares an interval: split its endpoints just like
            // overlaps between different branches so physical edges dedupe.
            if first_segment.branch_index == second_segment.branch_index
                && first_segment
                    .segment_index
                    .abs_diff(second_segment.segment_index)
                    == 1
                && contacts.len() <= 1
            {
                continue;
            }
            for contact in contacts {
                if let Some(segment_cuts) =
                    cuts.get_mut(&(first_segment.branch_index, first_segment.segment_index))
                {
                    segment_cuts.push(contact);
                }
                if let Some(segment_cuts) =
                    cuts.get_mut(&(second_segment.branch_index, second_segment.segment_index))
                {
                    segment_cuts.push(contact);
                }
            }
        }
    }

    for &branch_index in selected {
        let original = &original_paths[&branch_index];
        // A terminal already reached by the tree can have a single contact
        // point and no new segment. Preserve that connectivity evidence.
        if original.len() < 2 {
            continue;
        }
        let mut normalized = Vec::new();
        for (segment_index, pair) in original.windows(2).enumerate() {
            let Some(segment_cuts) = cuts.get(&(branch_index, segment_index)) else {
                if normalized.is_empty() {
                    normalized.push(pair[0].clone());
                }
                normalized.push(pair[1].clone());
                continue;
            };
            let mut segment_cuts = segment_cuts.clone();
            segment_cuts.sort_by(|left, right| {
                route_segment_parameter(*left, pair[0].at, pair[1].at)
                    .total_cmp(&route_segment_parameter(*right, pair[0].at, pair[1].at))
                    .then_with(|| left[0].total_cmp(&right[0]))
                    .then_with(|| left[1].total_cmp(&right[1]))
            });
            segment_cuts.dedup_by(|left, right| {
                distance_squared(*left, *right)
                    <= ROUTE_CONTACT_EPSILON_MM * ROUTE_CONTACT_EPSILON_MM
            });
            for (cut_index, at) in segment_cuts.into_iter().enumerate() {
                if segment_index > 0 && cut_index == 0 {
                    continue;
                }
                normalized.push(KiCadRoutePoint {
                    at,
                    layer: pair[0].layer.clone(),
                });
            }
        }
        branches[branch_index].path = normalized;
    }
}

#[derive(Clone, Debug)]
struct BoardOutline {
    points: Vec<[f64; 2]>,
    bounds: [f64; 4],
}

impl BoardOutline {
    fn rectangle(bounds: [f64; 4]) -> Result<Self, String> {
        Self::new(vec![
            [bounds[0], bounds[1]],
            [bounds[2], bounds[1]],
            [bounds[2], bounds[3]],
            [bounds[0], bounds[3]],
        ])
    }

    fn new(points: Vec<[f64; 2]>) -> Result<Self, String> {
        if points.len() < 3
            || points
                .iter()
                .flatten()
                .any(|coordinate| !coordinate.is_finite())
        {
            return Err("board outline has invalid polygon coordinates".into());
        }
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for point in &points {
            bounds[0] = bounds[0].min(point[0]);
            bounds[1] = bounds[1].min(point[1]);
            bounds[2] = bounds[2].max(point[0]);
            bounds[3] = bounds[3].max(point[1]);
        }
        let outline = Self { points, bounds };
        if outline.area() <= 1.0e-12 {
            return Err("board outline polygon has zero area".into());
        }
        Ok(outline)
    }

    fn area(&self) -> f64 {
        self.points
            .iter()
            .zip(self.points.iter().cycle().skip(1))
            .take(self.points.len())
            .map(|(left, right)| left[0] * right[1] - right[0] * left[1])
            .sum::<f64>()
            .abs()
            / 2.0
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ObstacleGeometry {
    Circle {
        center: [f64; 2],
        radius: f64,
    },
    Segment {
        start: [f64; 2],
        end: [f64; 2],
        radius: f64,
    },
    Rectangle {
        center: [f64; 2],
        half_size: [f64; 2],
        angle_degrees: f64,
    },
    Polygon {
        points: Vec<[f64; 2]>,
    },
    Union {
        parts: Vec<ObstacleGeometry>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct GeometryAabb {
    minimum: [f64; 2],
    maximum: [f64; 2],
}

impl GeometryAabb {
    fn around_segment(start: [f64; 2], end: [f64; 2], inflate: f64) -> Self {
        Self {
            minimum: [
                start[0].min(end[0]) - inflate,
                start[1].min(end[1]) - inflate,
            ],
            maximum: [
                start[0].max(end[0]) + inflate,
                start[1].max(end[1]) + inflate,
            ],
        }
    }
}

impl ObstacleGeometry {
    fn aabb(&self) -> GeometryAabb {
        match self {
            Self::Circle { center, radius } => GeometryAabb {
                minimum: [center[0] - radius, center[1] - radius],
                maximum: [center[0] + radius, center[1] + radius],
            },
            Self::Segment { start, end, radius } => {
                GeometryAabb::around_segment(*start, *end, *radius)
            }
            Self::Rectangle {
                center,
                half_size,
                angle_degrees,
            } => {
                let (sine, cosine) = quarter_sin_cos(*angle_degrees);
                let extent = [
                    cosine.abs() * half_size[0] + sine.abs() * half_size[1],
                    sine.abs() * half_size[0] + cosine.abs() * half_size[1],
                ];
                GeometryAabb {
                    minimum: [center[0] - extent[0], center[1] - extent[1]],
                    maximum: [center[0] + extent[0], center[1] + extent[1]],
                }
            }
            Self::Polygon { points } => polygon_aabb(points),
            Self::Union { parts } => parts
                .iter()
                .map(Self::aabb)
                .reduce(|left, right| GeometryAabb {
                    minimum: [
                        left.minimum[0].min(right.minimum[0]),
                        left.minimum[1].min(right.minimum[1]),
                    ],
                    maximum: [
                        left.maximum[0].max(right.maximum[0]),
                        left.maximum[1].max(right.maximum[1]),
                    ],
                })
                .expect("compound pad geometry has an anchor"),
        }
    }
}

fn polygon_aabb(points: &[[f64; 2]]) -> GeometryAabb {
    let mut minimum = [f64::INFINITY; 2];
    let mut maximum = [f64::NEG_INFINITY; 2];
    for point in points {
        for axis in 0..2 {
            minimum[axis] = minimum[axis].min(point[axis]);
            maximum[axis] = maximum[axis].max(point[axis]);
        }
    }
    GeometryAabb { minimum, maximum }
}

fn point_in_polygon(point: [f64; 2], polygon: &[[f64; 2]]) -> bool {
    let mut inside = false;
    let mut previous = polygon.len() - 1;
    for current in 0..polygon.len() {
        let first = polygon[current];
        let second = polygon[previous];
        if point_segment_distance_squared(point, first, second) <= 1.0e-18 {
            return true;
        }
        if (first[1] > point[1]) != (second[1] > point[1])
            && point[0]
                < (second[0] - first[0]) * (point[1] - first[1]) / (second[1] - first[1]) + first[0]
        {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

fn rule_area_layer_mask(zone: &Expr) -> Result<[bool; 2], String> {
    let mut mask = [false; 2];
    if let Some(layer) = form_atom(zone, "layer", 1) {
        let index = copper_layer_index(layer)
            .ok_or_else(|| format!("unsupported rule-area layer {layer:?}"))?;
        mask[index] = true;
    } else if let Some(layers) = zone.child("layers") {
        for layer in layers.children().iter().skip(1).filter_map(Expr::atom) {
            match layer {
                "F&B.Cu" | "*.Cu" => mask = [true, true],
                "F.Cu" => mask[0] = true,
                "B.Cu" => mask[1] = true,
                _ => return Err(format!("unsupported rule-area layer {layer:?}")),
            }
        }
    }
    if mask == [false, false] {
        Err("rule area has no supported copper layer".into())
    } else {
        Ok(mask)
    }
}

/// A rule area's outline as one simple polygon. KiCad writes the zone's
/// outline as its first `polygon` and every hole as a further `polygon`
/// (Glasgow keeps tracks off a 1.7 mm rim this way: a board-sized outline
/// with the inside cut out). Each hole is joined to the outline by a slit
/// between their closest vertices, walked in and out again, which the
/// even-odd point test and the edge distances treat as the ring it is.
fn rule_area_polygon_points(zone: &Expr) -> Result<Vec<[f64; 2]>, String> {
    let mut polygons: Vec<Vec<[f64; 2]>> = Vec::new();
    for polygon in zone
        .children()
        .iter()
        .filter(|item| item.head() == Some("polygon"))
    {
        // Sides drawn as arcs (Telemetry's GND zones along its round
        // edge), flattened as KiCad's filler does (ClearArcs); they were
        // left out, the zone then a polygon through the other corners.
        let arcs = polygon
            .child("pts")
            .is_some_and(|pts| pts.children().iter().any(|item| item.head() == Some("arc")));
        if arcs {
            let points = outline::pts_points(polygon)?;
            if points.len() >= 3 {
                polygons.push(points);
            }
            continue;
        }
        let points = polygon
            .child("pts")
            .ok_or_else(|| "rule area has no polygon points".to_string())?
            .children()
            .iter()
            .skip(1)
            .filter(|point| point.head() == Some("xy"))
            .map(|point| {
                let coordinate = |index: usize| {
                    point
                        .children()
                        .get(index)
                        .and_then(Expr::atom)
                        .ok_or_else(|| "rule-area xy point is missing a coordinate".to_string())?
                        .parse::<f64>()
                        .map_err(|error| format!("invalid rule-area xy coordinate: {error}"))
                };
                Ok::<[f64; 2], String>([coordinate(1)?, coordinate(2)?])
            })
            .collect::<Result<Vec<_>, _>>()?;
        if points.len() >= 3 {
            polygons.push(points);
        }
    }
    let mut polygons = polygons.into_iter();
    let mut outline = polygons
        .next()
        .ok_or_else(|| "rule area polygon has fewer than three points".to_string())?;
    for hole in polygons {
        let (i, j) = (0..outline.len())
            .flat_map(|i| (0..hole.len()).map(move |j| (i, j)))
            .min_by(|(a, b), (c, d)| {
                let da =
                    (outline[*a][0] - hole[*b][0]).powi(2) + (outline[*a][1] - hole[*b][1]).powi(2);
                let dc =
                    (outline[*c][0] - hole[*d][0]).powi(2) + (outline[*c][1] - hole[*d][1]).powi(2);
                da.partial_cmp(&dc).unwrap()
            })
            .unwrap();
        let mut joined = outline[..=i].to_vec();
        joined.extend(hole[j..].iter().chain(hole[..=j].iter()));
        joined.extend_from_slice(&outline[i..]);
        outline = joined;
    }
    Ok(outline)
}

struct LoweredPad {
    center: [f64; 2],
    geometry: ObstacleGeometry,
}

/// Board-space copper of one footprint pad, as KiCad's effective shape.
fn lower_pad(pad: &Expr, footprint_at: [f64; 3]) -> Result<LoweredPad, String> {
    let pad_at = form_at(pad)?;
    // KiCad's board coordinates have Y pointing down: positive footprint
    // angles therefore rotate local pad coordinates clockwise.
    let center_offset = rotate_vector([pad_at[0], pad_at[1]], -footprint_at[2]);
    let center = [
        footprint_at[0] + center_offset[0],
        footprint_at[1] + center_offset[1],
    ];
    let size = form_xy(pad, "size")?;
    let shape = pad.children().get(3).and_then(Expr::atom).unwrap_or("rect");
    // KiCad's drill offset displaces copper relative to the drill/pad
    // anchor. Pad angles in board files are absolute, and Y points down.
    let copper_offset = match pad.child("drill").and_then(|d| d.child("offset")) {
        Some(offset) => rotate_vector(
            [
                expression_coordinate(offset, 1, "pad copper offset x")?,
                expression_coordinate(offset, 2, "pad copper offset y")?,
            ],
            -pad_at[2],
        ),
        None => [0.0, 0.0],
    };
    let copper_center = [center[0] + copper_offset[0], center[1] + copper_offset[1]];
    let anchor_geometry = match shape {
        "roundrect" => roundrect_pad::geometry(pad, copper_center, size, -pad_at[2])?,
        "circle" => ObstacleGeometry::Circle {
            center: copper_center,
            radius: size[0].max(size[1]) / 2.0,
        },
        "oval" => {
            // Pad angles in a board file are already absolute, rather
            // than relative to the footprint angle.
            let angle = -pad_at[2];
            let (half_length, radius, local_end) = if size[0] >= size[1] {
                ((size[0] - size[1]) / 2.0, size[1] / 2.0, [1.0, 0.0])
            } else {
                ((size[1] - size[0]) / 2.0, size[0] / 2.0, [0.0, 1.0])
            };
            let offset = rotate_vector(
                [local_end[0] * half_length, local_end[1] * half_length],
                angle,
            );
            ObstacleGeometry::Segment {
                start: [copper_center[0] - offset[0], copper_center[1] - offset[1]],
                end: [copper_center[0] + offset[0], copper_center[1] + offset[1]],
                radius,
            }
        }
        "trapezoid" => {
            // KiCad's corners (PAD::BuildPadPolygon), with half the delta.
            let delta = form_xy(pad, "rect_delta").unwrap_or([0.0, 0.0]);
            let (half, d) = (
                [size[0] / 2.0, size[1] / 2.0],
                [delta[0] / 2.0, delta[1] / 2.0],
            );
            let corners = [
                [-half[0] - d[1], half[1] + d[0]],
                [-half[0] + d[1], -half[1] - d[0]],
                [half[0] - d[1], -half[1] + d[0]],
                [half[0] + d[1], half[1] - d[0]],
            ];
            ObstacleGeometry::Polygon {
                points: corners
                    .iter()
                    .map(|corner| {
                        let offset = rotate_vector(*corner, -pad_at[2]);
                        [copper_center[0] + offset[0], copper_center[1] + offset[1]]
                    })
                    .collect(),
            }
        }
        _ => ObstacleGeometry::Rectangle {
            center: copper_center,
            half_size: [size[0] / 2.0, size[1] / 2.0],
            angle_degrees: -pad_at[2],
        },
    };
    let geometry = if shape == "custom" {
        custom_pad_geometry(pad, copper_center, pad_at[2], anchor_geometry)?
    } else {
        anchor_geometry
    };
    Ok(LoweredPad { center, geometry })
}

fn custom_pad_geometry(
    pad: &Expr,
    center: [f64; 2],
    pad_angle_degrees: f64,
    anchor: ObstacleGeometry,
) -> Result<ObstacleGeometry, String> {
    let mut parts = vec![anchor];
    let primitives = pad
        .child("primitives")
        .ok_or_else(|| "custom pad has no primitives form".to_string())?;
    for primitive in primitives.children().iter().skip(1) {
        let width = primitive
            .child("stroke")
            .and_then(|stroke| form_f64(stroke, "width", 1).ok())
            .or_else(|| form_f64(primitive, "width", 1).ok())
            .unwrap_or(0.0)
            .max(0.0);
        let filled = !matches!(form_atom(primitive, "fill", 1), None | Some("no" | "none"));
        let local_point = |point: &Expr, label: &str| -> Result<[f64; 2], String> {
            let local = [
                expression_coordinate(point, 1, label)?,
                expression_coordinate(point, 2, label)?,
            ];
            let offset = rotate_vector(local, -pad_angle_degrees);
            Ok([center[0] + offset[0], center[1] + offset[1]])
        };
        let xy = |head: &str| -> Result<[f64; 2], String> {
            let form = primitive
                .child(head)
                .ok_or_else(|| format!("custom-pad primitive has no {head}"))?;
            local_point(form, head)
        };
        match primitive.head() {
            Some("gr_poly") => {
                let points_form = primitive
                    .child("pts")
                    .ok_or_else(|| "custom-pad polygon has no pts form".to_string())?;
                let mut points = Vec::new();
                for point in points_form.children().iter().skip(1) {
                    match point.head() {
                        Some("xy") => points.push(local_point(point, "custom-pad point")?),
                        // A rounded side, flattened.
                        Some("arc") => {
                            let corner = |head: &str| {
                                point
                                    .child(head)
                                    .ok_or_else(|| format!("custom-pad arc has no {head}"))
                                    .and_then(|form| local_point(form, head))
                            };
                            for at in outline::arc_points(
                                corner("start")?,
                                corner("mid")?,
                                corner("end")?,
                            ) {
                                if points.last() != Some(&at) {
                                    points.push(at);
                                }
                            }
                        }
                        _ => {
                            return Err(
                                "custom-pad polygon contains a point that is neither xy nor arc"
                                    .into(),
                            );
                        }
                    }
                }
                if points.len() < 3 {
                    return Err("custom-pad polygon has fewer than three points".into());
                }
                if filled {
                    parts.push(ObstacleGeometry::Polygon {
                        points: points.clone(),
                    });
                }
                // A stroked outline is copper too: one capsule per edge.
                if width > 0.0 {
                    for index in 0..points.len() {
                        parts.push(ObstacleGeometry::Segment {
                            start: points[index],
                            end: points[(index + 1) % points.len()],
                            radius: width / 2.0,
                        });
                    }
                }
            }
            Some("gr_line") => parts.push(ObstacleGeometry::Segment {
                start: xy("start")?,
                end: xy("end")?,
                radius: width.max(0.01) / 2.0,
            }),
            Some("gr_rect") => {
                let (a, b) = (xy("start")?, xy("end")?);
                // Rotated pads make the rectangle a general quadrilateral.
                let local_a = [
                    expression_coordinate(primitive.child("start").unwrap(), 1, "x")?,
                    expression_coordinate(primitive.child("start").unwrap(), 2, "y")?,
                ];
                let local_b = [
                    expression_coordinate(primitive.child("end").unwrap(), 1, "x")?,
                    expression_coordinate(primitive.child("end").unwrap(), 2, "y")?,
                ];
                let corners: Vec<[f64; 2]> = [
                    local_a,
                    [local_b[0], local_a[1]],
                    local_b,
                    [local_a[0], local_b[1]],
                ]
                .iter()
                .map(|corner| {
                    let offset = rotate_vector(*corner, -pad_angle_degrees);
                    [center[0] + offset[0], center[1] + offset[1]]
                })
                .collect();
                let _ = (a, b);
                if filled {
                    parts.push(ObstacleGeometry::Polygon {
                        points: corners.clone(),
                    });
                }
                if width > 0.0 {
                    for index in 0..4 {
                        parts.push(ObstacleGeometry::Segment {
                            start: corners[index],
                            end: corners[(index + 1) % 4],
                            radius: width / 2.0,
                        });
                    }
                }
            }
            Some("gr_circle") => {
                let circle_center = xy("center")?;
                let end = xy("end")?;
                let radius = distance_squared(circle_center, end).sqrt();
                if filled || width <= 0.0 {
                    parts.push(ObstacleGeometry::Circle {
                        center: circle_center,
                        radius: radius + width / 2.0,
                    });
                } else {
                    // An unfilled circle is its ring (a dome switch's outer
                    // contact around its inner pad, Sisu's keypad).
                    for pair in outline::circle_points(circle_center, radius).windows(2) {
                        parts.push(ObstacleGeometry::Segment {
                            start: pair[0],
                            end: pair[1],
                            radius: width / 2.0 + 2.0 * outline::ARC_TOLERANCE,
                        });
                    }
                }
            }
            Some("gr_arc") => {
                // Our chords lie inside the arc (by up to ARC_TOLERANCE) and
                // KiCad's approximation outside it (by up to its own 5 um):
                // the swept width covers both, or tracks end up a fraction
                // of a micrometre too close (capacitive sensor pads drawn
                // from arcs: 0.1993 mm against 0.2).
                let (start, mid, end) = (xy("start")?, xy("mid")?, xy("end")?);
                for pair in outline::arc_points(start, mid, end).windows(2) {
                    parts.push(ObstacleGeometry::Segment {
                        start: pair[0],
                        end: pair[1],
                        radius: width.max(0.01) / 2.0 + 2.0 * outline::ARC_TOLERANCE,
                    });
                }
            }
            // The pad editor's number box: KiCad reads it as a proxy shape
            // (SetIsProxyItem) that its pad outline leaves out, no copper.
            Some("gr_bbox") => {}
            other => {
                return Err(format!(
                    "KiCad grid adapter does not yet lower custom-pad primitive {}",
                    other.unwrap_or("unknown")
                ));
            }
        }
    }
    if parts.len() == 1 {
        // No primitives: KiCad draws just the anchor shape.
        return Ok(parts.pop().expect("the anchor"));
    }
    Ok(ObstacleGeometry::Union { parts })
}

fn expression_coordinate(node: &Expr, index: usize, label: &str) -> Result<f64, String> {
    node.children()
        .get(index)
        .and_then(Expr::atom)
        .ok_or_else(|| format!("{label} is missing"))?
        .parse::<f64>()
        .map_err(|error| format!("invalid {label}: {error}"))
}

fn pad_copper_layers(pad: &Expr) -> [bool; 2] {
    let Some(layers) = pad.child("layers") else {
        return [false, false];
    };
    let names: Vec<_> = layers
        .children()
        .iter()
        .skip(1)
        .filter_map(Expr::atom)
        .collect();
    [
        names.iter().any(|name| matches!(*name, "F.Cu" | "*.Cu")),
        names.iter().any(|name| matches!(*name, "B.Cu" | "*.Cu")),
    ]
}

fn copper_layer_index(layer: &str) -> Option<usize> {
    match layer {
        "F.Cu" => Some(0),
        "B.Cu" => Some(1),
        _ => None,
    }
}

fn form_atom<'a>(node: &'a Expr, head: &str, index: usize) -> Option<&'a str> {
    node.child(head)?.children().get(index)?.atom()
}

fn form_f64(node: &Expr, head: &str, index: usize) -> Result<f64, String> {
    form_atom(node, head, index)
        .ok_or_else(|| format!("{head} form is missing coordinate {index}"))?
        .parse::<f64>()
        .map_err(|error| format!("invalid {head} coordinate: {error}"))
}

fn form_xy(node: &Expr, head: &str) -> Result<[f64; 2], String> {
    Ok([form_f64(node, head, 1)?, form_f64(node, head, 2)?])
}

fn form_at(node: &Expr) -> Result<[f64; 3], String> {
    let at = node
        .child("at")
        .ok_or_else(|| "copper item has no at form".to_string())?;
    Ok([
        form_f64(node, "at", 1)?,
        form_f64(node, "at", 2)?,
        // KiCad 6 writes `(at x y unlocked)` and `(at x y angle unlocked)`
        // for texts: the flag is not a rotation.
        match at.children().get(3).and_then(Expr::atom) {
            None | Some("unlocked" | "locked") => 0.0,
            Some(angle) => angle
                .parse::<f64>()
                .map_err(|error| format!("invalid at rotation: {error}"))?,
        },
    ])
}

/// The sine and cosine of an angle in degrees, exact for quarter turns
/// (`pcb_router::geometry::degrees_sin_cos`): one angle written 180 or
/// -180, 270 or -90 by two saves lowers alike.
fn quarter_sin_cos(degrees: f64) -> (f64, f64) {
    pcb_router::geometry::degrees_sin_cos(degrees)
}

fn rotate_vector(vector: [f64; 2], degrees: f64) -> [f64; 2] {
    let (sine, cosine) = quarter_sin_cos(degrees);
    [
        vector[0] * cosine - vector[1] * sine,
        vector[0] * sine + vector[1] * cosine,
    ]
}

fn distance_squared(left: [f64; 2], right: [f64; 2]) -> f64 {
    (left[0] - right[0]).powi(2) + (left[1] - right[1]).powi(2)
}

fn point_segment_distance_squared(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> f64 {
    let segment = [end[0] - start[0], end[1] - start[1]];
    let length_squared = segment[0].powi(2) + segment[1].powi(2);
    if length_squared <= f64::EPSILON {
        return distance_squared(point, start);
    }
    let projection = (((point[0] - start[0]) * segment[0] + (point[1] - start[1]) * segment[1])
        / length_squared)
        .clamp(0.0, 1.0);
    distance_squared(
        point,
        [
            start[0] + projection * segment[0],
            start[1] + projection * segment[1],
        ],
    )
}

fn read_json(path: &Path) -> Result<serde_json::Value, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn is_intentional_no_connect_group(violation: &serde_json::Value) -> bool {
    let Some(items) = violation["items"].as_array() else {
        return false;
    };
    if items.is_empty() {
        return false;
    }
    let mut component = None;
    for item in items {
        let Some(description) = item["description"].as_str() else {
            return false;
        };
        let Some(net_start) = description.find("[unconnected-(") else {
            return false;
        };
        let Some(net_end) = description[net_start..].find("] of ") else {
            return false;
        };
        let after_net = &description[net_start + net_end + 5..];
        let Some(reference) = after_net.split(" on ").next() else {
            return false;
        };
        match component {
            None => component = Some(reference),
            Some(expected) if expected == reference => {}
            Some(_) => return false,
        }
    }
    true
}

fn supplemental_via_expr(via: &SupplementalVia) -> Expr {
    let identity = format!(
        "pcb-maker:supplemental-via:{}:{:.9}:{:.9}:{:.9}:{:.9}:{}:{}",
        via.connection, via.at[0], via.at[1], via.size, via.drill, via.layers[0], via.layers[1]
    );
    Expr::List(vec![
        Expr::Atom("via".into()),
        coordinate_form("at", via.at),
        Expr::List(vec![
            Expr::Atom("size".into()),
            Expr::Atom(via.size.to_string()),
        ]),
        Expr::List(vec![
            Expr::Atom("drill".into()),
            Expr::Atom(via.drill.to_string()),
        ]),
        Expr::List(vec![
            Expr::Atom("layers".into()),
            Expr::Atom(quote(&via.layers[0])),
            Expr::Atom(quote(&via.layers[1])),
        ]),
        net_expr(&written_net_name(&via.connection)),
        Expr::List(vec![
            Expr::Atom("uuid".into()),
            Expr::Atom(quote(&deterministic_uuid_for(&identity))),
        ]),
    ])
}

/// Connection IDs omit one hierarchical slash, but native net names do not.
/// Pads identify the exact spelling to use for emitted/retained copper. Reject
/// aliases with two distinct pad nets rather than silently shorting them.
fn unambiguous_pad_net_names(pcb: &Expr) -> Result<BTreeMap<String, Expr>, String> {
    note_net_names(pcb);
    let mut names: BTreeMap<String, Expr> = BTreeMap::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|n| n.head() == Some("footprint"))
    {
        for pad in footprint
            .children()
            .iter()
            .filter(|n| n.head() == Some("pad"))
        {
            let Some(name) = node_net(pad).filter(|name| !name.is_empty()) else {
                continue;
            };
            let connection = normalize_net(name).to_string();
            let atom = pad.child("net").unwrap().children().last().unwrap().clone();
            if let Some(previous) = names.get(&connection) {
                if previous.atom() != Some(name) {
                    return Err(format!(
                        "ambiguous native net identity {connection:?}: pads name both {:?} and {name:?}",
                        previous.atom().unwrap()
                    ));
                }
            } else {
                names.insert(connection, atom);
            }
        }
    }
    Ok(names)
}

fn canonicalize_copper_net_names(pcb: &mut Expr, names: &BTreeMap<String, Expr>) {
    let Expr::List(items) = pcb else {
        return;
    };
    for item in items {
        if !matches!(item.head(), Some("segment" | "arc" | "via" | "zone")) {
            continue;
        }
        // The writer names a connection "/X"; the pads' map is keyed by the
        // normalised name, which keeps the slash when a bare "X" exists too
        // (`note_net_names`): then "/GND" must still find "GND", or KiCad
        // makes a net "/GND" of our copper (Hub on x153: 636 items, 36
        // clearance errors between "/GND" and "GND" vias).
        let Some(name) = node_net(item)
            .and_then(|name| {
                names
                    .get(normalize_net(name))
                    .or_else(|| name.strip_prefix('/').and_then(|bare| names.get(bare)))
                    .or_else(|| names.get(name))
            })
            .cloned()
        else {
            continue;
        };
        let Expr::List(parts) = item else {
            continue;
        };
        for part in parts {
            if let Some(head @ ("net" | "net_name")) = part.head() {
                // Preserve the source atom, including escaping, rather than
                // quoting its already escaped contents a second time.
                *part = Expr::List(vec![Expr::Atom(head.into()), name.clone()]);
            }
        }
    }
}

fn supplemental_segment_expr(segment: &SupplementalSegment) -> Expr {
    let identity = format!(
        "pcb-maker:supplemental:{}:{:.9}:{:.9}:{:.9}:{:.9}:{:.9}:{}",
        segment.connection,
        segment.start[0],
        segment.start[1],
        segment.end[0],
        segment.end[1],
        segment.width,
        segment.layer
    );
    Expr::List(vec![
        Expr::Atom("segment".into()),
        coordinate_form("start", segment.start),
        coordinate_form("end", segment.end),
        Expr::List(vec![
            Expr::Atom("width".into()),
            Expr::Atom(segment.width.to_string()),
        ]),
        string_form("layer", &segment.layer),
        net_expr(&written_net_name(&segment.connection)),
        Expr::List(vec![
            Expr::Atom("uuid".into()),
            Expr::Atom(quote(&deterministic_uuid_for(&identity))),
        ]),
    ])
}

fn coordinate_form(head: &str, coordinate: [f64; 2]) -> Expr {
    Expr::List(vec![
        Expr::Atom(head.into()),
        Expr::Atom(coordinate[0].to_string()),
        Expr::Atom(coordinate[1].to_string()),
    ])
}

thread_local! {
    /// The board's net names without a leading slash (its global labels
    /// and power nets), noted by `note_net_names`: a connection id drops
    /// one hierarchical slash, which must not fold `/VBUS` (a root-sheet
    /// label) into `VBUS` when the board has both (a business-card panel
    /// had both, and lowering rejected it as ambiguous).
    static BARE_NETS: std::cell::RefCell<std::collections::HashSet<String>> = Default::default();
}

/// Records `pcb`'s bare net names for `normalize_net` (on this thread,
/// for the lowering that follows).
pub(crate) fn note_net_names(pcb: &Expr) {
    // The net table; a KiCad 10 board has none (its items name their
    // nets), so the pads' names too: gametank's expansion-port board has
    // pads on "/A15" and on "A15", one net read before.
    let table = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("net"))
        .filter_map(|item| item.children().get(2).and_then(Expr::atom));
    let pads = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .flat_map(|footprint| footprint.children())
        .filter(|item| item.head() == Some("pad"))
        .filter_map(node_net);
    let bare: std::collections::HashSet<String> = table
        .chain(pads)
        .filter(|name| !name.starts_with('/') && !name.is_empty())
        .map(str::to_string)
        .collect();
    BARE_NETS.with(|names| *names.borrow_mut() = bare);
}

/// The net name copper of `connection` is written with: the connection
/// with the hierarchical slash it dropped, or as it is when the board has
/// a bare net of that name (`/A15` then is another net, its connection
/// `/A15`, written `//A15`); `canonicalize_copper_net_names` then takes
/// the pads' own spelling.
fn written_net_name(connection: &str) -> String {
    if BARE_NETS.with(|names| names.borrow().contains(connection)) {
        connection.to_string()
    } else {
        format!("/{connection}")
    }
}

fn normalize_net(net: &str) -> &str {
    match net.strip_prefix('/') {
        Some(bare) if !BARE_NETS.with(|names| names.borrow().contains(bare)) => bare,
        _ => net,
    }
}

fn node_net(node: &Expr) -> Option<&str> {
    net_name_from_form(node.child("net")?)
}

fn net_expr(name: &str) -> Expr {
    Expr::List(vec![
        Expr::Atom("net".into()),
        Expr::Atom(format!("\"{name}\"")),
    ])
}

fn string_form(head: &str, value: &str) -> Expr {
    Expr::List(vec![Expr::Atom(head.into()), Expr::Atom(quote(value))])
}

/// A footprint's reference: KiCad 8 and later write it as a property,
/// KiCad 6 and 7 as `(fp_text reference "R1" ...)`.
fn footprint_reference(node: &Expr) -> Option<String> {
    node.children().iter().find_map(|item| {
        let kind = item.children().get(1)?.atom()?;
        match item.head()? {
            "property" if kind == "Reference" => Some(item.children().get(2)?.atom()?.to_string()),
            "fp_text" if kind == "reference" => Some(item.children().get(2)?.atom()?.to_string()),
            _ => None,
        }
    })
}

/// A footprint's own identity: its `uuid` (KiCad 8 on) or `tstamp` (KiCad
/// 5 to 7), which stays with it when it moves and tells apart footprints
/// that share a reference (a panel's copies).
fn footprint_uuid(node: &Expr) -> Option<String> {
    node.children().iter().find_map(|item| match item.head()? {
        "uuid" | "tstamp" => Some(item.children().get(1)?.atom()?.to_string()),
        _ => None,
    })
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn net_name_from_form(node: &Expr) -> Option<&str> {
    if node.head() != Some("net") {
        return None;
    }
    let fields = node.children();
    match fields.len() {
        3.. => fields.get(2)?.atom(),
        2 => fields.get(1)?.atom(),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Point(i64, i64);

fn deterministic_uuid_for(identity: &str) -> String {
    let digest = Sha256::digest(identity);
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

fn count_head(root: &Expr, head: &str) -> usize {
    root.children()
        .iter()
        .filter(|item| item.head() == Some(head))
        .count()
}

fn root_head(items: &[Expr]) -> Option<&str> {
    items.first()?.atom()
}

fn parse(source: &str) -> Result<Expr, String> {
    let mut parser = Parser { source, offset: 0 };
    let expression = parser.expression()?;
    parser.whitespace();
    if parser.offset != source.len() {
        return Err(format!("trailing input at byte {}", parser.offset));
    }
    Ok(expression)
}

struct Parser<'a> {
    source: &'a str,
    offset: usize,
}

impl Parser<'_> {
    fn expression(&mut self) -> Result<Expr, String> {
        self.whitespace();
        match self.peek() {
            Some('(') => self.list(),
            Some(_) => self.atom(),
            None => Err("unexpected end of input".into()),
        }
    }

    fn list(&mut self) -> Result<Expr, String> {
        self.bump();
        let mut items = Vec::new();
        loop {
            self.whitespace();
            match self.peek() {
                Some(')') => {
                    self.bump();
                    return Ok(Expr::List(items));
                }
                // KiCad reads a teardrop setting's opening parenthesis as
                // optional (PCB_IO_KICAD_SEXPR_PARSER's
                // parseTEARDROP_PARAMETERS: a key after a missing "(" ends
                // at its ")" as usual), and a pad written as
                // "(curved_edges no)filter_ratio 0.9)" loads; read as a
                // list, the stray ")" closed the pad and then the board
                // (RoyalBlue54L-Feather: "trailing input at byte 14111").
                Some(character)
                    if character != '('
                        && matches!(items.first(), Some(Expr::Atom(head)) if head == "teardrops") =>
                {
                    items.push(self.list_without_opening()?)
                }
                Some(_) => items.push(self.expression()?),
                None => return Err("unterminated list".into()),
            }
        }
    }

    /// A list whose "(" is missing: its items up to and with its ")".
    fn list_without_opening(&mut self) -> Result<Expr, String> {
        let mut items = Vec::new();
        loop {
            self.whitespace();
            match self.peek() {
                Some(')') => {
                    self.bump();
                    return Ok(Expr::List(items));
                }
                Some(_) => items.push(self.expression()?),
                None => return Err("unterminated list".into()),
            }
        }
    }

    fn atom(&mut self) -> Result<Expr, String> {
        let start = self.offset;
        if self.peek() == Some('"') {
            self.bump();
            let mut escaped = false;
            while let Some(character) = self.bump() {
                if character == '"' && !escaped {
                    return Ok(Expr::Atom(self.source[start..self.offset].to_string()));
                }
                escaped = character == '\\' && !escaped;
                if character != '\\' {
                    escaped = false;
                }
            }
            return Err("unterminated string".into());
        }
        while let Some(character) = self.peek() {
            if character.is_whitespace() || matches!(character, '(' | ')') {
                break;
            }
            self.bump();
        }
        if self.offset == start {
            Err(format!("expected atom at byte {}", self.offset))
        } else {
            Ok(Expr::Atom(self.source[start..self.offset].to_string()))
        }
    }

    fn whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    fn peek(&self) -> Option<char> {
        self.source[self.offset..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.offset += character.len_utf8();
        Some(character)
    }
}

fn unquote(atom: &str) -> &str {
    atom.strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(atom)
}

/// The text of `expression` as written to a file: a footprint's graphics
/// in the order of the file they were read from when only the canonical
/// order moved them ([`canonical_order::source_order`]).
fn encode(expression: &Expr) -> String {
    let mut output = String::new();
    encode_into(expression, 0, &mut output, true);
    output
}

/// The text of `expression` as it is held, footprint graphics in the
/// canonical order (for keys and digests).
fn encode_canonical(expression: &Expr) -> String {
    let mut output = String::new();
    encode_into(expression, 0, &mut output, false);
    output
}

fn encode_into(expression: &Expr, depth: usize, output: &mut String, source_order: bool) {
    match expression {
        Expr::Atom(atom) => output.push_str(atom),
        Expr::List(items) => {
            let order = if source_order && expression.head() == Some("footprint") {
                canonical_order::source_order(expression)
            } else {
                None
            };
            // Embedded data (an image's base64) one string a line, as
            // KiCad writes it: KiCad reads lines of at most 1 MB, and an
            // image written on one 24 MB line made the board unreadable
            // (stonedDiscord's EMP).
            let data = expression.head() == Some("data");
            output.push('(');
            for index in 0..items.len() {
                let item = &items[order.as_ref().map_or(index, |order| order[index])];
                if index > 0 {
                    if matches!(item, Expr::List(_)) || (data && index > 1) {
                        output.push('\n');
                        output.push_str(&"  ".repeat(depth + 1));
                    } else {
                        output.push(' ');
                    }
                }
                encode_into(item, depth + 1, output, source_order);
            }
            output.push(')');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_cache_hashes_design_inputs_and_ignores_reports() {
        assert_eq!(kicad_verifier_implementation_sha256().len(), 64);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-kicad-cache-input-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("board.kicad_pcb"), "(kicad_pcb)\n").unwrap();
        fs::write(directory.join("board.kicad_pro"), "{}\n").unwrap();
        fs::write(
            directory.join("sym-lib-table"),
            r#"(sym_lib_table (lib (uri "${KIPRJMOD}/Generated.kicad_sym")))"#,
        )
        .unwrap();
        let digest = match kicad_cache_input_sha256(&directory).unwrap() {
            KiCadCacheInput::Cacheable(digest) => digest,
            KiCadCacheInput::Bypassed(reason) => panic!("unexpected bypass: {reason}"),
        };
        fs::write(directory.join("erc.json"), "generated report\n").unwrap();
        let after_report = match kicad_cache_input_sha256(&directory).unwrap() {
            KiCadCacheInput::Cacheable(digest) => digest,
            KiCadCacheInput::Bypassed(reason) => panic!("unexpected bypass: {reason}"),
        };
        assert_eq!(digest, after_report);
        fs::write(directory.join("board.kicad_pcb"), "(kicad_pcb (zone))\n").unwrap();
        let after_board = match kicad_cache_input_sha256(&directory).unwrap() {
            KiCadCacheInput::Cacheable(digest) => digest,
            KiCadCacheInput::Bypassed(reason) => panic!("unexpected bypass: {reason}"),
        };
        assert_ne!(digest, after_board);

        fs::write(
            directory.join("fp-lib-table"),
            r#"(fp_lib_table (lib (uri "/external/library.pretty")))"#,
        )
        .unwrap();
        assert!(matches!(
            kicad_cache_input_sha256(&directory).unwrap(),
            KiCadCacheInput::Bypassed(reason) if reason.contains("external library")
        ));
        fs::remove_dir_all(directory).unwrap();
    }

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
    fn cold_board_rewrite_removes_copper_but_retains_rule_areas_and_placement() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-strip-copper-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let board = directory.join("board.kicad_pcb");
        fs::write(
            &board,
            r#"(kicad_pcb
                (net 0 "")
                (net 1 "SIGNAL")
                (footprint "test" (at 12 34 90)
                  (property "Reference" "J1")
                  (pad "1" smd rect (at 0 0) (size 1 1)
                    (layers "F.Cu") (net 1 "SIGNAL")))
                (segment (start 12 34) (end 14 34) (width 0.25)
                  (layer "F.Cu") (net 1))
                (arc (start 14 34) (mid 15 35) (end 14 36) (width 0.25)
                  (layer "F.Cu") (net 1))
                (via (at 14 36) (size 0.7) (drill 0.3)
                  (layers "F.Cu" "B.Cu") (net 1))
                (gr_text "COPPER" (at 8 8) (layer "F.Cu"))
                (gr_text "SILK" (at 8 9) (layer "F.SilkS"))
                (zone (net 1) (net_name "SIGNAL") (layer "B.Cu")
                  (polygon (pts (xy 0 0) (xy 2 0) (xy 2 2) (xy 0 2))))
                (zone (net 0) (net_name "") (layers "F&B.Cu")
                  (keepout (tracks not_allowed) (vias not_allowed))
                  (polygon (pts (xy 4 4) (xy 6 4) (xy 6 6) (xy 4 6)))))"#,
        )
        .unwrap();

        let report = write_kicad_board_without_copper(&board, &board).unwrap();
        assert_eq!(report.removed_segments, 1);
        assert_eq!(report.removed_arcs, 1);
        assert_eq!(report.removed_vias, 1);
        assert_eq!(report.removed_copper_zones, 1);
        assert_eq!(report.removed_copper_graphics, 1);
        assert_eq!(report.retained_rule_areas, 1);
        assert_eq!(report.before.zones, 2);
        assert_eq!(report.before.copper_zones, 1);
        assert_eq!(report.before.rule_areas, 1);
        assert_eq!(report.after.zones, 1);
        assert_eq!(report.after.copper_zones, 0);
        assert_eq!(report.after.rule_areas, 1);
        assert_eq!(report.before.copper_graphics, 1);
        assert_eq!(report.after.copper_graphics, 0);
        assert_eq!(report.before.footprints, report.after.footprints);
        assert_eq!(
            report.before.component_placement_sha256,
            report.after.component_placement_sha256
        );
        let stripped = fs::read_to_string(&board).unwrap();
        assert!(stripped.contains("(keepout"));
        assert!(stripped.contains("SILK"));
        assert!(!stripped.contains("COPPER"));
        fs::remove_dir_all(directory).unwrap();
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
    fn footprint_link_normalization_changes_only_schematic_library_links() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-footprint-link-normalization-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let board = directory.join("board.kicad_pcb");
        let schematic = directory.join("board.kicad_sch");
        fs::write(
            &board,
            r#"(kicad_pcb
                (footprint "Local:R" (at 4 5 90)
                  (property "Reference" "R1")
                  (pad "1" thru_hole circle (at 0 0) (size 1 1)
                    (drill 0.5) (layers "*.Cu" "*.Mask")))
                (gr_rect (start 0 0) (end 10 10) (layer "Edge.Cuts")))"#,
        )
        .unwrap();
        fs::write(
            &schematic,
            r#"(kicad_sch
                (symbol (lib_id "Device:R") (at 1 2 0)
                  (property "Reference" "R1")
                  (property "Footprint" "Device:R")))"#,
        )
        .unwrap();
        let board_before = fs::read(&board).unwrap();

        let report = normalize_kicad_footprint_links(&directory, "board").unwrap();

        assert_eq!(report.updated_references.len(), 1);
        assert_eq!(report.schema_version, 2);
        assert_eq!(report.schematic_files.len(), 1);
        assert_eq!(
            report.updated_references[0].schematic_path,
            "board.kicad_sch"
        );
        assert_eq!(report.updated_references[0].reference, "R1");
        assert_eq!(report.updated_references[0].schematic_before, "Device:R");
        assert_eq!(report.updated_references[0].board_link, "Local:R");
        assert_ne!(
            report.schematic_before_sha256,
            report.schematic_after_sha256
        );
        assert_eq!(fs::read(&board).unwrap(), board_before);
        assert!(
            fs::read_to_string(&schematic)
                .unwrap()
                .contains("(property \"Footprint\" \"Local:R\")")
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn footprint_link_normalization_traverses_referenced_schematic_hierarchy() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-hierarchical-footprint-link-normalization-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("board.kicad_pcb"),
            r#"(kicad_pcb
                (footprint "Local:R" (at 4 5) (property "Reference" "R1"))
                (footprint "Local:C" (at 6 5) (property "Reference" "C2"))
                (gr_rect (start 0 0) (end 10 10) (layer "Edge.Cuts")))"#,
        )
        .unwrap();
        fs::write(
            directory.join("board.kicad_sch"),
            r#"(kicad_sch
                (symbol (property "Reference" "R1") (property "Footprint" "Device:R"))
                (sheet (property "Sheetfile" "child.kicad_sch")))"#,
        )
        .unwrap();
        fs::write(
            directory.join("child.kicad_sch"),
            r#"(kicad_sch
                (symbol (property "Reference" "C2") (property "Footprint" "Device:C")))"#,
        )
        .unwrap();
        let board_before = fs::read(directory.join("board.kicad_pcb")).unwrap();

        let report = normalize_kicad_footprint_links(&directory, "board").unwrap();

        assert_eq!(report.updated_references.len(), 2);
        assert_eq!(report.schematic_files.len(), 2);
        assert_eq!(
            fs::read(directory.join("board.kicad_pcb")).unwrap(),
            board_before
        );
        assert!(
            fs::read_to_string(directory.join("child.kicad_sch"))
                .unwrap()
                .contains("(property \"Footprint\" \"Local:C\")")
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
    fn four_closed_edge_lines_define_the_same_rectangular_bounds_and_area() {
        let pcb = parse(
            r#"(kicad_pcb
                (gr_line (start 10 20) (end 30 20) (layer "Edge.Cuts"))
                (gr_line (start 30 40) (end 10 40) (layer "Edge.Cuts"))
                (gr_line (start 10 40) (end 10 20) (layer "Edge.Cuts"))
                (gr_line (start 30 20) (end 30 40) (layer "Edge.Cuts")))"#,
        )
        .unwrap();

        assert_eq!(
            rectangular_board_bounds(&pcb).unwrap(),
            Some([10.0, 20.0, 30.0, 40.0])
        );
        assert_eq!(rectangular_board_outline_area(&pcb).unwrap(), Some(400.0));
    }

    #[test]
    fn collinearly_split_edge_lines_still_define_a_rectangle() {
        let pcb = parse(
            r#"(kicad_pcb
                (gr_line (start 10 20) (end 17 20) (layer "Edge.Cuts"))
                (gr_line (start 17 20) (end 30 20) (layer "Edge.Cuts"))
                (gr_line (start 30 20) (end 30 40) (layer "Edge.Cuts"))
                (gr_line (start 30 40) (end 10 40) (layer "Edge.Cuts"))
                (gr_line (start 10 40) (end 10 20) (layer "Edge.Cuts")))"#,
        )
        .unwrap();

        assert_eq!(
            rectangular_board_bounds(&pcb).unwrap(),
            Some([10.0, 20.0, 30.0, 40.0])
        );
        assert_eq!(rectangular_board_outline_area(&pcb).unwrap(), Some(400.0));
    }

    #[test]
    fn non_convex_closed_line_outline_keeps_exact_area_and_blocks_corner_cutting() {
        let pcb = parse(
            r#"(kicad_pcb
                (gr_line (start 0 0) (end 10 0) (layer "Edge.Cuts"))
                (gr_line (start 10 0) (end 10 10) (layer "Edge.Cuts"))
                (gr_line (start 10 10) (end 6 10) (layer "Edge.Cuts"))
                (gr_line (start 6 10) (end 6 14) (layer "Edge.Cuts"))
                (gr_line (start 6 14) (end 4 14) (layer "Edge.Cuts"))
                (gr_line (start 4 14) (end 4 10) (layer "Edge.Cuts"))
                (gr_line (start 4 10) (end 0 10) (layer "Edge.Cuts"))
                (gr_line (start 0 10) (end 0 0) (layer "Edge.Cuts")))"#,
        )
        .unwrap();

        let outline = board_outline(&pcb).unwrap().unwrap();
        assert_eq!(outline.bounds, [0.0, 0.0, 10.0, 14.0]);
        assert!((outline.area() - 108.0).abs() < 1.0e-12);
        assert_eq!(board_outline_area(&pcb).unwrap(), Some(108.0));
        assert_eq!(rectangular_board_bounds(&pcb).unwrap(), None);
        assert!(outline.contains_point_with_clearance([5.0, 12.0], 0.1));
        assert!(!outline.contains_point_with_clearance([2.0, 12.0], 0.1));
        assert!(outline.contains_segment_with_clearance([5.0, 9.0], [5.0, 12.0], 0.1));
        assert!(!outline.contains_segment_with_clearance([2.0, 9.0], [5.0, 12.0], 0.1));
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

    #[test]
    fn copper_application_uses_exact_pad_net_spelling() {
        let mut pcb = parse(
            r#"(kicad_pcb
            (footprint "test"
              (pad "1" smd (net "GND"))
              (pad "2" smd (net "/DATA"))
              (pad "3" smd (net "N\"Q")))
            (segment (net "/GND"))
            (via (net "/GND"))
            (arc (net "/DATA"))
            (zone (net "/GND") (net_name "/GND"))
            (segment (net "/N\"Q"))
            (segment (net "ORPHAN")))"#,
        )
        .unwrap();
        let names = unambiguous_pad_net_names(&pcb).unwrap();
        canonicalize_copper_net_names(&mut pcb, &names);
        let copper = pcb
            .children()
            .iter()
            .filter(|n| matches!(n.head(), Some("segment" | "via" | "arc" | "zone")))
            .collect::<Vec<_>>();
        assert_eq!(
            copper
                .iter()
                .map(|n| node_net(n).unwrap())
                .collect::<Vec<_>>(),
            vec!["GND", "GND", "/DATA", "GND", r#"N\"Q"#, "ORPHAN"]
        );
        assert_eq!(
            copper[3].child("net_name").unwrap().children()[1].atom(),
            Some("GND")
        );
        // Raw escaped atoms must survive a complete text round trip.
        let reparsed = parse(&encode(&pcb)).unwrap();
        assert_eq!(encode(&reparsed), encode(&pcb));
    }

    #[test]
    fn colliding_normalized_pad_names_stay_two_nets_when_the_board_declares_both() {
        // A KiCad 10 board has no net table: its pads name both nets
        // (gametank's expansion-port board: "/A15" and "A15"), and they
        // stay two, through the copper written back.
        let mut pcb = parse(
            r#"(kicad_pcb (version 20260206) (footprint "test"
            (pad "1" smd (net "A15")) (pad "2" smd (net "/A15")) (pad "3" smd (net "/D0"))))"#,
        )
        .unwrap();
        let names = unambiguous_pad_net_names(&pcb).unwrap();
        assert_eq!(names.len(), 3);
        assert_eq!(normalize_net("/A15"), "/A15");
        assert_eq!(normalize_net("/D0"), "D0");
        let segment = |connection: &str| SupplementalSegment {
            connection: connection.into(),
            start: [0.0, 0.0],
            end: [1.0, 0.0],
            width: 0.2,
            layer: "F.Cu".into(),
        };
        if let Expr::List(items) = &mut pcb {
            for connection in ["A15", "/A15", "D0"] {
                items.push(supplemental_segment_expr(&segment(connection)));
            }
            items.push(supplemental_via_expr(&SupplementalVia {
                connection: "A15".into(),
                at: [0.0, 0.0],
                size: 0.6,
                drill: 0.3,
                layers: ["F.Cu".into(), "B.Cu".into()],
            }));
        }
        canonicalize_copper_net_names(&mut pcb, &names);
        let written: Vec<&str> = pcb
            .children()
            .iter()
            .filter(|item| matches!(item.head(), Some("segment" | "via")))
            .filter_map(node_net)
            .collect();
        assert_eq!(written, ["A15", "/A15", "/D0", "A15"]);
        // With both declared (a global and a root-sheet label), two nets.
        let pcb = parse(
            r#"(kicad_pcb (net 1 "SIGNAL") (net 2 "/SIGNAL") (footprint "test"
            (pad "1" smd (net "SIGNAL")) (pad "2" smd (net "/SIGNAL"))))"#,
        )
        .unwrap();
        let names = unambiguous_pad_net_names(&pcb).unwrap();
        assert_eq!(names.len(), 2);
        assert_eq!(normalize_net("/SIGNAL"), "/SIGNAL");
        assert_eq!(normalize_net("//SIGNAL"), "/SIGNAL");
        note_net_names(&parse("(kicad_pcb)").unwrap());
        assert_eq!(normalize_net("/SIGNAL"), "SIGNAL");
    }

    #[test]
    fn a_teardrop_setting_without_its_opening_parenthesis_reads_as_kicad_reads_it() {
        // RoyalBlue54L-Feather's pads: "(curved_edges no)filter_ratio 0.9)".
        // KiCad takes the "(" of a teardrop setting as optional; read as
        // a list, the stray ")" closed the pad and then the board.
        let board = parse(
            r#"(kicad_pcb (footprint "C" (pad "1" smd roundrect (at -0.48 0)
              (teardrops (best_length_ratio 0.5) (max_length 1) (best_width_ratio 1)
                (max_width 2) (curved_edges no)filter_ratio 0.9)
                (enabled yes) (allow_two_segments yes) (prefer_zone_connections yes))
              (uuid "a")) (uuid "b")) (gr_line (start 0 0) (end 1 0)))"#,
        )
        .unwrap();
        let footprint = board.child("footprint").unwrap();
        let teardrops = footprint.child("pad").unwrap().child("teardrops").unwrap();
        assert_eq!(form_atom(teardrops, "filter_ratio", 1), Some("0.9"));
        assert_eq!(form_atom(teardrops, "enabled", 1), Some("yes"));
        assert!(footprint.child("uuid").is_some());
        assert!(board.child("gr_line").is_some());
        // Written back with its parenthesis.
        assert!(encode(teardrops).contains("(filter_ratio 0.9)"));
        // Elsewhere a stray atom is still an atom.
        assert!(parse("(kicad_pcb (x 1) y)").is_ok());
    }

    #[test]
    fn embedded_data_is_written_one_string_a_line() {
        // An image's base64 (stonedDiscord's EMP: 24 MB on one line, which
        // KiCad cannot read back), as KiCad writes it.
        let image = parse(
            r#"(image (at 84.7 106.25) (layer "F.Paste")
              (data "/9j/4AAQ" "CwsNDhIQ"
                "FBQUFBQU") (uuid "u"))"#,
        )
        .unwrap();
        let text = encode(&image);
        assert!(
            text.contains("(data \"/9j/4AAQ\"\n    \"CwsNDhIQ\"\n    \"FBQUFBQU\")"),
            "{text}"
        );
        assert_eq!(parse(&text).unwrap(), image);
    }

    #[test]
    fn parser_round_trips_strings_and_nested_lists() {
        let source = "(root (name \"a b\\\"c\") (xy 1.25 -2))";
        assert_eq!(
            parse(&encode(&parse(source).unwrap())).unwrap(),
            parse(source).unwrap()
        );
    }

    #[test]
    fn continuous_obstacle_check_rejects_crossing_shortcuts() {
        let obstacle = ObstacleGeometry::Circle {
            center: [5.0, 5.0],
            radius: 0.5,
        };
        assert!(obstacle.intersects_segment([1.0, 5.0], [9.0, 5.0], 0.2));
        assert!(!obstacle.intersects_segment([1.0, 7.0], [9.0, 7.0], 0.2));
    }

    #[test]
    fn custom_polygon_geometry_is_not_reduced_to_its_anchor() {
        let pad = parse(
            r#"(pad "2" smd custom
                (at 0 0)
                (size 0.3 0.3)
                (primitives
                    (gr_poly
                        (pts (xy 0.5 -0.75) (xy -0.65 -0.75) (xy -0.15 0)
                             (xy -0.65 0.75) (xy 0.5 0.75))
                        (width 0)
                        (fill yes))))"#,
        )
        .unwrap();
        let geometry = custom_pad_geometry(
            &pad,
            [10.0, 20.0],
            0.0,
            ObstacleGeometry::Rectangle {
                center: [10.0, 20.0],
                half_size: [0.15, 0.15],
                angle_degrees: 0.0,
            },
        )
        .unwrap();
        assert!(geometry.contains([10.45, 20.5], 0.0));
        assert!(!geometry.contains([9.0, 20.7], 0.0));
        assert_eq!(geometry.aabb().minimum, [9.35, 19.25]);
        assert_eq!(geometry.aabb().maximum, [10.5, 20.75]);
    }

    #[test]
    fn a_custom_pads_number_box_is_no_copper() {
        // A heatsink pad whose primitives are only the pad editor's number
        // box (gr_bbox), as KiCad 7 writes it: the anchor alone is copper.
        let pad = parse(
            r#"(pad "5" thru_hole custom (at 1.2 -1.5 90) (size 0.4 0.4) (drill 0.2)
                (layers "*.Cu") (options (clearance outline) (anchor rect))
                (primitives
                    (gr_bbox (start -0.75 -0.75) (end 0.75 0.75) (width 0.000001) (fill none))))"#,
        )
        .unwrap();
        let anchor = ObstacleGeometry::Rectangle {
            center: [10.0, 20.0],
            half_size: [0.2, 0.2],
            angle_degrees: 90.0,
        };
        let geometry = custom_pad_geometry(&pad, [10.0, 20.0], 90.0, anchor).unwrap();
        assert!(matches!(geometry, ObstacleGeometry::Rectangle { .. }));
        assert!(!geometry.contains([10.5, 20.5], 0.0));
    }

    #[test]
    fn custom_polygon_follows_its_arcs() {
        // A square with a half circle bulging out to the right.
        let pad = parse(
            r#"(pad "1" smd custom
                (at 0 0)
                (size 0.3 0.3)
                (primitives
                    (gr_poly
                        (pts (xy -1 -1) (arc (start 1 -1) (mid 2 0) (end 1 1)) (xy -1 1))
                        (width 0)
                        (fill yes))))"#,
        )
        .unwrap();
        let geometry = custom_pad_geometry(
            &pad,
            [10.0, 20.0],
            0.0,
            ObstacleGeometry::Rectangle {
                center: [10.0, 20.0],
                half_size: [0.15, 0.15],
                angle_degrees: 0.0,
            },
        )
        .unwrap();
        assert!(geometry.contains([11.9, 20.0], 0.0));
        assert!(!geometry.contains([11.9, 20.9], 0.0));
        assert!((geometry.aabb().maximum[0] - 12.0).abs() < 0.01);
    }

    #[test]
    fn route_graph_normalization_splits_collinear_overlap_and_interior_contacts() {
        let point = |at| KiCadRoutePoint {
            at,
            layer: "F.Cu".into(),
        };
        let branch = |path: Vec<[f64; 2]>| KiCadRouteBranch {
            start_terminal: path[0],
            finish_terminal: *path.last().unwrap(),
            cost: 0,
            expansions: 0,
            path: path.into_iter().map(point).collect(),
        };
        let mut branches = vec![
            branch(vec![[0.0, 0.0], [4.0, 0.0], [5.0, 1.0]]),
            branch(vec![[0.0, 0.0], [2.0, 0.0], [5.0, 2.0]]),
            branch(vec![[-1.0, -1.0], [1.0, 0.0], [1.0, 1.0]]),
        ];

        normalize_selected_route_graph(&mut branches, &[0, 1, 2]);

        assert_eq!(
            branches[0]
                .path
                .iter()
                .map(|point| point.at)
                .collect::<Vec<_>>(),
            vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [4.0, 0.0], [5.0, 1.0]]
        );
        assert_eq!(
            branches[1]
                .path
                .iter()
                .map(|point| point.at)
                .collect::<Vec<_>>(),
            vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [5.0, 2.0]]
        );
    }

    #[test]
    fn route_normalization_preserves_an_already_connected_terminal() {
        let mut branches = vec![KiCadRouteBranch {
            start_terminal: [2.0, 3.0],
            finish_terminal: [2.0, 3.0],
            cost: 0,
            expansions: 0,
            path: vec![KiCadRoutePoint {
                at: [2.0, 3.0],
                layer: "F.Cu".into(),
            }],
        }];
        normalize_selected_route_graph(&mut branches, &[0]);
        assert_eq!(branches[0].path.len(), 1);
        assert_eq!(branches[0].path[0].at, [2.0, 3.0]);
        assert_eq!(branches[0].path[0].layer, "F.Cu");
    }

    #[test]
    fn native_layer_render_config_is_fail_closed() {
        assert!(validate_layer_render_config(&KiCadLayerRenderConfig::default()).is_ok());
        let legacy: KiCadLayerRenderConfig = serde_json::from_str(
            r#"{"width": 1000, "height": 750, "quality": "basic", "background": "opaque"}"#,
        )
        .unwrap();
        assert!(legacy.show_rule_areas);
        assert!(!legacy.show_copper_zones);
        let invalid = KiCadLayerRenderConfig {
            width: 0,
            quality: "cinematic".into(),
            ..KiCadLayerRenderConfig::default()
        };
        assert_eq!(
            validate_layer_render_config(&invalid).unwrap_err(),
            "KiCad render width and height must be within 1..=8192"
        );
        let invalid = KiCadLayerRenderConfig {
            quality: "cinematic".into(),
            ..KiCadLayerRenderConfig::default()
        };
        assert!(
            validate_layer_render_config(&invalid)
                .unwrap_err()
                .contains("unsupported KiCad render quality")
        );
    }

    #[test]
    fn render_overlay_exposes_rule_areas_without_mutating_source() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-kicad-rule-area-render-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let source_path = directory.join("source.kicad_pcb");
        let render_path = directory.join("render.kicad_pcb");
        let source = r#"(kicad_pcb
          (zone (layer "F.Cu")
            (keepout (tracks not_allowed) (vias not_allowed))
            (polygon (pts (xy 1 1) (xy 4 1) (xy 4 4) (xy 1 4))))
          (zone (layers "F&B.Cu")
            (keepout (tracks not_allowed) (vias not_allowed))
            (polygon (pts (xy 5 1) (xy 8 1) (xy 8 4) (xy 5 4))))
          (zone (layer "B.Cu")
            (keepout (tracks not_allowed) (vias not_allowed))
            (polygon (pts (xy 9 1) (xy 12 1) (xy 12 4) (xy 9 4))))
          (zone (net_name "/GND") (layer "F.Cu")
            (polygon (pts (xy 1 5) (xy 12 5) (xy 12 8) (xy 1 8)))))"#;
        fs::write(&source_path, source).unwrap();

        let config = KiCadLayerRenderConfig {
            show_copper_zones: true,
            ..KiCadLayerRenderConfig::default()
        };
        assert_eq!(
            prepare_layer_render_board(&source_path, &render_path, &config).unwrap(),
            KiCadLayerRenderOverlayCounts {
                rule_areas: 4,
                copper_zones: 1
            }
        );
        assert_eq!(fs::read_to_string(&source_path).unwrap(), source);
        let rendered = parse(&fs::read_to_string(&render_path).unwrap()).unwrap();
        let overlays = rendered
            .children()
            .iter()
            .filter(|item| item.head() == Some("gr_poly"))
            .collect::<Vec<_>>();
        assert_eq!(overlays.len(), 5);
        assert_eq!(
            overlays
                .iter()
                .filter(|item| form_atom(item, "layer", 1) == Some("F.SilkS"))
                .count(),
            3
        );
        assert_eq!(
            overlays
                .iter()
                .filter(|item| form_atom(item, "layer", 1) == Some("B.SilkS"))
                .count(),
            2
        );
        assert!(overlays.iter().all(|item| {
            item.child("fill").and_then(|fill| fill.children().get(1))
                == Some(&Expr::Atom("no".into()))
        }));
        let labels = rendered
            .children()
            .iter()
            .filter(|item| item.head() == Some("gr_text"))
            .collect::<Vec<_>>();
        assert_eq!(labels.len(), 1);
        assert_eq!(
            labels[0].children().get(1).and_then(Expr::atom),
            Some("COPPER ZONE /GND")
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
