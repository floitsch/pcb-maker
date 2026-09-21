// Copyright (C) 2026 Toit contributors.

use super::{LadderDeclaration, deterministic_uuid_for, encode, parse};
use layout_trace_model::{
    geometry::{add, rotate_degrees},
    model::{Component, CopperShape, Movement, Problem, Rect, Rotation, Vec2},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

/// A placement selected by an initializer before creating the native KiCad
/// template. Keeping this type independent from `pcb-placement` avoids making
/// the KiCad adapter depend on one particular placement producer.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticKiCadPose {
    pub component: String,
    pub position: Vec2,
    pub rotation_degrees: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SemanticKiCadTemplateConfig {
    pub board_id: String,
    /// Ordered electrical-net identities. Empty uses first appearance in the
    /// semantic input. Every electrical identity must occur exactly once.
    pub connection_order: Vec<String>,
    /// Native KiCad offset. Semantic coordinates are otherwise preserved so a
    /// route/candidate can cross the boundary without an undocumented scale or
    /// reflection.
    pub board_origin_mm: [f64; 2],
    /// Extra native board material around the semantic bounds. Some imported
    /// fixtures place copper exactly on their abstract boundary; retaining a
    /// small margin makes that geometry manufacturable without moving it.
    pub board_edge_margin_mm: f64,
    pub courtyard_clearance_mm: f64,
    pub minimum_annular_ring_mm: f64,
    pub through_hole_drill_ratio: f64,
    /// Diameter used when importing legacy semantic pins that declare only an
    /// attachment point and no explicit pad geometry.
    pub implicit_pad_diameter_mm: f64,
    /// Explicit copper clearance for the generated native project (board
    /// minimum and Default netclass). None preserves KiCad's defaults.
    pub copper_clearance_mm: Option<f64>,
    /// Explicit copper-to-board-edge clearance. Semantic routing uses
    /// `problem.rules.clearance` at its boundary; None retains KiCad's default.
    pub copper_edge_clearance_mm: Option<f64>,
    /// Fabrication minimum, independent of the width selected by the router.
    pub minimum_trace_width_mm: Option<f64>,
}

impl Default for SemanticKiCadTemplateConfig {
    fn default() -> Self {
        Self {
            board_id: "semantic-board".into(),
            connection_order: Vec::new(),
            board_origin_mm: [20.0, 20.0],
            board_edge_margin_mm: 1.0,
            courtyard_clearance_mm: 0.25,
            minimum_annular_ring_mm: 0.25,
            through_hole_drill_ratio: 0.6,
            implicit_pad_diameter_mm: 1.5,
            copper_clearance_mm: None,
            copper_edge_clearance_mm: None,
            minimum_trace_width_mm: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SemanticKiCadTemplateReport {
    pub board_id: String,
    pub semantic_components: usize,
    pub schematic_components: usize,
    pub board_only_components: usize,
    pub electrical_connections: usize,
    pub terminal_assignments: usize,
    pub schematic_template: PathBuf,
    pub pcb_template: PathBuf,
    pub project_template: PathBuf,
    pub declaration: PathBuf,
}

#[derive(Clone, Debug)]
struct ElectricalConnection {
    terminals: BTreeSet<(String, String)>,
}

#[derive(Clone, Copy, Debug)]
struct SymbolPinPlacement {
    endpoint: Vec2,
    stub_end: Vec2,
    angle: i32,
}

pub fn write_semantic_kicad_ladder_template(
    problem: &Problem,
    poses: &[SemanticKiCadPose],
    config: &SemanticKiCadTemplateConfig,
    output_directory: &Path,
) -> Result<SemanticKiCadTemplateReport, String> {
    problem.check_schema()?;
    validate_config(config)?;
    let poses = checked_poses(problem, poses)?;
    let (connections, source_order) = collect_connections(problem)?;
    let connection_order = checked_connection_order(config, &connections, &source_order)?;
    let references = assign_references(problem);
    let terminal_connections = terminal_connection_map(&connections)?;

    let schematic = build_schematic(
        problem,
        &references,
        &terminal_connections,
        &config.board_id,
    )?;
    let pcb = build_pcb(
        problem,
        &poses,
        &references,
        &terminal_connections,
        config,
        &connection_order,
    )?;

    fs::create_dir_all(output_directory).map_err(|error| {
        format!(
            "failed to create semantic KiCad template directory {}: {error}",
            output_directory.display()
        )
    })?;
    let schematic_name = format!("{}-full.kicad_sch", config.board_id);
    let pcb_name = format!("{}-full.kicad_pcb", config.board_id);
    let project_name = format!("{}-full.kicad_pro", config.board_id);
    let schematic_path = output_directory.join(&schematic_name);
    let pcb_path = output_directory.join(&pcb_name);
    let project_path = output_directory.join(&project_name);
    let declaration_path = output_directory.join("declaration.json");
    write_encoded(&schematic_path, &schematic)?;
    write_encoded(&pcb_path, &pcb)?;
    let mut project = if let Some(clearance) = config.copper_clearance_mm {
        serde_json::json!({
            "board": {"design_settings": {"rules": {"min_clearance": clearance}}},
            "net_settings": {"classes": [{"name": "Default", "clearance": clearance}]}
        })
    } else {
        serde_json::json!({})
    };
    if let Some(width) = config.minimum_trace_width_mm {
        project["board"]["design_settings"]["rules"]["min_track_width"] = serde_json::json!(width);
    }
    if let Some(clearance) = config.copper_edge_clearance_mm {
        project["board"]["design_settings"]["rules"]["min_copper_edge_clearance"] =
            serde_json::json!(clearance);
    }
    fs::write(
        &project_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&project).map_err(|error| error.to_string())?
        ),
    )
    .map_err(|error| format!("failed to write {}: {error}", project_path.display()))?;

    let declaration = LadderDeclaration {
        schema_version: 1,
        board_id: config.board_id.clone(),
        schematic_template: PathBuf::from(schematic_name),
        pcb_template: PathBuf::from(pcb_name),
        project_template: Some(PathBuf::from(project_name)),
        connections: connection_order,
        route_vertex_overrides: Vec::new(),
        supplemental_segments: Vec::new(),
        supplemental_vias: Vec::new(),
        replacement_route_candidates: Vec::new(),
        connection_zone_replacements: Vec::new(),
    };
    fs::write(
        &declaration_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&declaration).map_err(|error| error.to_string())?
        ),
    )
    .map_err(|error| format!("failed to write {}: {error}", declaration_path.display()))?;

    let schematic_components = problem
        .components
        .iter()
        .filter(|component| !component.pins.is_empty())
        .count();
    let report = SemanticKiCadTemplateReport {
        board_id: config.board_id.clone(),
        semantic_components: problem.components.len(),
        schematic_components,
        board_only_components: problem.components.len() - schematic_components,
        electrical_connections: connections.len(),
        terminal_assignments: terminal_connections.len(),
        schematic_template: schematic_path,
        pcb_template: pcb_path,
        project_template: project_path,
        declaration: declaration_path,
    };
    let report_path = output_directory.join("template-generation.json");
    fs::write(
        &report_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
        ),
    )
    .map_err(|error| format!("failed to write {}: {error}", report_path.display()))?;
    Ok(report)
}

