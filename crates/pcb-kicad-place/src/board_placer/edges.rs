// Copyright (C) 2026 Toit contributors.

//! Parts on `"edge": "any"` settle to their nearest edge.

use super::*;
use pcb_placer as core;

/// Places the movable footprints of `<source>/<board_id>.kicad_pcb`, removes
/// all routed copper, and writes the project plus an HTML playback to
/// `output`.
/// Parts on `"edge": "any"` settle first without that constraint (in a
/// placement thrown away); each then keeps to the board edge nearest to
/// where it settled, the one along its long side on a near tie. Rewrites
/// the constraints and returns the choices.
pub(super) fn choose_any_edges(
    source_directory: &Path,
    board_id: &str,
    config: &mut KiCadBoardPlacerConfig,
) -> Result<BTreeMap<String, String>, String> {
    let Some(KiCadConstraintsSource::Inline(constraints)) = &config.constraints else {
        return Ok(BTreeMap::new());
    };
    if !constraints.edge.iter().any(|entry| entry.edge == "any") {
        return Ok(BTreeMap::new());
    }
    let mut free = config.clone();
    if let Some(KiCadConstraintsSource::Inline(constraints)) = &mut free.constraints {
        constraints.edge.retain(|entry| entry.edge != "any");
    }
    let directory = std::env::temp_dir().join(format!(
        "pcb-maker-any-edge-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0)
    ));
    let polygonal = constraints
        .outline
        .as_ref()
        .is_some_and(|outline| outline.polygonal());
    let trial = place_kicad_board(source_directory, board_id, &directory, &free);
    let loops = fs::read_to_string(directory.join(format!("{board_id}.kicad_pcb")))
        .ok()
        .and_then(|text| crate::canonical_order::parse_board(&text).ok())
        .and_then(|pcb| outline::board_loops(&pcb).ok());
    // On a polygon, the distance to the nearest side facing each way.
    let sides = match (&loops, polygonal) {
        (Some(loops), true) => core::constraints::outline_sides(&loops.piece_loops),
        _ => Vec::new(),
    };
    let bounds = loops.map(|loops| {
        loops.outline.iter().fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |b, p| {
                [
                    b[0].min(p[0]),
                    b[1].min(p[1]),
                    b[2].max(p[0]),
                    b[3].max(p[1]),
                ]
            },
        )
    });
    let _ = fs::remove_dir_all(&directory);
    let trial = trial?;
    let bounds =
        bounds.ok_or("\"edge\": \"any\" needs a board outline (or an `outline` constraint)")?;
    let mut chosen = BTreeMap::new();
    if let Some(KiCadConstraintsSource::Inline(constraints)) = &mut config.constraints {
        for entry in constraints
            .edge
            .iter_mut()
            .filter(|entry| entry.edge == "any")
        {
            let placed = trial
                .footprints
                .iter()
                .find(|footprint| footprint.reference == entry.part)
                .ok_or_else(|| format!("constraint names unknown part {:?}", entry.part))?;
            let ([x, y], [half_x, half_y]) = (placed.body_center, placed.body_half);
            let along_x = half_x >= half_y;
            let box_distances = [
                x - half_x - bounds[0],
                y - half_y - bounds[1],
                bounds[2] - x - half_x,
                bounds[3] - y - half_y,
            ];
            let distance = |slot: usize, edge: core::constraints::Edge| {
                if sides.is_empty() {
                    box_distances[slot]
                } else {
                    core::constraints::side_distance(&sides, edge, [x, y], [half_x, half_y])
                        .unwrap_or(f64::INFINITY)
                }
            };
            use core::constraints::Edge;
            let edges = [
                ("left", distance(0, Edge::Left), !along_x),
                ("top", distance(1, Edge::Top), along_x),
                ("right", distance(2, Edge::Right), !along_x),
                ("bottom", distance(3, Edge::Bottom), along_x),
            ];
            let cost = |(_, distance, along): &(&str, f64, bool)| {
                distance - if *along { 1.0 } else { 0.0 }
            };
            let (name, _, _) = edges
                .iter()
                .min_by(|a, b| cost(a).total_cmp(&cost(b)))
                .expect("four edges");
            entry.edge = name.to_string();
            chosen.insert(entry.part.clone(), name.to_string());
        }
    }
    Ok(chosen)
}
