// Copyright (C) 2026 Toit contributors.

//! Prepared fan-outs (`bga_fanout`): the dog-bones of the grid arrays'
//! inner balls, fixed copper of their nets, each via the ball's terminal on
//! the layers off the ball's.

use super::*;
use pcb_router as core;

impl Builder {
    /// The planned fan-outs' copper, holes and terminals, with the plan's
    /// work (0 without a plan).
    pub(super) fn place_fanouts(
        &mut self,
        ctx: &Context,
        via_spans: &[core::ViaSpan],
        planes: &[core::Plane],
    ) -> u64 {
        if !ctx.fanout_on || self.fanout_parts.is_empty() {
            return 0;
        }
        self.balls_holding_vias(ctx);
        let plan = bga_fanout::plan(
            &self.fanout_parts,
            &self.obstacles,
            &self.nets,
            &self.classes,
            &self.no_via_pads,
            &self.holding_vias,
            (via_spans, planes),
            ctx.config,
        );
        bga_fanout::report(&plan);
        let all = ctx.layers.all();
        for fanout in plan.fanouts {
            self.place_fanout(fanout, all);
        }
        plan.work
    }

    /// A ball with a via of the board's own in its copper keeps it: it is
    /// added to `holding_vias`.
    fn balls_holding_vias(&mut self, ctx: &Context) {
        for item in ctx
            .pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("via"))
        {
            if let Ok(at) = form_at(item) {
                for part in &self.fanout_parts {
                    for &(index, _) in &part.pads {
                        if self.obstacles[index]
                            .shape
                            .distance_to_point([at[0], at[1]])
                            <= 0.0
                        {
                            self.holding_vias.insert(index);
                        }
                    }
                }
            }
        }
    }

    /// One dog-bone: its stub and via ring on the ball's layer, the via on
    /// the others (`all`, the board's layers) as the ball's terminal there,
    /// and its hole.
    fn place_fanout(&mut self, fanout: bga_fanout::PlannedFanout, all: core::LayerMask) {
        let net = Some(fanout.net);
        let ball = 1 << fanout.layer;
        // A via of a span is copper and a hole on its layers only.
        let all = fanout.span.map_or(all, |span| span.mask());
        let copper = |shape: core::Shape, layers: core::LayerMask, suffix: &str| core::Obstacle {
            shape,
            layers,
            kind: core::ObstacleKind::Copper,
            net,
            clearance: 0.0,
            clearance_override: None,
            plated: false,
            blocks_tracks: true,
            blocks_vias: true,
            label: format!("{} {suffix}", fanout.label),
        };
        let ring = core::Shape::Circle {
            center: fanout.via,
            radius: fanout.diameter / 2.0,
        };
        self.obstacles.push(copper(
            core::Shape::Capsule {
                start: fanout.start,
                end: fanout.via,
                radius: fanout.width / 2.0,
            },
            ball,
            "fan-out stub",
        ));
        self.obstacles
            .push(copper(ring.clone(), ball, "fan-out via ring"));
        let other = self.obstacles.len();
        if all & !ball != 0 {
            self.obstacles
                .push(copper(ring, all & !ball, "fan-out via"));
        }
        self.obstacles.push(core::Obstacle {
            shape: core::Shape::Circle {
                center: fanout.via,
                radius: fanout.drill / 2.0,
            },
            layers: all,
            kind: core::ObstacleKind::Hole,
            net,
            clearance: 0.0,
            clearance_override: None,
            plated: false,
            blocks_tracks: true,
            blocks_vias: true,
            label: format!("{} fan-out hole", fanout.label),
        });
        if all & !ball == 0 {
            return;
        }
        // The via is the ball's terminal on the other layers (the terminal
        // the ball was merged into, if it was), solid to a pour there.
        self.solid_pad_obstacles.push(other);
        if let Some((host, anchor)) = self.terminal_host(fanout.net, fanout.pad) {
            self.nets[fanout.net as usize].terminals[host].layers |= all;
            self.merged_pads.push(core::MergedPad {
                net: fanout.net,
                host,
                pad: other,
                anchor,
                layers: all & !ball,
                padstack: true,
            });
        }
    }
}