fn validate_config(config: &SemanticKiCadTemplateConfig) -> Result<(), String> {
    if config
        .copper_edge_clearance_mm
        .is_some_and(|value| !value.is_finite() || value < 0.0)
    {
        return Err("native copper edge clearance must be finite and nonnegative".into());
    }
    if config
        .minimum_trace_width_mm
        .is_some_and(|value| !value.is_finite() || value <= 0.0)
    {
        return Err("native minimum trace width must be finite and positive".into());
    }
    if config
        .copper_clearance_mm
        .is_some_and(|value| !value.is_finite() || value < 0.0)
    {
        return Err("native copper clearance must be finite and nonnegative".into());
    }
    if config.board_id.is_empty()
        || !config
            .board_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("semantic KiCad board_id must use ASCII letters, digits, '-' or '_'".into());
    }
    if config
        .board_origin_mm
        .iter()
        .any(|value| !value.is_finite())
        || !config.board_edge_margin_mm.is_finite()
        || config.board_edge_margin_mm < 0.0
        || !config.courtyard_clearance_mm.is_finite()
        || config.courtyard_clearance_mm < 0.0
        || !config.minimum_annular_ring_mm.is_finite()
        || config.minimum_annular_ring_mm <= 0.0
        || !config.through_hole_drill_ratio.is_finite()
        || !(0.0..1.0).contains(&config.through_hole_drill_ratio)
        || !config.implicit_pad_diameter_mm.is_finite()
        || config.implicit_pad_diameter_mm <= 0.0
    {
        return Err(
            "semantic KiCad template dimensions must be finite and physically positive".into(),
        );
    }
    Ok(())
}

fn checked_poses<'a>(
    problem: &Problem,
    poses: &'a [SemanticKiCadPose],
) -> Result<BTreeMap<&'a str, &'a SemanticKiCadPose>, String> {
    if poses.len() != problem.components.len() {
        return Err(format!(
            "semantic KiCad template needs exactly one pose for each of {} components; got {}",
            problem.components.len(),
            poses.len()
        ));
    }
    let declared: BTreeSet<_> = problem
        .components
        .iter()
        .map(|component| component.id.as_str())
        .collect();
    let mut result = BTreeMap::new();
    for pose in poses {
        if !declared.contains(pose.component.as_str()) {
            return Err(format!(
                "pose references unknown component {}",
                pose.component
            ));
        }
        if !pose.position.x.is_finite()
            || !pose.position.y.is_finite()
            || !pose.rotation_degrees.is_finite()
        {
            return Err(format!(
                "pose for {} contains non-finite values",
                pose.component
            ));
        }
        if result.insert(pose.component.as_str(), pose).is_some() {
            return Err(format!("duplicate pose for component {}", pose.component));
        }
    }
    for id in declared {
        if !result.contains_key(id) {
            return Err(format!("missing pose for component {id}"));
        }
    }
    Ok(result)
}

fn collect_connections(
    problem: &Problem,
) -> Result<(BTreeMap<String, ElectricalConnection>, Vec<String>), String> {
    let mut connections = BTreeMap::<String, ElectricalConnection>::new();
    let mut order = Vec::new();
    for net in &problem.nets {
        let name = net.electrical_net.as_ref().unwrap_or(&net.id);
        let entry = connections.entry(name.clone()).or_insert_with(|| {
            order.push(name.clone());
            ElectricalConnection {
                terminals: BTreeSet::new(),
            }
        });
        entry
            .terminals
            .insert((net.from.component.clone(), net.from.pin.clone()));
        entry
            .terminals
            .insert((net.to.component.clone(), net.to.pin.clone()));
    }
    for net in &problem.electrical_nets {
        let entry = connections.entry(net.id.clone()).or_insert_with(|| {
            order.push(net.id.clone());
            ElectricalConnection {
                terminals: BTreeSet::new(),
            }
        });
        for terminal in &net.terminals {
            entry
                .terminals
                .insert((terminal.component.clone(), terminal.pin.clone()));
        }
    }
    for (name, connection) in &connections {
        if connection.terminals.len() < 2 {
            return Err(format!(
                "electrical connection {name:?} has fewer than two unique terminals"
            ));
        }
    }
    Ok((connections, order))
}

fn checked_connection_order(
    config: &SemanticKiCadTemplateConfig,
    connections: &BTreeMap<String, ElectricalConnection>,
    source_order: &[String],
) -> Result<Vec<String>, String> {
    if config.connection_order.is_empty() {
        return Ok(source_order.to_vec());
    }
    let expected: BTreeSet<_> = connections.keys().cloned().collect();
    let actual: BTreeSet<_> = config.connection_order.iter().cloned().collect();
    if actual.len() != config.connection_order.len() {
        return Err("semantic KiCad connection_order contains duplicates".into());
    }
    if expected != actual {
        let missing: Vec<_> = expected.difference(&actual).cloned().collect();
        let extra: Vec<_> = actual.difference(&expected).cloned().collect();
        return Err(format!(
            "semantic KiCad connection_order does not cover the problem: missing={missing:?}, extra={extra:?}"
        ));
    }
    Ok(config.connection_order.clone())
}

fn terminal_connection_map(
    connections: &BTreeMap<String, ElectricalConnection>,
) -> Result<BTreeMap<(String, String), String>, String> {
    let mut result = BTreeMap::new();
    for (name, connection) in connections {
        for terminal in &connection.terminals {
            if let Some(existing) = result.insert(terminal.clone(), name.clone())
                && existing != *name
            {
                return Err(format!(
                    "terminal {}.{} belongs to both {existing:?} and {name:?}",
                    terminal.0, terminal.1
                ));
            }
        }
    }
    Ok(result)
}

fn assign_references(problem: &Problem) -> BTreeMap<String, String> {
    let mut counters = BTreeMap::<&str, usize>::new();
    let mut result = BTreeMap::new();
    for component in &problem.components {
        let prefix = reference_prefix(&component.id);
        let index = counters.entry(prefix).or_default();
        *index += 1;
        result.insert(component.id.clone(), format!("{prefix}{index}"));
    }
    result
}

fn reference_prefix(id: &str) -> &'static str {
    if id.starts_with("ESP") {
        "U"
    } else if id.starts_with("R_") {
        "R"
    } else if id.starts_with("C_") {
        "C"
    } else if id.starts_with("D_") {
        "D"
    } else if id.starts_with("J_") {
        "J"
    } else if id.starts_with("H_") {
        "H"
    } else {
        "X"
    }
}

