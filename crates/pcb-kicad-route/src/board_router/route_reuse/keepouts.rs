// Copyright (C) 2026 Florian Loitsch.

//! Screen prior copper against explicit current track and via keepouts.

use super::*;

pub(super) struct CopperKeepouts(Vec<pcb_router::Obstacle>);

impl CopperKeepouts {
    pub(super) fn of(board: &Expr, layers: &LayerTable) -> Self {
        let items = board.children();
        let footprints = items
            .iter()
            .filter(|item| matches!(item.head(), Some("footprint" | "module")));
        let areas = items
            .iter()
            .chain(footprints.flat_map(Expr::children))
            .filter(|item| is_rule_area(item) && is_straight_polygon(item))
            .filter_map(|zone| {
                rule_area_obstacle(zone, layers, "unnamed rule area")
                    .ok()
                    .flatten()
            })
            .filter(|obstacle| obstacle.layers != 0)
            .collect();
        Self(areas)
    }

    pub(super) fn blocks_any(&self, items: &[&Expr], layers: &LayerTable) -> bool {
        !self.0.is_empty()
            && items
                .iter()
                .any(|item| self.blocks(item, layers).unwrap_or(false))
    }

    /// Inexact boundary contacts stay for native DRC. Only tracks whose
    /// occupied width reaches into the area are withheld.
    fn blocks(&self, item: &Expr, layers: &LayerTable) -> Option<bool> {
        const NANOMETRE: f64 = 0.000001;
        if item.head() == Some("via") {
            let center = form_xy(item, "at").ok()?;
            let radius = form_f64(item, "size", 1).ok()? / 2.0 - NANOMETRE;
            // The reuse geometry gate admits only vias from F.Cu to B.Cu.
            return Some(
                self.0
                    .iter()
                    .any(|area| area.blocks_vias && area.shape.distance_to_point(center) < radius),
            );
        }
        let layer = layers.index(form_atom(item, "layer", 1)?)?;
        let start = form_xy(item, "start").ok()?;
        let end = form_xy(item, "end").ok()?;
        let radius = form_f64(item, "width", 1).ok()? / 2.0 - NANOMETRE;
        let crosses = |start, end, radius| {
            self.0.iter().any(|area| {
                area.blocks_tracks
                    && area.layers & (1 << layer) != 0
                    && area.shape.distance_to_segment(start, end) < radius
            })
        };
        if item.head() == Some("arc") {
            let mid = form_xy(item, "mid").ok()?;
            // The three written points are on the arc. Chords can cross
            // areas the actual stroke misses, especially on large arcs.
            // KiCad handles intersections between these exact samples.
            Some(
                [start, mid, end]
                    .into_iter()
                    .any(|at| crosses(at, at, radius)),
            )
        } else {
            Some(crosses(start, end, radius))
        }
    }
}

/// Keepouts with holes or curved boundaries stay with the native gate.
/// Their lowered polygon can contain a synthetic slit or flattened edge.
fn is_straight_polygon(area: &Expr) -> bool {
    let mut polygons = area
        .children()
        .iter()
        .filter(|item| item.head() == Some("polygon"));
    let Some(points) = polygons.next().and_then(|polygon| polygon.child("pts")) else {
        return false;
    };
    polygons.next().is_none()
        && points
            .children()
            .iter()
            .skip(1)
            .all(|point| point.head() == Some("xy"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obstacles(layer: &str, policies: &str) -> (LayerTable, CopperKeepouts) {
        let board = parse(&format!(
            r#"(kicad_pcb
            (layers (0 "F.Cu" signal) (2 "In1.Cu" signal) (31 "B.Cu" signal))
            (zone (layers {layer}) (keepout {policies})
              (polygon (pts (xy 1 1) (xy 2 1) (xy 2 2) (xy 1 2)))))"#
        ))
        .unwrap();
        let layers = LayerTable::from_pcb(&board).unwrap();
        let keepouts = CopperKeepouts::of(&board, &layers);
        (layers, keepouts)
    }

    #[test]
    fn a_keepout_checks_the_tracks_width_and_layers() {
        let (layers, area) = obstacles(r#""F.Cu" "In1.Cu""#, "(tracks not_allowed) (vias allowed)");
        let segment = r#"(segment (start 0 0.95) (end 3 0.95) (width 0.2) (layer "F.Cu"))"#;
        let hits = |item: &str| area.blocks_any(&[&parse(item).unwrap()], &layers);
        assert!(hits(segment));
        assert!(hits(&segment.replace("F.Cu", "In1.Cu")));
        assert!(!hits(&segment.replace("F.Cu", "B.Cu")));
        assert!(!hits(&segment.replace("0.95", "0.89")));
        assert!(!hits(
            r#"(via (at 1.5 1.5) (size 0.6) (layers "F.Cu" "B.Cu"))"#
        ));
        assert!(hits(
            r#"(arc (start 0.5 0) (mid 1.5 1) (end 2.5 0) (width 0.2) (layer "F.Cu"))"#
        ));
        assert!(!hits(
            r#"(arc (start 0 1.5) (mid 1.5 3) (end 3 1.5) (width 0.2) (layer "F.Cu"))"#
        ));
    }

    #[test]
    fn an_inner_layer_via_keepout_reaches_through_copper_only_when_asked() {
        let via = parse(r#"(via (at 1.5 0.8) (size 0.6) (layers "F.Cu" "B.Cu"))"#).unwrap();
        let track =
            parse(r#"(segment (start 0 1.5) (end 3 1.5) (width 0.2) (layer "In1.Cu"))"#).unwrap();
        let (layers, area) = obstacles(r#""In1.Cu""#, "(vias not_allowed)");
        assert!(area.blocks_any(&[&via], &layers));
        assert!(!area.blocks_any(&[&track], &layers));
        let (layers, area) = obstacles(
            r#""In1.Cu""#,
            "(copperpour not_allowed) (footprints not_allowed)",
        );
        assert!(!area.blocks_any(&[&via, &track], &layers));
    }
}
