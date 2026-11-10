// Copyright (C) 2026 Toit contributors.

//! The board's graphics, dimensions and text boxes: copper (queued until
//! the pads' nets are known, then placed), mask openings (mask.rs) and
//! Margin keepouts.

use super::*;
use pcb_router as core;

/// A board graphic on copper, placed once the nets of the pads are known.
pub(super) struct CopperGraphic {
    pub shapes: Vec<core::Shape>,
    pub layers: core::LayerMask,
    /// Its routed net, by name.
    pub net: Option<String>,
    pub kind: String,
    /// The clearance of its class (its net's, else Default's), kept where
    /// it has no routed net.
    pub clearance: f64,
}

/// A graphic on the Margin layer as KiCad 10.0.7's copper_edge_clearance
/// sees it: board edge, with its stroke (only Edge.Cuts strokes count as
/// lines of no width) and its fill, which copper on every layer keeps the
/// edge clearance from (the zone filler knocks it out too).
/// `copper_graphic_shapes` already holds an arc's chord error.
pub(super) fn margin_obstacles(
    shapes: Vec<core::Shape>,
    layers: &LayerTable,
    config: &KiCadBoardRouterConfig,
    label: &str,
) -> impl Iterator<Item = core::Obstacle> {
    let all = layers.all();
    let clearance = config.edge_clearance_mm;
    let label = label.to_string();
    shapes.into_iter().map(move |shape| core::Obstacle {
        shape,
        layers: all,
        kind: core::ObstacleKind::Keepout,
        net: None,
        clearance,
        clearance_override: None,
        plated: false,
        blocks_tracks: true,
        blocks_vias: true,
        label: label.clone(),
    })
}

impl Builder {
    /// A dimension or a text box: queued copper of no net on the copper
    /// layers it names, a queued opening on the mask layers. A dimension
    /// or a text box on a copper layer is copper of no net to KiCad
    /// (shrimp-18's "Dimension '10' on F.Cu": three shorting_items); a
    /// text box on a mask layer opens it.
    pub(super) fn annotation(&mut self, ctx: &Context, item: &Expr) -> Result<(), String> {
        let copper = ctx.layers.mask_of_item(item)?;
        let text_box = item.head() == Some("gr_text_box");
        let sides = if text_box {
            mask_sides(&item_layer_names(item), ctx.layers.len())
        } else {
            Vec::new()
        };
        if copper == 0 && sides.is_empty() {
            return Ok(());
        }
        let shapes = if text_box {
            text_box_shapes(item)?
        } else {
            dimension_shapes(item)?
        };
        if shapes.is_empty() {
            return Ok(());
        }
        let kind = if text_box { "text box" } else { "dimension" };
        for side in sides {
            self.mask_openings.push(MaskOpening {
                shapes: shapes.clone(),
                layer: side,
                kind: kind.to_string(),
                margin: 0.0,
                expansion: 0.0,
            });
        }
        if copper != 0 {
            self.copper_graphics.push(CopperGraphic {
                shapes,
                layers: copper,
                net: None,
                kind: kind.to_string(),
                clearance: ctx.no_net_clearance,
            });
        }
        Ok(())
    }

    /// A board graphic or text: a Margin keepout, queued copper of its net
    /// on the copper layers it names, a queued opening on the mask layers.
    pub(super) fn board_graphic(&mut self, ctx: &Context, item: &Expr) -> Result<(), String> {
        let copper = ctx.layers.mask_of_item(item)?;
        let names = item_layer_names(item);
        let sides = mask_sides(&names, ctx.layers.len());
        // KiCad keeps the edge clearance from graphics on Margin as from
        // the board edge (copper_edge_clearance: "Segment on Margin",
        // michpro's PY32F002 board).
        if names.contains(&"Margin") && item.head() != Some("gr_text") {
            self.obstacles.extend(margin_obstacles(
                copper_graphic_shapes(item)?,
                &ctx.layers,
                ctx.config,
                &format!("board margin ({})", item.head().unwrap_or("graphic")),
            ));
        }
        if copper == 0 && sides.is_empty() {
            return Ok(());
        }
        let shapes = copper_graphic_shapes(item)?;
        let kind = item.head().unwrap_or("graphic").to_string();
        if copper != 0 {
            let net = item_net(item, &ctx.net_codes)
                .filter(|raw| ctx.net_names.routable(raw))
                .map(|raw| ctx.net_names.normalize(raw).to_string());
            let clearance = item_net(item, &ctx.net_codes)
                .map(|raw| ctx.net_names.normalize(raw))
                .and_then(|name| ctx.config.connection_rules.get(name))
                .map_or(ctx.no_net_clearance, |rules| rules.clearance_mm);
            let mut copper_shapes = shapes.clone();
            if item.head() == Some("gr_text") {
                copper_shapes = drawn_graphic_shapes(item)?;
                copper_shapes.extend(cached_text_line_boxes(item)?);
            }
            self.copper_graphics.push(CopperGraphic {
                shapes: copper_shapes,
                layers: copper,
                net,
                kind: kind.clone(),
                clearance,
            });
        }
        let shapes = if ctx.mask.strict && !sides.is_empty() {
            mask_graphic_shapes(item)?
        } else {
            shapes
        };
        // KiCad's bridge test grows a shape by its mask expansion, a text
        // by nothing.
        let expansion = if item.head() == Some("gr_text") {
            0.0
        } else {
            form_f64(item, "solder_mask_margin", 1)
                .ok()
                .unwrap_or(ctx.mask.board_pad_to_mask)
                .max(0.0)
        };
        for side in sides {
            self.mask_openings.push(MaskOpening {
                shapes: shapes.clone(),
                layer: side,
                kind: kind.clone(),
                margin: 0.0,
                expansion,
            });
        }
        Ok(())
    }