fn build_schematic(
    problem: &Problem,
    references: &BTreeMap<String, String>,
    terminal_connections: &BTreeMap<(String, String), String>,
    board_id: &str,
) -> Result<super::Expr, String> {
    let root_uuid = deterministic_uuid_for(&format!("pcb-maker:semantic:{board_id}:schematic"));
    let mut source = format!(
        "(kicad_sch (version 20250114) (generator \"pcb-maker\") (generator_version \"0.1\") (uuid \"{root_uuid}\") (paper \"A2\") (title_block (title \"{} progressive routing benchmark\") (date \"2026-09-02\") (rev \"semantic-v1\") (company \"Toit contributors\") (comment 1 \"Generated from layout-trace semantic geometry\")) (lib_symbols",
        escaped(board_id)
    );
    for component in problem
        .components
        .iter()
        .filter(|component| !component.pins.is_empty())
    {
        source.push_str(&symbol_definition(component)?);
    }
    source.push(')');

    let layouts = schematic_component_layouts(problem);
    for (component, (position, pin_positions)) in problem
        .components
        .iter()
        .filter(|component| !component.pins.is_empty())
        .zip(layouts)
    {
        let reference = &references[&component.id];
        let symbol_uuid = deterministic_uuid_for(&format!(
            "pcb-maker:semantic:{board_id}:symbol:{}",
            component.id
        ));
        let footprint_name = footprint_name(component);
        write!(
            source,
            "(symbol (lib_id \"Generated:{}\") (at {} {} 0) (unit 1) (body_style 1) (exclude_from_sim no) (in_bom yes) (on_board yes) (in_pos_files yes) (dnp no) (uuid \"{}\") (property \"Reference\" \"{}\" (at {} {} 0) (show_name no) (do_not_autoplace no) (effects (font (size 1.27 1.27)))) (property \"Value\" \"{}\" (at {} {} 0) (show_name no) (do_not_autoplace no) (effects (font (size 1.27 1.27)))) (property \"Footprint\" \"Generated:{}\" (at {} {} 0) (hide yes) (show_name no) (do_not_autoplace no) (effects (font (size 1.27 1.27)))) (property \"Datasheet\" \"\" (at {} {} 0) (hide yes) (show_name no) (do_not_autoplace no) (effects (font (size 1.27 1.27))))",
            escaped(&symbol_name(component)),
            number(position.x),
            number(position.y),
            symbol_uuid,
            escaped(reference),
            number(position.x),
            number(position.y - 4.0),
            escaped(&component.id),
            number(position.x),
            number(position.y + 4.0),
            escaped(&footprint_name),
            number(position.x),
            number(position.y),
            number(position.x),
            number(position.y),
        )
        .map_err(|error| error.to_string())?;
        for pin in &component.pins {
            write!(
                source,
                "(pin \"{}\" (uuid \"{}\"))",
                escaped(&pin.id),
                deterministic_uuid_for(&format!(
                    "pcb-maker:semantic:{board_id}:symbol:{}/pin:{}",
                    component.id, pin.id
                ))
            )
            .map_err(|error| error.to_string())?;
        }
        write!(
            source,
            "(instances (project \"{}\" (path \"/{}\" (reference \"{}\") (unit 1)))))",
            escaped(board_id),
            root_uuid,
            escaped(reference)
        )
        .map_err(|error| error.to_string())?;

        for (pin, placement) in component.pins.iter().zip(pin_positions) {
            let key = (component.id.clone(), pin.id.clone());
            if let Some(connection) = terminal_connections.get(&key) {
                write!(
                    source,
                    "(wire (pts (xy {} {}) (xy {} {})) (stroke (width 0) (type default)) (uuid \"{}\"))(label \"{}\" (at {} {} {}) (effects (font (size 1 1))) (uuid \"{}\"))",
                    number(placement.endpoint.x),
                    number(placement.endpoint.y),
                    number(placement.stub_end.x),
                    number(placement.stub_end.y),
                    deterministic_uuid_for(&format!(
                        "pcb-maker:semantic:{board_id}:wire:{connection}:{}:{}",
                        component.id, pin.id
                    )),
                    escaped(connection),
                    number(placement.stub_end.x),
                    number(placement.stub_end.y),
                    placement.angle,
                    deterministic_uuid_for(&format!(
                        "pcb-maker:semantic:{board_id}:label:{connection}:{}:{}",
                        component.id, pin.id
                    )),
                )
                .map_err(|error| error.to_string())?;
            } else {
                write!(
                    source,
                    "(no_connect (at {} {}) (uuid \"{}\"))",
                    number(placement.endpoint.x),
                    number(placement.endpoint.y),
                    deterministic_uuid_for(&format!(
                        "pcb-maker:semantic:{board_id}:no-connect:{}:{}",
                        component.id, pin.id
                    ))
                )
                .map_err(|error| error.to_string())?;
            }
        }
    }
    source.push_str("(sheet_instances (path \"/\" (page \"1\")))(embedded_fonts no))");
    parse(&source)
}

fn symbol_definition(component: &Component) -> Result<String, String> {
    let name = symbol_name(component);
    let rows = component.pins.len().div_ceil(2).max(1);
    let half_height = (rows as f64 * 1.27 + 1.27).max(3.81);
    let mut source = format!(
        "(symbol \"Generated:{}\" (exclude_from_sim no) (in_bom yes) (on_board yes) (in_pos_files yes) (duplicate_pin_numbers_are_jumpers no) (property \"Reference\" \"{}\" (at -12.7 {} 0) (effects (font (size 1.27 1.27)))) (property \"Value\" \"{}\" (at 0 {} 0) (effects (font (size 1.27 1.27)))) (property \"Footprint\" \"\" (at 0 0 0) (hide yes) (effects (font (size 1.27 1.27)))) (property \"Datasheet\" \"\" (at 0 0 0) (hide yes) (effects (font (size 1.27 1.27)))) (symbol \"{}_0_1\" (rectangle (start -12.7 {}) (end 12.7 {}) (stroke (width 0.254) (type default)) (fill (type background)))) (symbol \"{}_1_1\"",
        escaped(&name),
        reference_prefix(&component.id),
        number(half_height + 1.27),
        escaped(&component.id),
        number(half_height + 1.27),
        escaped(&name),
        number(half_height),
        number(-half_height),
        escaped(&name),
    );
    for (index, pin) in component.pins.iter().enumerate() {
        let placement = local_symbol_pin_placement(component.pins.len(), index);
        write!(
            source,
            "(pin passive line (at {} {} {}) (length 2.54) (name \"{}\" (effects (font (size 1.27 1.27)))) (number \"{}\" (effects (font (size 1.27 1.27)))))",
            number(placement.endpoint.x),
            number(placement.endpoint.y),
            if placement.endpoint.x < 0.0 { 0 } else { 180 },
            escaped(&pin.id),
            escaped(&pin.id)
        )
        .map_err(|error| error.to_string())?;
    }
    source.push_str("))");
    Ok(source)
}

fn schematic_component_layout(
    component: &Component,
    index: usize,
    compact_start_y: f64,
) -> (Vec2, Vec<SymbolPinPlacement>) {
    let position = if component.pins.len() >= 30 {
        Vec2::new(
            76.2 + (index % 2) as f64 * 152.4,
            76.2 + (index / 2) as f64 * 63.5,
        )
    } else {
        Vec2::new(
            38.1 + (index % 6) as f64 * 88.9,
            compact_start_y + (index / 6) as f64 * 38.1,
        )
    };
    let pins = (0..component.pins.len())
        .map(|pin| {
            let local = local_symbol_pin_placement(component.pins.len(), pin);
            SymbolPinPlacement {
                // KiCad library-symbol Y coordinates are Cartesian, while
                // schematic-sheet Y increases downward.
                endpoint: Vec2::new(position.x + local.endpoint.x, position.y - local.endpoint.y),
                stub_end: Vec2::new(position.x + local.stub_end.x, position.y - local.stub_end.y),
                angle: local.angle,
            }
        })
        .collect();
    (position, pins)
}

/// Allocate large and compact symbols in separate grids. The indices must be
/// local to each grid: using the component's global index made compact symbols
/// 0 and 2 occupy the same point whenever the first two components were not
/// large devices, electrically joining unrelated labels in KiCad.
fn schematic_component_layouts(problem: &Problem) -> Vec<(Vec2, Vec<SymbolPinPlacement>)> {
    let large_count = problem
        .components
        .iter()
        .filter(|component| !component.pins.is_empty() && component.pins.len() >= 30)
        .count();
    let large_rows = large_count.div_ceil(2);
    let compact_start_y = 139.7 + large_rows.saturating_sub(1) as f64 * 63.5;
    let mut large_index = 0;
    let mut compact_index = 0;
    problem
        .components
        .iter()
        .filter(|component| !component.pins.is_empty())
        .map(|component| {
            let index = if component.pins.len() >= 30 {
                let index = large_index;
                large_index += 1;
                index
            } else {
                let index = compact_index;
                compact_index += 1;
                index
            };
            schematic_component_layout(component, index, compact_start_y)
        })
        .collect()
}

