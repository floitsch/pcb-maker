// Copyright (C) 2026 Toit contributors.

//! Copper written into a board: the routed segments, vias and branches, and
//! the cold board without any copper.

use super::*;

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

pub(crate) fn supplemental_via_expr(via: &SupplementalVia) -> Expr {
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

pub(crate) fn supplemental_segment_expr(segment: &SupplementalSegment) -> Expr {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