    /// The queued copper graphics, as copper of their net or, without one,
    /// of none (other nets keep clear), and a filled graphic or a stroke of
    /// a net as a terminal of it too.
    pub(super) fn place_copper_graphics(&mut self) {
        let obstacles = &mut self.obstacles;
        let nets = &mut self.nets;
        for (index, graphic) in std::mem::take(&mut self.copper_graphics)
            .into_iter()
            .enumerate()
        {
            let CopperGraphic {
                shapes,
                layers: copper,
                net,
                kind,
                clearance,
            } = graphic;
            let net = net.and_then(|name| net_ids_lookup(nets, &name));
            // A filled graphic with a net is copper KiCad wants connected
            // like a pad (Sisu's antenna feed, a rectangle on /RF/ANT,
            // stayed an island): it is a terminal of its net too.
            // Its fill is the one shape that is not a stroke's capsule.
            let fills: Vec<usize> = (0..shapes.len())
                .filter(|index| !matches!(shapes[*index], core::Shape::Capsule { .. }))
                .collect();
            if let (Some(id), [fill]) = (net, fills.as_slice()) {
                let shape = &shapes[*fill];
                let bounds = shape.aabb();
                let center = [
                    (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
                    (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
                ];
                if shape.contains(center) {
                    nets[id as usize].terminals.push(core::Terminal {
                        anchor: center,
                        layers: copper,
                        pad: obstacles.len() + fill,
                        contact: None,
                        label: format!("{kind} {}", index + 1),
                    });
                }
            } else if let (Some(id), [], Some(core::Shape::Capsule { start, end, .. })) =
                (net, fills.as_slice(), shapes.first())
            {
                // A stroke with a net (a line, an arc, a circle's ring) is
                // an item of KiCad's connectivity like a pad, which wants it
                // joined to the net (BleJoystick's arcs, the chessboards'
                // rings, all `(net N)` by code): a terminal at the middle of
                // its first piece, unless it lies on copper of a terminal of
                // its net already (a line drawn from a pad: one item with
                // it).
                let touches = nets[id as usize].terminals.iter().any(|terminal| {
                    let other = &obstacles[terminal.pad];
                    other.layers & copper != 0
                        && shapes.iter().any(|shape| match shape {
                            core::Shape::Capsule { start, end, radius } => {
                                other.shape.distance_to_segment(*start, *end) <= *radius
                            }
                            _ => false,
                        })
                });
                if !touches {
                    nets[id as usize].terminals.push(core::Terminal {
                        anchor: [(start[0] + end[0]) / 2.0, (start[1] + end[1]) / 2.0],
                        layers: copper,
                        pad: obstacles.len(),
                        contact: None,
                        label: format!("{kind} {}", index + 1),
                    });
                }
            }
            obstacles.extend(shapes.into_iter().map(|shape| core::Obstacle {
                shape,
                layers: copper,
                kind: core::ObstacleKind::Copper,
                net,
                clearance: if net.is_some() { 0.0 } else { clearance },
                clearance_override: None,
                plated: false,
                blocks_tracks: true,
                blocks_vias: true,
                label: format!("copper {kind}"),
            }));
        }
    }
}