fn local_symbol_pin_placement(pin_count: usize, index: usize) -> SymbolPinPlacement {
    let left_count = pin_count.div_ceil(2);
    let (left, row, rows) = if index < left_count {
        (true, index, left_count)
    } else {
        (false, index - left_count, pin_count - left_count)
    };
    let y = (row as f64 - (rows.saturating_sub(1)) as f64 / 2.0) * 2.54;
    let x = if left { -15.24 } else { 15.24 };
    let stub_x = if left { x - 2.54 } else { x + 2.54 };
    SymbolPinPlacement {
        endpoint: Vec2::new(x, y),
        stub_end: Vec2::new(stub_x, y),
        angle: if left { 0 } else { 180 },
    }
}

fn build_pcb(
    problem: &Problem,
    poses: &BTreeMap<&str, &SemanticKiCadPose>,
    references: &BTreeMap<String, String>,
    terminal_connections: &BTreeMap<(String, String), String>,
    config: &SemanticKiCadTemplateConfig,
    connections: &[String],
) -> Result<super::Expr, String> {
    let mut source = "(kicad_pcb (version 20260206) (generator \"pcb-maker\") (generator_version \"0.1\") (general (thickness 1.6) (legacy_teardrops no)) (paper \"A4\") (layers (0 \"F.Cu\" signal) (2 \"B.Cu\" signal) (9 \"F.Adhes\" user \"F.Adhesive\") (11 \"B.Adhes\" user \"B.Adhesive\") (13 \"F.Paste\" user) (15 \"B.Paste\" user) (5 \"F.SilkS\" user \"F.Silkscreen\") (7 \"B.SilkS\" user \"B.Silkscreen\") (1 \"F.Mask\" user) (3 \"B.Mask\" user) (17 \"Dwgs.User\" user \"User.Drawings\") (19 \"Cmts.User\" user \"User.Comments\") (21 \"Eco1.User\" user \"User.Eco1\") (23 \"Eco2.User\" user \"User.Eco2\") (25 \"Edge.Cuts\" user) (27 \"Margin\" user) (31 \"F.CrtYd\" user \"F.Courtyard\") (29 \"B.CrtYd\" user \"B.Courtyard\") (35 \"F.Fab\" user) (33 \"B.Fab\" user)) (setup (pad_to_mask_clearance 0) (allow_soldermask_bridges_in_footprints yes) (tenting front back)) (net 0 \"\")".to_string();
    for (index, connection) in connections.iter().enumerate() {
        write!(source, "(net {} \"/{}\")", index + 1, escaped(connection))
            .map_err(|error| error.to_string())?;
    }
    for component in &problem.components {
        source.push_str(&footprint(
            component,
            poses[component.id.as_str()],
            &references[&component.id],
            terminal_connections,
            problem.board.bounds,
            config,
        )?);
    }

    let min_x = config.board_origin_mm[0] - config.board_edge_margin_mm;
    let min_y = config.board_origin_mm[1] - config.board_edge_margin_mm;
    let max_x =
        config.board_origin_mm[0] + problem.board.bounds.width() + config.board_edge_margin_mm;
    let max_y =
        config.board_origin_mm[1] + problem.board.bounds.height() + config.board_edge_margin_mm;
    write!(
        source,
        "(gr_rect (start {} {}) (end {} {}) (stroke (width 0.25) (type solid)) (fill none) (layer \"Edge.Cuts\") (uuid \"{}\"))(gr_text \"{} · semantic progressive benchmark\" (at {} {} 0) (layer \"B.Fab\") (uuid \"{}\") (effects (font (size 1.1 1.1) (thickness 0.18)) (justify mirror)))",
        number(min_x),
        number(min_y),
        number(max_x),
        number(max_y),
        deterministic_uuid_for(&format!("pcb-maker:semantic:{}:outline", config.board_id)),
        escaped(&config.board_id),
        number((min_x + max_x) / 2.0),
        number(max_y - 2.0),
        deterministic_uuid_for(&format!("pcb-maker:semantic:{}:silk", config.board_id)),
    )
    .map_err(|error| error.to_string())?;
    let mut connection_widths = BTreeMap::<String, f64>::new();
    for net in &problem.nets {
        let connection = net.electrical_net.as_ref().unwrap_or(&net.id);
        connection_widths
            .entry(connection.clone())
            .and_modify(|width| *width = width.max(net.width))
            .or_insert(net.width);
    }
    for net in &problem.electrical_nets {
        connection_widths
            .entry(net.id.clone())
            .and_modify(|width| *width = width.max(net.width))
            .or_insert(net.width);
    }
    for component in &problem.components {
        let pose = poses[component.id.as_str()];
        if component.body_is_routing_keepout {
            source.push_str(&component_body_keepout_zones(
                component,
                pose,
                terminal_connections,
                &connection_widths,
                problem.rules.clearance,
                problem.board.bounds.min,
                config,
            )?);
        }
        for (index, keepout) in component.routing_keepouts.iter().enumerate() {
            source.push_str(&keepout_zone(
                &format!("{}:keepout:{index}", component.id),
                KeepoutKind::Explicit(&[kicad_layer(&keepout.layer)?]),
                pose,
                keepout.offset,
                &keepout.shape,
                problem.board.bounds.min,
                config,
            )?);
        }
    }
    source.push_str("(embedded_fonts no))");
    parse(&source)
}

/// Keep generated reference silk inside the rectangular board after applying
/// the component pose. KiCad stores the field position in footprint coordinates
/// but its text angle is absolute, so these horizontal labels need an unrotated
/// text envelope. This only solves edge placement; native DRC still checks pads
/// and other silk when admitting the generated board.
fn reference_position(
    component: &Component,
    pose: &SemanticKiCadPose,
    reference: &str,
    bounds: Rect,
    config: &SemanticKiCadTemplateConfig,
) -> Result<Vec2, String> {
    let preferred = Vec2::new(0.0, -component.size.y / 2.0 - 1.2);
    let center = add(
        pose.position,
        rotate_degrees(preferred, pose.rotation_degrees),
    );
    // Conservative envelope for generated ASCII references at 0.8 mm, with
    // room for stroke/font overhang, plus 0.3 mm from the board edge. This is
    // not a general KiCad font metric or a replacement for the native check.
    let half_width = reference.len() as f64 * 0.8 / 2.0 + 0.3;
    let half_height = 0.8;
    let min_x = bounds.min.x - config.board_edge_margin_mm + half_width + 0.3;
    let max_x = bounds.max.x + config.board_edge_margin_mm - half_width - 0.3;
    let min_y = bounds.min.y - config.board_edge_margin_mm + half_height + 0.3;
    let max_y = bounds.max.y + config.board_edge_margin_mm - half_height - 0.3;
    if min_x > max_x || min_y > max_y {
        return Err(format!(
            "board too small for reference silkscreen {reference}"
        ));
    }
    let adjusted = Vec2::new(center.x.clamp(min_x, max_x), center.y.clamp(min_y, max_y));
    Ok(rotate_degrees(
        Vec2::new(adjusted.x - pose.position.x, adjusted.y - pose.position.y),
        -pose.rotation_degrees,
    ))
}

