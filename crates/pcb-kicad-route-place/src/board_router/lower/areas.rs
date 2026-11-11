// Copyright (C) 2026 Toit contributors.

//! Rule areas (the board's and its footprints'), board cutouts and the
//! regions where a pour stays whole.

use super::*;
use pcb_router as core;

impl Builder {
    /// The rule areas inside a footprint (an antenna keepout): they keep
    /// tracks and vias out like board-level ones, and fills too.
    pub(super) fn footprint_rule_areas(
        &mut self,
        ctx: &Context,
        item: &Expr,
        reference: &str,
    ) -> Result<(), String> {
        for zone in item
            .children()
            .iter()
            .filter(|child| child.head() == Some("zone") && is_rule_area(child))
        {
            if let Some(obstacle) =
                rule_area_obstacle(zone, &ctx.layers, &format!("{reference} keepout"))?
            {
                self.obstacles.push(obstacle);
            }
            if let Some(keepout) = pour_keepout(zone, &ctx.layers)? {
                self.pour_keepouts.push(keepout);
            }
        }
        Ok(())
    }

    /// A board-level rule area: its keepout obstacle (one the fill passes
    /// where it allows pours), its keep-whole region and its fill keepout.
    pub(super) fn board_rule_area(&mut self, ctx: &Context, item: &Expr) -> Result<(), String> {
        if let Some(obstacle) = rule_area_obstacle(item, &ctx.layers, "unnamed rule area")? {
            // Tracks out, pours in (a plane layer's `no tracks` area):
            // KiCad fills it, so the pour model does too.
            if obstacle.blocks_tracks && rule_area_allows_pours(item) {
                self.fill_passes.push(self.obstacles.len());
            }
            self.obstacles.push(obstacle);
        }
        if let Some(region) = keep_whole_region(item, &ctx.layers)? {
            self.keep_whole.push(region);
        }
        if let Some(keepout) = pour_keepout(item, &ctx.layers)? {
            self.pour_keepouts.push(keepout);
        }
        Ok(())
    }

    /// The outline's cutouts as holes on every layer.
    pub(super) fn board_cutouts(&mut self, ctx: &Context) {
        for cutout in &ctx.loops.cutouts {
            // A cutout is board edge to KiCad: the edge clearance applies,
            // not the hole clearance.
            self.obstacles.push(core::Obstacle {
                shape: core::Shape::Polygon {
                    points: cutout.clone(),
                },
                layers: ctx.layers.all(),
                kind: core::ObstacleKind::Hole,
                net: None,
                clearance: ctx.config.edge_clearance_mm + outline::ARC_TOLERANCE,
                clearance_override: None,
                plated: false,
                blocks_tracks: true,
                blocks_vias: true,
                label: "board cutout".into(),
            });
        }
    }

    /// The queued keep-whole regions, as keepouts of their pour's net. A
    /// region where a pour stays whole keeps other nets' tracks off its
    /// layers and their vias out; the pour's own net may pass (and its
    /// fill fills it).
    pub(super) fn keep_whole_keepouts(&mut self) {
        for (net, mask, points, label) in std::mem::take(&mut self.keep_whole) {
            let Some(id) = net_ids_lookup(&self.nets, &net) else {
                continue;
            };
            self.obstacles.push(core::Obstacle {
                shape: core::Shape::Polygon { points },
                layers: mask,
                kind: core::ObstacleKind::Keepout,
                net: Some(id),
                clearance: 0.0,
                clearance_override: None,
                plated: false,
                blocks_tracks: true,
                blocks_vias: true,
                label,
            });
        }
    }
}
