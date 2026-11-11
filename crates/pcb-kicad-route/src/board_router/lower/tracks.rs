// Copyright (C) 2026 Toit contributors.

//! The board's own tracks, arcs and vias: fixed copper of their nets.

use super::*;
use pcb_router as core;

impl Builder {
    /// A track segment on a copper layer: a capsule of its net.
    pub(super) fn existing_segment(&mut self, ctx: &Context, item: &Expr) -> Result<(), String> {
        let Some(layer) = form_atom(item, "layer", 1).and_then(|name| ctx.layers.index(name))
        else {
            return Ok(());
        };
        let net = self.item_net_id(ctx, item)?;
        self.obstacles.push(core::Obstacle {
            shape: core::Shape::Capsule {
                start: form_xy(item, "start")?,
                end: form_xy(item, "end")?,
                radius: form_f64(item, "width", 1)? / 2.0,
            },
            layers: 1 << layer,
            kind: core::ObstacleKind::Copper,
            net,
            clearance: 0.0,
            clearance_override: None,
            plated: false,
            blocks_tracks: true,
            blocks_vias: true,
            label: "existing track".into(),
        });
        Ok(())
    }

    /// A via: a disc of its net on its layers (`via_layers`).
    pub(super) fn existing_via(&mut self, ctx: &Context, item: &Expr) -> Result<(), String> {
        let net = self.item_net_id(ctx, item)?;
        self.obstacles.push(core::Obstacle {
            shape: core::Shape::Circle {
                center: form_xy(item, "at")?,
                radius: form_f64(item, "size", 1)? / 2.0,
            },
            layers: via_layers(item, &ctx.layers),
            kind: core::ObstacleKind::Copper,
            net,
            clearance: 0.0,
            clearance_override: None,
            plated: false,
            blocks_tracks: true,
            blocks_vias: true,
            label: "existing via".into(),
        });
        Ok(())
    }

    /// An arc track: its chords, grown by the flattening's tolerance so
    /// they hold the true arc (as copper graphics' arcs are), fixed copper
    /// of its net like a segment.
    pub(super) fn existing_arc(&mut self, ctx: &Context, item: &Expr) -> Result<(), String> {
        let Some(layer) = form_atom(item, "layer", 1).and_then(|name| ctx.layers.index(name))
        else {
            return Ok(());
        };
        let net = self.item_net_id(ctx, item)?;
        let radius = form_f64(item, "width", 1)? / 2.0 + outline::ARC_TOLERANCE;
        let points = outline::arc_points(
            form_xy(item, "start")?,
            form_xy(item, "mid")?,
            form_xy(item, "end")?,
        );
        for pair in points.windows(2) {
            self.obstacles.push(core::Obstacle {
                shape: core::Shape::Capsule {
                    start: pair[0],
                    end: pair[1],
                    radius,
                },
                layers: 1 << layer,
                kind: core::ObstacleKind::Copper,
                net,
                clearance: 0.0,
                clearance_override: None,
                plated: false,
                blocks_tracks: true,
                blocks_vias: true,
                label: "existing track".into(),
            });
        }
        Ok(())
    }
}