fn footprint(
    component: &Component,
    pose: &SemanticKiCadPose,
    reference: &str,
    terminal_connections: &BTreeMap<(String, String), String>,
    semantic_bounds: Rect,
    config: &SemanticKiCadTemplateConfig,
) -> Result<String, String> {
    let position = board_point(problem_local_to_board(
        pose.position,
        semantic_bounds.min,
        config,
    ));
    let name = footprint_name(component);
    // Tiny passives have nowhere stable to place readable reference silk in a
    // deliberately poor seed placement. Keep their references on fabrication
    // documentation; larger landmarks retain visible journal labels.
    let reference_layer =
        if component.pins.is_empty() || (component.size.x < 3.0 && component.size.y < 3.0) {
            "F.Fab"
        } else {
            "F.SilkS"
        };
    let reference_position = if reference_layer == "F.SilkS" {
        reference_position(component, pose, reference, semantic_bounds, config)?
    } else {
        Vec2::new(0.0, -component.size.y / 2.0 - 1.2)
    };
    let uuid = deterministic_uuid_for(&format!(
        "pcb-maker:semantic:{}:footprint:{}",
        config.board_id, component.id
    ));
    let mut source = format!(
        "(footprint \"Generated:{}\" (layer \"F.Cu\") (uuid \"{}\") (at {} {} {}) (property \"Reference\" \"{}\" (at {} {} 0) (layer \"{}\") (uuid \"{}\") (effects (font (size 0.8 0.8) (thickness 0.12)))) (property \"Value\" \"{}\" (at 0 {} 0) (layer \"F.Fab\") (uuid \"{}\") (effects (font (size 0.8 0.8) (thickness 0.12)))) (property \"Footprint\" \"Generated:{}\" (at 0 0 0) (hide yes) (effects (font (size 1 1)))) (property \"Semantic_Id\" \"{}\" (at 0 0 0) (hide yes) (effects (font (size 1 1))))",
        escaped(&name),
        uuid,
        number(position.x),
        number(position.y),
        number(-pose.rotation_degrees),
        escaped(reference),
        number(reference_position.x),
        number(reference_position.y),
        reference_layer,
        deterministic_uuid_for(&format!("{uuid}:reference")),
        escaped(&component.id),
        number(component.size.y / 2.0 + 1.2),
        deterministic_uuid_for(&format!("{uuid}:value")),
        escaped(&name),
        escaped(&component.id),
    );
    if component.pins.is_empty() {
        source.push_str("(attr board_only exclude_from_pos_files exclude_from_bom)");
    } else {
        source.push_str("(attr smd)");
    }
    if component.constraints.movement != Movement::Fixed {
        source.push_str("(property \"Router_Movable\" \"yes\" (at 0 0 0) (hide yes) (effects (font (size 1 1))))");
        let translate_x = matches!(
            component.constraints.movement,
            Movement::Free | Movement::Horizontal
        );
        let translate_y = matches!(
            component.constraints.movement,
            Movement::Free | Movement::Vertical
        );
        write!(
            source,
            "(property \"Router_Translate_X\" \"{}\" (at 0 0 0) (hide yes) (effects (font (size 1 1))))(property \"Router_Translate_Y\" \"{}\" (at 0 0 0) (hide yes) (effects (font (size 1 1))))(property \"Router_Continuous_Rotation\" \"{}\" (at 0 0 0) (hide yes) (effects (font (size 1 1))))",
            if translate_x { "yes" } else { "no" },
            if translate_y { "yes" } else { "no" },
            if component.constraints.rotation == Rotation::Free { "yes" } else { "no" },
        )
        .map_err(|error| error.to_string())?;
    }
    let half_x = component.size.x / 2.0;
    let half_y = component.size.y / 2.0;
    let courtyard_x = half_x + config.courtyard_clearance_mm;
    let courtyard_y = half_y + config.courtyard_clearance_mm;
    write!(
        source,
        "(fp_rect (start {} {}) (end {} {}) (stroke (width 0.15) (type solid)) (fill none) (layer \"F.Fab\") (uuid \"{}\"))(fp_rect (start {} {}) (end {} {}) (stroke (width 0.05) (type solid)) (fill none) (layer \"F.CrtYd\") (uuid \"{}\"))",
        number(-half_x),
        number(-half_y),
        number(half_x),
        number(half_y),
        deterministic_uuid_for(&format!("{uuid}:fab")),
        number(-courtyard_x),
        number(-courtyard_y),
        number(courtyard_x),
        number(courtyard_y),
        deterministic_uuid_for(&format!("{uuid}:courtyard")),
    )
    .map_err(|error| error.to_string())?;

    if component.pins.is_empty() {
        let diameter = component.size.x.min(component.size.y);
        write!(
            source,
            "(pad \"\" np_thru_hole circle (at 0 0) (size {} {}) (drill {}) (layers \"*.Cu\" \"*.Mask\") (uuid \"{}\"))",
            number(diameter),
            number(diameter),
            number(diameter),
            deterministic_uuid_for(&format!("{uuid}:npth")),
        )
        .map_err(|error| error.to_string())?;
    } else {
        for pin in &component.pins {
            let connection = terminal_connections
                .get(&(component.id.clone(), pin.id.clone()))
                .map(String::as_str);
            if pin.pads.is_empty() {
                write!(
                    source,
                    "(pad \"{}\" smd circle (at {} {}) (size {} {}) (layers \"F.Cu\" \"F.Mask\" \"F.Paste\"){} (uuid \"{}\"))",
                    escaped(&pin.id),
                    number(pin.offset.x),
                    number(pin.offset.y),
                    number(config.implicit_pad_diameter_mm),
                    number(config.implicit_pad_diameter_mm),
                    pad_net(connection),
                    deterministic_uuid_for(&format!("{uuid}:pad:{}:implicit", pin.id)),
                )
                .map_err(|error| error.to_string())?;
                continue;
            }
            if let Some((center, diameter)) = combined_through_hole(pin) {
                let drill = (diameter * config.through_hole_drill_ratio)
                    .min(diameter - 2.0 * config.minimum_annular_ring_mm)
                    .max(0.1);
                write!(
                    source,
                    "(pad \"{}\" thru_hole circle (at {} {}) (size {} {}) (drill {}) (layers \"*.Cu\" \"*.Mask\"){} (uuid \"{}\"))",
                    escaped(&pin.id),
                    number(center.x),
                    number(center.y),
                    number(diameter),
                    number(diameter),
                    number(drill),
                    pad_net(connection),
                    deterministic_uuid_for(&format!("{uuid}:pad:{}:through", pin.id)),
                )
                .map_err(|error| error.to_string())?;
                continue;
            }
            for (pad_index, pad) in pin.pads.iter().enumerate() {
                let center = pin.pad_local_center(pad);
                let layer = kicad_layer(&pad.layer)?;
                let layers = if layer == "F.Cu" {
                    "\"F.Cu\" \"F.Mask\" \"F.Paste\""
                } else {
                    "\"B.Cu\" \"B.Mask\" \"B.Paste\""
                };
                match &pad.shape {
                    CopperShape::Circle { diameter } => write!(
                        source,
                        "(pad \"{}\" smd circle (at {} {}) (size {} {}) (layers {}){} (uuid \"{}\"))",
                        escaped(&pin.id),
                        number(center.x),
                        number(center.y),
                        number(*diameter),
                        number(*diameter),
                        layers,
                        pad_net(connection),
                        deterministic_uuid_for(&format!("{uuid}:pad:{}:{pad_index}", pin.id)),
                    ),
                    CopperShape::Rect {
                        size,
                        rotation_degrees,
                    } => write!(
                        source,
                        "(pad \"{}\" smd rect (at {} {} {}) (size {} {}) (layers {}){} (uuid \"{}\"))",
                        escaped(&pin.id),
                        number(center.x),
                        number(center.y),
                        number(-(pose.rotation_degrees + rotation_degrees)),
                        number(size.x),
                        number(size.y),
                        layers,
                        pad_net(connection),
                        deterministic_uuid_for(&format!("{uuid}:pad:{}:{pad_index}", pin.id)),
                    ),
                }
                .map_err(|error| error.to_string())?;
            }
        }
    }
    source.push(')');
    Ok(source)
}

fn combined_through_hole(pin: &layout_trace_model::model::Pin) -> Option<(Vec2, f64)> {
    if pin.pads.len() != 2 {
        return None;
    }
    let first = &pin.pads[0];
    let second = &pin.pads[1];
    let layers: BTreeSet<_> = [first.layer.as_str(), second.layer.as_str()]
        .into_iter()
        .collect();
    if layers != BTreeSet::from(["bottom", "top"])
        || pin.pad_local_center(first) != pin.pad_local_center(second)
    {
        return None;
    }
    match (&first.shape, &second.shape) {
        (CopperShape::Circle { diameter: first }, CopperShape::Circle { diameter: second })
            if (*first - *second).abs() <= 1.0e-9 =>
        {
            Some((pin.pad_local_center(&pin.pads[0]), *first))
        }
        _ => None,
    }
}

fn pad_net(connection: Option<&str>) -> String {
    connection.map_or_else(String::new, |connection| {
        format!(" (net \"/{}\")", escaped(connection))
    })
}

#[derive(Clone, Copy)]
enum KeepoutKind<'a> {
    ComponentBody(&'a [&'a str]),
    Explicit(&'a [&'a str]),
}

/// Materialize a component body as fixed rule-area cells with explicit pad
/// access ports. A monolithic body polygon makes an edge pad legal as a pad,
/// but leaves no legal route from that pad to the board: the first track cap
/// still intersects the body's track keepout. The semantic engine exempts an
/// attached trace while it exits its own body, so the native adapter must
/// encode the same topology rather than silently deleting the body.
fn component_body_keepout_zones(
    component: &Component,
    pose: &SemanticKiCadPose,
    terminal_connections: &BTreeMap<(String, String), String>,
    connection_widths: &BTreeMap<String, f64>,
    clearance_mm: f64,
    semantic_origin: Vec2,
    config: &SemanticKiCadTemplateConfig,
) -> Result<String, String> {
    let half_x = component.size.x / 2.0;
    let half_y = component.size.y / 2.0;
    let mut ports = [Vec::<[f64; 4]>::new(), Vec::<[f64; 4]>::new()];
    for pin in &component.pins {
        let Some(connection) = terminal_connections.get(&(component.id.clone(), pin.id.clone()))
        else {
            continue;
        };
        let width = connection_widths.get(connection).ok_or_else(|| {
            format!("connection {connection:?} has no width for component-body pad port")
        })?;
        let route_envelope_mm = clearance_mm + width / 2.0;
        let pads = if pin.pads.is_empty() {
            vec![(
                pin.offset,
                Vec2::new(
                    config.implicit_pad_diameter_mm / 2.0,
                    config.implicit_pad_diameter_mm / 2.0,
                ),
                [true, false],
            )]
        } else {
            pin.pads
                .iter()
                .map(|pad| -> Result<_, String> {
                    let half_size = match &pad.shape {
                        CopperShape::Circle { diameter } => {
                            Vec2::new(*diameter / 2.0, *diameter / 2.0)
                        }
                        CopperShape::Rect {
                            size,
                            rotation_degrees,
                        } => {
                            let radians = rotation_degrees.to_radians();
                            Vec2::new(
                                radians.cos().abs() * size.x / 2.0
                                    + radians.sin().abs() * size.y / 2.0,
                                radians.sin().abs() * size.x / 2.0
                                    + radians.cos().abs() * size.y / 2.0,
                            )
                        }
                    };
                    let layers = match kicad_layer(&pad.layer)? {
                        "F.Cu" => [true, false],
                        "B.Cu" => [false, true],
                        layer => return Err(format!("unsupported pad-port layer {layer}")),
                    };
                    Ok((pin.pad_local_center(pad), half_size, layers))
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        for (center, pad_half_size, layers) in pads {
            let access = Vec2::new(
                pad_half_size.x + route_envelope_mm,
                pad_half_size.y + route_envelope_mm,
            );
            let distances = [
                (center.x + half_x).abs(),
                (half_x - center.x).abs(),
                (center.y + half_y).abs(),
                (half_y - center.y).abs(),
            ];
            let side = distances
                .iter()
                .enumerate()
                .min_by(|left, right| left.1.total_cmp(right.1))
                .map(|(index, _)| index)
                .unwrap_or(0);
            let mut port = match side {
                0 => [
                    -half_x,
                    center.y - access.y,
                    center.x + access.x,
                    center.y + access.y,
                ],
                1 => [
                    center.x - access.x,
                    center.y - access.y,
                    half_x,
                    center.y + access.y,
                ],
                2 => [
                    center.x - access.x,
                    -half_y,
                    center.x + access.x,
                    center.y + access.y,
                ],
                _ => [
                    center.x - access.x,
                    center.y - access.y,
                    center.x + access.x,
                    half_y,
                ],
            };
            port[0] = port[0].clamp(-half_x, half_x);
            port[1] = port[1].clamp(-half_y, half_y);
            port[2] = port[2].clamp(-half_x, half_x);
            port[3] = port[3].clamp(-half_y, half_y);
            if port[0] + 1.0e-9 < port[2] && port[1] + 1.0e-9 < port[3] {
                for layer in 0..2 {
                    if layers[layer] {
                        ports[layer].push(port);
                    }
                }
            }
        }
    }
    if ports.iter().all(Vec::is_empty) {
        return keepout_zone(
            &format!("{}:body", component.id),
            KeepoutKind::ComponentBody(&["F.Cu", "B.Cu"]),
            pose,
            Vec2::ZERO,
            &CopperShape::Rect {
                size: component.size,
                rotation_degrees: 0.0,
            },
            semantic_origin,
            config,
        );
    }

    let mut source = String::new();
    for (layer_index, layer_ports) in ports.iter().enumerate() {
        let layer = if layer_index == 0 { "F.Cu" } else { "B.Cu" };
        if layer_ports.is_empty() {
            source.push_str(&keepout_zone(
                &format!("{}:body:{layer}:closed", component.id),
                KeepoutKind::ComponentBody(&[layer]),
                pose,
                Vec2::ZERO,
                &CopperShape::Rect {
                    size: component.size,
                    rotation_degrees: 0.0,
                },
                semantic_origin,
                config,
            )?);
        } else {
            append_component_body_cells(
                &mut source,
                component,
                pose,
                layer,
                layer_ports,
                semantic_origin,
                config,
            )?;
        }
    }
    Ok(source)
}

fn append_component_body_cells(
    source: &mut String,
    component: &Component,
    pose: &SemanticKiCadPose,
    layer: &str,
    ports: &[[f64; 4]],
    semantic_origin: Vec2,
    config: &SemanticKiCadTemplateConfig,
) -> Result<(), String> {
    let half_x = component.size.x / 2.0;
    let half_y = component.size.y / 2.0;
    let mut x_cuts = vec![-half_x, half_x];
    let mut y_cuts = vec![-half_y, half_y];
    for port in ports {
        x_cuts.extend([port[0], port[2]]);
        y_cuts.extend([port[1], port[3]]);
    }
    x_cuts.sort_by(|left, right| left.total_cmp(right));
    y_cuts.sort_by(|left, right| left.total_cmp(right));
    x_cuts.dedup_by(|left, right| (*left - *right).abs() <= 1.0e-9);
    y_cuts.dedup_by(|left, right| (*left - *right).abs() <= 1.0e-9);

    let mut cell_index = 0;
    for y in y_cuts.windows(2) {
        let y_mid = (y[0] + y[1]) / 2.0;
        let mut run_start = None;
        for (x_index, x) in x_cuts.windows(2).enumerate() {
            let x_mid = (x[0] + x[1]) / 2.0;
            let is_port = ports.iter().any(|port| {
                x_mid >= port[0] && x_mid <= port[2] && y_mid >= port[1] && y_mid <= port[3]
            });
            if !is_port && run_start.is_none() {
                run_start = Some(x[0]);
            }
            let end_run = run_start.is_some() && (is_port || x_index + 1 == x_cuts.len() - 1);
            if end_run {
                let x_start = run_start.take().unwrap();
                let x_end = if is_port { x[0] } else { x[1] };
                if x_start + 1.0e-9 < x_end {
                    source.push_str(&keepout_zone(
                        &format!("{}:body:{layer}:port-cell:{cell_index}", component.id),
                        KeepoutKind::ComponentBody(&[layer]),
                        pose,
                        Vec2::new((x_start + x_end) / 2.0, y_mid),
                        &CopperShape::Rect {
                            size: Vec2::new(x_end - x_start, y[1] - y[0]),
                            rotation_degrees: 0.0,
                        },
                        semantic_origin,
                        config,
                    )?);
                    cell_index += 1;
                }
            }
        }
    }
    Ok(())
}

fn keepout_zone(
    identity: &str,
    kind: KeepoutKind<'_>,
    pose: &SemanticKiCadPose,
    offset: Vec2,
    shape: &CopperShape,
    semantic_origin: Vec2,
    config: &SemanticKiCadTemplateConfig,
) -> Result<String, String> {
    let (layers, block_pads): (&[&str], bool) = match kind {
        KeepoutKind::ComponentBody(layers) => (layers, false),
        KeepoutKind::Explicit(layers) => (layers, true),
    };
    // Rule-area points are emitted directly in board coordinates, so they use
    // the semantic rotation. Footprint-local objects use the opposite
    // serialized KiCad angle above; both paths land on the same board points.
    let center = add(pose.position, rotate_degrees(offset, pose.rotation_degrees));
    let (half_size, rotation) = match shape {
        CopperShape::Circle { diameter } => (
            Vec2::new(*diameter / 2.0, *diameter / 2.0),
            pose.rotation_degrees,
        ),
        CopperShape::Rect {
            size,
            rotation_degrees,
        } => (
            Vec2::new(size.x / 2.0, size.y / 2.0),
            pose.rotation_degrees + rotation_degrees,
        ),
    };
    // KiCad rule areas are polygonal. A circular semantic keepout is retained
    // conservatively as its bounding square until native curved rule areas are
    // added; the source shape remains authoritative in the semantic model.
    let corners = [
        Vec2::new(-half_size.x, -half_size.y),
        Vec2::new(half_size.x, -half_size.y),
        Vec2::new(half_size.x, half_size.y),
        Vec2::new(-half_size.x, half_size.y),
    ]
    .map(|corner| {
        board_point(problem_local_to_board(
            add(center, rotate_degrees(corner, rotation)),
            semantic_origin,
            config,
        ))
    });
    let mut points = String::new();
    for corner in corners {
        write!(points, "(xy {} {})", number(corner.x), number(corner.y))
            .map_err(|error| error.to_string())?;
    }
    let layer_form = if layers.len() == 1 {
        format!("(layer \"{}\")", escaped(layers[0]))
    } else {
        format!(
            "(layers {})",
            layers
                .iter()
                .map(|layer| format!("\"{}\"", escaped(layer)))
                .collect::<Vec<_>>()
                .join(" ")
        )
    };
    let pad_restriction = if block_pads {
        " (pads not_allowed)"
    } else {
        ""
    };
    Ok(format!(
        "(zone {layer_form} (uuid \"{}\") (name \"{}\") (hatch edge 0.5) (connect_pads (clearance 0)) (min_thickness 0.25) (keepout (tracks not_allowed) (vias not_allowed){pad_restriction} (copperpour not_allowed)) (fill (thermal_gap 0.3) (thermal_bridge_width 0.3)) (polygon (pts {points})))",
        deterministic_uuid_for(&format!(
            "pcb-maker:semantic:{}:rule-area:{identity}",
            config.board_id
        )),
        escaped(identity),
    ))
}

fn problem_local_to_board(
    point: Vec2,
    semantic_origin: Vec2,
    config: &SemanticKiCadTemplateConfig,
) -> Vec2 {
    Vec2::new(
        point.x - semantic_origin.x + config.board_origin_mm[0],
        point.y - semantic_origin.y + config.board_origin_mm[1],
    )
}

fn board_point(point: Vec2) -> Vec2 {
    point
}

fn kicad_layer(layer: &str) -> Result<&'static str, String> {
    match layer {
        "top" => Ok("F.Cu"),
        "bottom" => Ok("B.Cu"),
        other => Err(format!(
            "semantic KiCad template currently supports top/bottom layers; got {other:?}"
        )),
    }
}

fn symbol_name(component: &Component) -> String {
    format!("Semantic_{}", sanitized(&component.id))
}

fn footprint_name(component: &Component) -> String {
    format!("Semantic_{}", sanitized(&component.id))
}

fn sanitized(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn number(value: f64) -> String {
    let mut formatted = format!("{value:.6}");
    while formatted.contains('.') && formatted.ends_with('0') {
        formatted.pop();
    }
    if formatted.ends_with('.') {
        formatted.pop();
    }
    if formatted == "-0" {
        "0".into()
    } else {
        formatted
    }
}

fn escaped(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn write_encoded(path: &Path, expression: &super::Expr) -> Result<(), String> {
    fs::write(path, format!("{}\n", encode(expression)))
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dual_problem() -> Problem {
        serde_json::from_str(include_str!(
            "../../../benchmarks/imported/layout-trace/dual-esp32-benchmark.json"
        ))
        .unwrap()
    }

    #[test]
    fn dual_problem_preserves_shared_electrical_identity() {
        let problem = dual_problem();
        let (connections, order) = collect_connections(&problem).unwrap();
        assert_eq!(connections.len(), 52);
        assert_eq!(order.len(), 52);
        assert_eq!(connections["VCC"].terminals.len(), 12);
        assert_eq!(connections["GND"].terminals.len(), 16);
        assert_eq!(connections["SIG01_AFTER_LINK"].terminals.len(), 3);
    }

    #[test]
    fn generated_dual_templates_are_parseable_and_complete() {
        let problem = dual_problem();
        let poses = problem
            .components
            .iter()
            .map(|component| SemanticKiCadPose {
                component: component.id.clone(),
                position: component.position,
                rotation_degrees: component.rotation_degrees,
            })
            .collect::<Vec<_>>();
        let references = assign_references(&problem);
        let (connections, order) = collect_connections(&problem).unwrap();
        let terminals = terminal_connection_map(&connections).unwrap();
        let config = SemanticKiCadTemplateConfig {
            board_id: "dual-esp32".into(),
            ..SemanticKiCadTemplateConfig::default()
        };

        let schematic = build_schematic(&problem, &references, &terminals, &config.board_id)
            .expect("build schematic");
        let pose_map = checked_poses(&problem, &poses).unwrap();
        let pcb = build_pcb(
            &problem,
            &pose_map,
            &references,
            &terminals,
            &config,
            &order,
        )
        .expect("build PCB");

        assert_eq!(super::super::count_head(&schematic, "symbol"), 39);
        assert_eq!(super::super::count_head(&pcb, "footprint"), 43);
        assert_eq!(super::super::count_head(&pcb, "segment"), 0);
        assert_eq!(super::super::count_head(&pcb, "via"), 0);
        assert!(encode(&schematic).contains("SIG01_AFTER_LINK"));
        assert!(encode(&pcb).contains("Router_Continuous_Rotation"));
    }

    #[test]
    fn schematic_layout_reflects_library_pin_y_into_sheet_coordinates() {
        let problem = dual_problem();
        let component = problem
            .components
            .iter()
            .find(|component| component.id == "ESP_W")
            .unwrap();
        let (position, pins) = schematic_component_layout(component, 0, 139.7);
        let local_pin_8 = local_symbol_pin_placement(component.pins.len(), 7);
        assert!(local_pin_8.endpoint.y < 0.0);
        assert_eq!(
            pins[7].endpoint,
            Vec2::new(
                position.x + local_pin_8.endpoint.x,
                position.y - local_pin_8.endpoint.y
            )
        );
    }

    #[test]
    fn compact_schematic_components_never_reuse_large_grid_indices() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/imported/layout-trace/forced-crossing-two-layer.json"
        ))
        .unwrap();
        let layouts = schematic_component_layouts(&problem);
        let positions = layouts
            .iter()
            .map(|(position, _)| (position.x.to_bits(), position.y.to_bits()))
            .collect::<BTreeSet<_>>();
        assert_eq!(layouts.len(), 4);
        assert_eq!(positions.len(), layouts.len());
    }

    #[test]
    fn generated_board_caption_is_documentation_not_silkscreen() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/imported/layout-trace/forced-crossing-two-layer.json"
        ))
        .unwrap();
        let poses = problem
            .components
            .iter()
            .map(|component| SemanticKiCadPose {
                component: component.id.clone(),
                position: component.position,
                rotation_degrees: component.rotation_degrees,
            })
            .collect::<Vec<_>>();
        let references = assign_references(&problem);
        let (connections, order) = collect_connections(&problem).unwrap();
        let terminals = terminal_connection_map(&connections).unwrap();
        let config = SemanticKiCadTemplateConfig {
            board_id: "mechanism-forced-crossing".into(),
            ..SemanticKiCadTemplateConfig::default()
        };
        let pose_map = checked_poses(&problem, &poses).unwrap();
        let pcb = build_pcb(
            &problem,
            &pose_map,
            &references,
            &terminals,
            &config,
            &order,
        )
        .unwrap();
        let caption = pcb
            .children()
            .iter()
            .find(|item| item.head() == Some("gr_text"))
            .unwrap();
        assert_eq!(
            caption
                .child("layer")
                .and_then(|layer| layer.children().get(1))
                .and_then(super::super::Expr::atom),
            Some("B.Fab")
        );
    }

    #[test]
    fn body_keepouts_allow_pads_but_explicit_keepouts_block_them() {
        let pose = SemanticKiCadPose {
            component: "BLOCK".into(),
            position: Vec2::new(10.0, 10.0),
            rotation_degrees: 0.0,
        };
        let shape = CopperShape::Rect {
            size: Vec2::new(4.0, 4.0),
            rotation_degrees: 0.0,
        };
        let config = SemanticKiCadTemplateConfig::default();
        let body = keepout_zone(
            "body",
            KeepoutKind::ComponentBody(&["F.Cu", "B.Cu"]),
            &pose,
            Vec2::ZERO,
            &shape,
            Vec2::ZERO,
            &config,
        )
        .unwrap();
        let explicit = keepout_zone(
            "explicit",
            KeepoutKind::Explicit(&["F.Cu"]),
            &pose,
            Vec2::ZERO,
            &shape,
            Vec2::ZERO,
            &config,
        )
        .unwrap();
        assert!(!body.contains("pads not_allowed"));
        assert!(explicit.contains("pads not_allowed"));
        assert!(body.contains("tracks not_allowed"));
        assert!(body.contains("vias not_allowed"));
    }

    #[test]
    fn rotated_rule_areas_follow_kicad_footprint_angle_direction() {
        let pose = SemanticKiCadPose {
            component: "BLOCK".into(),
            position: Vec2::new(10.0, 10.0),
            rotation_degrees: 90.0,
        };
        let source = keepout_zone(
            "offset-body",
            KeepoutKind::ComponentBody(&["F.Cu", "B.Cu"]),
            &pose,
            Vec2::new(2.0, 0.0),
            &CopperShape::Rect {
                size: Vec2::new(2.0, 4.0),
                rotation_degrees: 0.0,
            },
            Vec2::ZERO,
            &SemanticKiCadTemplateConfig {
                board_origin_mm: [0.0, 0.0],
                ..SemanticKiCadTemplateConfig::default()
            },
        )
        .unwrap();
        assert!(source.contains("(xy 12 11)"));
        assert!(source.contains("(xy 12 13)"));
        assert!(source.contains("(xy 8 13)"));
        assert!(source.contains("(xy 8 11)"));
    }

    #[test]
    fn rotated_footprint_pads_land_on_semantic_route_endpoints() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/imported/layout-trace/esp32-slices/esp-pad-fanout-clearance.json"
        ))
        .unwrap();
        let poses = problem
            .components
            .iter()
            .map(|component| SemanticKiCadPose {
                component: component.id.clone(),
                position: component.position,
                rotation_degrees: component.rotation_degrees,
            })
            .collect::<Vec<_>>();
        let references = assign_references(&problem);
        let (connections, order) = collect_connections(&problem).unwrap();
        let terminals = terminal_connection_map(&connections).unwrap();
        let config = SemanticKiCadTemplateConfig {
            board_id: "esp-pad-orientation".into(),
            board_origin_mm: [10.0, 10.0],
            ..SemanticKiCadTemplateConfig::default()
        };
        let pose_map = checked_poses(&problem, &poses).unwrap();
        let pcb = build_pcb(
            &problem,
            &pose_map,
            &references,
            &terminals,
            &config,
            &order,
        )
        .unwrap();
        let model = super::super::KiCadRoutingModel::from_pcb(&pcb, "L3").unwrap();
        let expected = problem.nets[0].seed_route[0];
        let expected = [
            expected.x - problem.board.bounds.min.x + config.board_origin_mm[0],
            expected.y - problem.board.bounds.min.y + config.board_origin_mm[1],
        ];
        assert!(
            model
                .terminals
                .iter()
                .any(|terminal| super::super::distance_squared(*terminal, expected) <= 1.0e-8)
        );
    }

    #[test]
    fn legacy_point_terminals_materialize_as_configured_pads() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/imported/layout-trace/decoupling-capacitor.json"
        ))
        .unwrap();
        let poses = problem
            .components
            .iter()
            .map(|component| SemanticKiCadPose {
                component: component.id.clone(),
                position: component.position,
                rotation_degrees: component.rotation_degrees,
            })
            .collect::<Vec<_>>();
        let references = assign_references(&problem);
        let (connections, order) = collect_connections(&problem).unwrap();
        let terminals = terminal_connection_map(&connections).unwrap();
        let config = SemanticKiCadTemplateConfig {
            board_id: "mechanism-decoupling".into(),
            implicit_pad_diameter_mm: 1.7,
            ..SemanticKiCadTemplateConfig::default()
        };
        let pose_map = checked_poses(&problem, &poses).unwrap();
        let pcb = build_pcb(
            &problem,
            &pose_map,
            &references,
            &terminals,
            &config,
            &order,
        )
        .unwrap();
        let encoded = encode(&pcb);
        assert!(encoded.contains("(pad \"VCC\" smd circle"));
        assert!(encoded.contains("(size 1.7 1.7)"));
        assert!(encoded.contains("(net \"/VCC_DECOUPLE\")"));
        assert!(encoded.contains("U1:body:F.Cu:port-cell:"));
        assert!(encoded.contains("C1:body:F.Cu:port-cell:"));
        assert!(encoded.contains("U1:body:B.Cu:closed"));
        assert!(encoded.contains("C1:body:B.Cu:closed"));
        assert!(encoded.contains("X_CENTER:body"));
    }
}
