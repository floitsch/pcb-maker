// Copyright (C) 2026 Toit contributors.

//! A footprint's pad: its hole, its copper (a padstack's other shapes, a
//! via in it), its mask apertures and its terminal, or, without copper,
//! its mask opening.

use super::footprint::Footprint;
use super::*;
use pcb_router as core;

/// A pad with copper as it is lowered: its names, its copper's layers and
/// net, and the clearances it keeps.
struct Pad<'a> {
    expr: &'a Expr,
    /// `<reference>.<number>`.
    label: String,
    /// Its number, without quotes (empty for none).
    number: &'a str,
    /// Its type: `thru_hole`, `np_thru_hole`, `smd` or `connect`.
    kind: &'a str,
    /// KiCad's zone connection: the pad's own, else its footprint's (0
    /// none, 1 thermal, 2 solid).
    zone_connect: Option<&'a str>,
    lowered: LoweredPad,
    /// Its copper layers (`LayerTable::pad_mask`), never 0.
    layers: core::LayerMask,
    net: Option<core::NetId>,
    clearance: f64,
    clearance_override: Option<f64>,
    /// Its own copper's obstacle.
    obstacle: usize,
}

/// The layer names of a pad's `layers` form.
fn listed_layers(pad: &Expr) -> Vec<&str> {
    pad.child("layers")
        .map(Expr::children)
        .unwrap_or_default()
        .iter()
        .skip(1)
        .filter_map(Expr::atom)
        .collect()
}

/// Whether `names` open the front and the back solder mask.
fn mask_layers_open(names: &[&str]) -> [bool; 2] {
    [
        names
            .iter()
            .any(|name| matches!(*name, "F.Mask" | "*.Mask" | "F&B.Mask")),
        names
            .iter()
            .any(|name| matches!(*name, "B.Mask" | "*.Mask" | "F&B.Mask")),
    ]
}

impl Builder {
    /// A pad of `footprint`: its zone connection, hole, copper, via in pad,
    /// mask apertures, plated hole and terminal, or, without copper, its
    /// mask opening.
    pub(super) fn pad(
        &mut self,
        ctx: &Context,
        footprint: &mut Footprint,
        pad: &Expr,
    ) -> Result<(), String> {
        let item = footprint.item;
        let pad_name = pad.children().get(1).and_then(Expr::atom).unwrap_or("");
        let label = format!("{}.{}", footprint.reference, pad_name.trim_matches('"'));
        // KiCad's zone connection: the pad's own, else its footprint's (0
        // none, 1 thermal, 2 solid).
        let zone_connect =
            form_atom(pad, "zone_connect", 1).or_else(|| form_atom(item, "zone_connect", 1));
        match zone_connect {
            Some("0") => self.isolated_pads.push(label.clone()),
            Some("2") => self.solid_pads.push(label.clone()),
            _ => {}
        }
        let lowered = lower_pad(pad, footprint.at)?;
        let pad_type = pad.children().get(2).and_then(Expr::atom).unwrap_or("");
        self.pad_hole(ctx, item, pad, pad_type, &label, lowered.center)?;
        // A plated hole is copper: a through-hole pad is copper on every
        // layer to KiCad 10.0.7 whatever it lists (`LayerTable::pad_mask`):
        // one that lists no copper layer (`(layers "*.Mask")`, aaronds's
        // ESP32 DevKitC mounting pins; kicad-cli: shorting_items on F.Cu, an
        // inner layer and B.Cu) and one that lists some (koeg's pin headers
        // without In2.Cu: a track there shorted J5.3).
        let pad_layers = ctx.layers.pad_mask(pad)?;
        if pad_layers == 0 {
            self.mask_only_pad_opening(ctx, item, pad, &label, &lowered);
            return Ok(());
        }
        let net_name = node_net(pad).map(|raw| ctx.net_names.normalize(raw));
        let net = node_net(pad)
            .filter(|raw| ctx.net_names.routable(raw))
            .map(|raw| ctx.net_names.normalize(raw))
            .map(|name| self.net_id(ctx, name))
            .transpose()?;
        // A positive pad or footprint override replaces the net class's
        // clearance in KiCad, also when smaller.
        let clearance_override =
            Some(local_clearance::pad_clearance(pad, item)?).filter(|value| *value > 0.0);
        let mut clearance = 0.0;
        if net.is_none() {
            // A pad the router never connects (single-pin net, unconnected
            // pin, no net) still keeps its net class's clearance in KiCad's
            // DRC.
            let rules = net_name
                .and_then(|name| ctx.config.connection_rules.get(name))
                .or(ctx.config.default_rules.as_ref());
            if let Some(rules) = rules {
                clearance = rules.clearance_mm;
            }
        }
        let pad = Pad {
            expr: pad,
            label,
            number: pad_name.trim_matches('"'),
            kind: pad_type,
            zone_connect,
            lowered,
            layers: pad_layers,
            net,
            clearance,
            clearance_override,
            obstacle: self.obstacles.len(),
        };
        self.pad_connections(footprint, &pad);
        let back_geometry = self.pad_copper(ctx, footprint, &pad)?;
        let via_in_pad = self.via_in_pad(ctx, &pad);
        self.pad_mask_apertures(ctx, item, &pad, back_geometry.as_ref());
        self.pad_plated_hole(ctx, &pad)?;
        if pad.net.is_some() {
            self.pad_terminal(ctx, item, pad, via_in_pad);
        }
        Ok(())
    }

    /// The hole of a pad that is no copper of a routed net: plated holes of
    /// pads without a net are just holes to everyone; the board's hole
    /// clearance applies. Holes of mechanical (unplated) pads always are.
    fn pad_hole(
        &mut self,
        ctx: &Context,
        item: &Expr,
        pad: &Expr,
        pad_type: &str,
        label: &str,
        center: [f64; 2],
    ) -> Result<(), String> {
        let hole = (pad_type == "thru_hole"
            && !node_net(pad).is_some_and(|raw| ctx.net_names.routable(raw)))
            || pad_type == "np_thru_hole";
        if hole && let Some(shape) = drill_shape(pad, center)? {
            // KiCad 10.0.7's copper_edge_clearance takes an unplated slot (a
            // milled hole, its drill sizes differ) for board edge: copper
            // keeps the edge clearance from it (facade60's H5 and H8, the
            // USB-C shell slots of buganini's J1 and J3). A round hole keeps
            // only the hole clearance.
            let edge = if pad_type == "np_thru_hole" && matches!(shape, core::Shape::Capsule { .. })
            {
                ctx.config.edge_clearance_mm
            } else {
                0.0
            };
            self.obstacles.push(core::Obstacle {
                shape,
                layers: ctx.layers.all(),
                kind: core::ObstacleKind::Hole,
                net: None,
                clearance: local_clearance::pad_clearance(pad, item)?.max(edge),
                clearance_override: None,
                plated: false,
                blocks_tracks: true,
                blocks_vias: true,
                label: if pad_type == "np_thru_hole" {
                    format!("{label} hole")
                } else {
                    format!("{label} plated hole")
                },
            });
        }
        Ok(())
    }

    /// The queued mask opening of a pad without copper. A pad with a mask
    /// opening and no copper (hackrf's mask-only test points over a track)
    /// bares the copper under it: KiCad reports a solder mask bridge when
    /// copper of a second net lies there. It is an opening like a mask
    /// graphic, grown by the pad's mask expansion.
    fn mask_only_pad_opening(
        &mut self,
        ctx: &Context,
        item: &Expr,
        pad: &Expr,
        label: &str,
        lowered: &LoweredPad,
    ) {
        if ctx.config.mask_only_pad_openings == Some(false) {
            return;
        }
        let [front, back] = mask_layers_open(&listed_layers(pad));
        let margin = form_f64(pad, "solder_mask_margin", 1)
            .ok()
            .or_else(|| form_f64(item, "solder_mask_margin", 1).ok())
            .unwrap_or(ctx.mask.board_pad_to_mask)
            .max(0.0);
        let kind = format!("pad {label}");
        for (open, side) in [(front, 0), (back, ctx.layers.len() - 1)] {
            if open {
                self.mask_openings.push(MaskOpening {
                    shapes: vec![shape(&lowered.geometry)],
                    layer: side,
                    kind: kind.clone(),
                    margin,
                    expansion: margin,
                });
            }
        }
    }

    /// What ties the pad to others and to the pours: its net tie and
    /// jumper groups, its zone connection by obstacle and its thermal
    /// spokes' frame.
    fn pad_connections(&mut self, footprint: &mut Footprint, pad: &Pad) {
        let number = pad.number;
        if let Some(net) = pad.net
            && footprint
                .tie_groups
                .iter()
                .any(|group| group.iter().any(|member| member == number))
        {
            footprint
                .tie_pads
                .push((pad.lowered.center, pad.layers, net));
        }
        if let Some(group) = footprint
            .pad_groups
            .iter()
            .position(|group| group.iter().any(|member| member == number))
        {
            self.jumper_groups.insert(
                pad.obstacle,
                (self.footprint_ordinal, format!("group {group}")),
            );
        } else if footprint.duplicates_are_jumpers && !number.is_empty() {
            self.jumper_groups.insert(
                pad.obstacle,
                (self.footprint_ordinal, format!("number {number}")),
            );
        }
        match pad.zone_connect {
            Some("0") => self.isolated_pad_obstacles.push(pad.obstacle),
            Some("2") => self.solid_pad_obstacles.push(pad.obstacle),
            _ => {}
        }
        if let Ok(frame) = pad_spokes_frame(pad.expr, pad.lowered.center) {
            self.pad_spokes.insert(pad.obstacle, frame);
        }
    }

    /// The pad's copper: its own shape, and a padstack's shapes on the
    /// layers its stack names; the pad joins `footprint`'s pads. Returns
    /// the back copper's geometry where a padstack draws it.
    fn pad_copper(
        &mut self,
        ctx: &Context,
        footprint: &mut Footprint,
        pad: &Pad,
    ) -> Result<Option<ObstacleGeometry>, String> {
        let pad_layers = pad.layers;
        // A KiCad 9 padstack draws the pad in another shape or size on its
        // inner or back layers (adrianmarple's Raspberry Pi header: 1.2 mm
        // on F.Cu, 1.778 on B.Cu; dennisss's lens mounts: 4 mm and 3): the
        // pad's own shape is copper only on the layers the stack does not
        // name (F.Cu always), each named layer has its entry's shape, a
        // copper obstacle of the pad's net merged into the pad's terminal
        // (`MergedPad::padstack`). A smaller back shape under the front one
        // left phantom copper: a B.Cu track ending there was "connected" to
        // the model and not to KiCad. (A pad whose every layer the stack
        // names keeps its own shape on all of them.)
        let variants: Vec<(core::LayerMask, Expr)> = padstack_variants(pad.expr, &ctx.layers)
            .into_iter()
            .map(|(mask, variant)| (mask & pad_layers, variant))
            .filter(|(mask, _)| *mask != 0)
            .collect();
        let named = variants.iter().fold(0, |union, (mask, _)| union | mask);
        let own_layers = match pad_layers & !named {
            0 => pad_layers,
            own => own,
        };
        self.obstacles.push(core::Obstacle {
            shape: shape(&pad.lowered.geometry),
            layers: own_layers,
            kind: core::ObstacleKind::Copper,
            net: pad.net,
            clearance: pad.clearance,
            clearance_override: pad.clearance_override,
            plated: pad.kind == "thru_hole",
            blocks_tracks: true,
            blocks_vias: true,
            label: pad.label.clone(),
        });
        let smd = matches!(pad.kind, "smd" | "connect");
        footprint.pads.push(PadBox {
            number: pad.number.to_string(),
            center: pad.lowered.center,
            smd,
            bounds: {
                let bounds = self.obstacles[pad.obstacle].shape.aabb();
                [bounds.minimum, bounds.maximum]
            },
        });
        footprint.copper.push(vec![pad.obstacle]);
        let mut back_geometry = None;
        for (mask, variant) in variants {
            let geometry = lower_pad(&variant, footprint.at)?.geometry;
            if mask & (1 << (ctx.layers.len() - 1)) != 0 {
                back_geometry = Some(geometry.clone());
            }
            let variant_obstacle = self.obstacles.len();
            if let Some(own) = footprint.copper.last_mut() {
                own.push(variant_obstacle);
            }
            if own_layers != pad_layers {
                // The pad's copper on those layers: its zone connection,
                // its spokes' frame in its own size, its terminal's.
                match pad.zone_connect {
                    Some("0") => self.isolated_pad_obstacles.push(variant_obstacle),
                    Some("2") => self.solid_pad_obstacles.push(variant_obstacle),
                    _ => {}
                }
                if let Ok(frame) = pad_spokes_frame(&variant, pad.lowered.center) {
                    self.pad_spokes.insert(variant_obstacle, frame);
                }
                if pad.net.is_some() {
                    self.padstack_copper.push((pad.obstacle, variant_obstacle));
                }
            }
            self.obstacles.push(core::Obstacle {
                shape: shape(&geometry),
                layers: mask,
                kind: core::ObstacleKind::Copper,
                net: pad.net,
                clearance: pad.clearance,
                clearance_override: pad.clearance_override,
                plated: pad.kind == "thru_hole",
                blocks_tracks: true,
                blocks_vias: true,
                label: format!("{} padstack", pad.label),
            });
        }
        Ok(back_geometry)
    }

    /// A via in the pad (`Context::pad_vias`), if one is planned there for
    /// a pad of a net: its copper on the pad's layer, and on the other
    /// layers the pad's terminal there (a padstack's copper, joining a pour
    /// solidly as KiCad joins a via), its drill a hole of the net. Returns
    /// whether the pad has one.
    fn via_in_pad(&mut self, ctx: &Context, pad: &Pad) -> bool {
        let center = pad.lowered.center;
        let Some(via) = ctx
            .pad_vias
            .get(&pad.label)
            .and_then(|vias| {
                vias.iter().find(|via| {
                    (via.pad_at[0] - center[0]).abs() < 1.0e-6
                        && (via.pad_at[1] - center[1]).abs() < 1.0e-6
                })
            })
            .filter(|_| pad.net.is_some())
        else {
            return false;
        };
        self.holding_vias.insert(pad.obstacle);
        let ring = core::Shape::Circle {
            center: via.at,
            radius: via.diameter / 2.0,
        };
        for (via_layers, suffix) in [
            (pad.layers, "via in pad ring"),
            (ctx.layers.all() & !pad.layers, "via in pad"),
        ] {
            if via_layers == 0 {
                continue;
            }
            let index = self.obstacles.len();
            if suffix == "via in pad" {
                self.padstack_copper.push((pad.obstacle, index));
                self.solid_pad_obstacles.push(index);
            }
            self.obstacles.push(core::Obstacle {
                shape: ring.clone(),
                layers: via_layers,
                kind: core::ObstacleKind::Copper,
                net: pad.net,
                clearance: pad.clearance,
                clearance_override: None,
                plated: false,
                blocks_tracks: true,
                blocks_vias: true,
                label: format!("{} {suffix}", pad.label),
            });
        }
        self.obstacles.push(core::Obstacle {
            shape: core::Shape::Circle {
                center: via.at,
                radius: via.drill / 2.0,
            },
            layers: ctx.layers.all(),
            kind: core::ObstacleKind::Hole,
            net: pad.net,
            clearance: 0.0,
            clearance_override: None,
            plated: false,
            blocks_tracks: true,
            blocks_vias: true,
            label: format!("{} via in pad hole", pad.label),
        });
        true
    }

    /// The pad's mask aperture, where it reaches past the copper clearance:
    /// keepouts of the pad's net on each outer side it opens.
    /// (`back_geometry`: a padstack's back copper, the back's aperture.)
    /// A mask opening of the pad's net; its own net's copper never bridges
    /// (KiCad skips pairs of one net). KiCad's bridge test (10.0.7,
    /// DRC_TEST_PROVIDER_SOLDER_MASK): covered copper (a track, a tented
    /// via) bridges within the project's solder-mask-to-copper clearance
    /// plus the pad's expansion (OpenESC 4in1's J1: margin 0.102 mm plus
    /// 0.005, tracks 0.105 mm from the pads bridged); an untented via is an
    /// opening itself and bridges within the minimum web plus both
    /// expansions. Kept from before, for a pad or footprint margin of its
    /// own: the margin plus half the web (a "Hole, 3mm" footprint, a ring
    /// of small pads with 0.1 mm margins whose apertures merge into a
    /// mask-free annulus).
    fn pad_mask_apertures(
        &mut self,
        ctx: &Context,
        item: &Expr,
        pad: &Pad,
        back_geometry: Option<&ObstacleGeometry>,
    ) {
        let mask = &ctx.mask;
        let pad_layers = pad.layers;
        let local_margin = form_f64(pad.expr, "solder_mask_margin", 1)
            .ok()
            .or_else(|| form_f64(item, "solder_mask_margin", 1).ok());
        let expansion = local_margin.unwrap_or(mask.board_pad_to_mask);
        let web_reach = local_margin
            .filter(|margin| *margin > 0.0)
            .map_or(0.0, |margin| margin + mask.pad_mask_reach);
        let covered = if mask.strict {
            web_reach.max(mask.mask_to_copper + expansion)
        } else {
            web_reach
        };
        let floor = ctx
            .smallest_clearance
            .min(pad.clearance_override.unwrap_or(f64::INFINITY))
            - 1.0e-9;
        let [front, back] = mask_layers_open(&listed_layers(pad.expr));
        let sides = [(0usize, front), (ctx.layers.len() - 1, back)];
        for (index, (side, open)) in sides.into_iter().enumerate() {
            // The looser rule: every outer copper layer of the pad, mask
            // layer or not, both sides in one.
            // A pad opens the mask on a side it has no copper on when its
            // layers say so (c-lydia's U3: copper on B.Cu, the opening on
            // F.Mask): KiCad bridges the pad to other nets' copper under
            // that opening.
            let bare_side = mask.strict && open && pad_layers & (1 << side) == 0;
            let open = open || !mask.strict;
            if !open || (pad_layers & (1 << side) == 0 && !bare_side) || (index == 1 && side == 0) {
                continue;
            }
            let via_reach = if mask.tented[index] || !mask.strict {
                covered
            } else {
                web_reach.max(mask.min_mask_web + expansion + mask.board_pad_to_mask)
            };
            // The back's aperture is the back copper's.
            let side_geometry = match back_geometry {
                Some(geometry) if index == 1 => geometry,
                _ => &pad.lowered.geometry,
            };
            let aperture =
                |clearance: f64, blocks_tracks: bool, blocks_vias: bool| core::Obstacle {
                    shape: shape(side_geometry),
                    layers: 1 << side,
                    kind: core::ObstacleKind::Keepout,
                    net: pad.net,
                    clearance,
                    clearance_override: None,
                    plated: false,
                    blocks_tracks,
                    blocks_vias,
                    label: format!("{} mask aperture", pad.label),
                };
            // A reach at the copper clearance adds nothing (KiCad's tests
            // are strict on both); the older rule keeps its own threshold
            // (USB_Cable_Tester: 0.05 + 0.05 mm against classes of 0.1
            // rerouted for nothing). Without copper on this side no copper
            // clearance holds for it: the aperture is all there is.
            let needed = |reach: f64| {
                bare_side || reach > floor + 2.0e-9 || (reach == web_reach && reach > floor)
            };
            if (via_reach - covered).abs() < 1.0e-9 {
                if needed(covered) {
                    self.obstacles.push(aperture(covered, true, true));
                }
            } else {
                if needed(covered) {
                    self.obstacles.push(aperture(covered, true, false));
                }
                if needed(via_reach) {
                    self.obstacles.push(aperture(via_reach, false, true));
                }
            }
        }
    }

    /// The plated hole of a through-hole pad of a routed net. Other nets
    /// keep the hole clearance from a plated hole too, which reaches past a
    /// thin annular ring (a thermal via pad: 0.3 drill in 0.5 copper,
    /// eurorack-pmod's U1). Their vias as well (jetson's J2: 0.55 drill in
    /// 0.75 copper, clearance 0.125, hole clearance 0.25); the pad's own
    /// net is exempt (`StaticMaps::via_owner`).
    fn pad_plated_hole(&mut self, ctx: &Context, pad: &Pad) -> Result<(), String> {
        if pad.kind == "thru_hole"
            && pad.net.is_some()
            && let Some(shape) = drill_shape(pad.expr, pad.lowered.center)?
        {
            self.obstacles.push(core::Obstacle {
                shape,
                layers: ctx.layers.all(),
                kind: core::ObstacleKind::Hole,
                net: pad.net,
                clearance: 0.0,
                clearance_override: None,
                plated: false,
                blocks_tracks: true,
                blocks_vias: true,
                label: format!("{} plated hole", pad.label),
            });
        }
        Ok(())
    }

    /// The terminal of a pad of a routed net, on the layers it is reached
    /// on (every layer with a via in it).
    fn pad_terminal(&mut self, ctx: &Context, item: &Expr, pad: Pad, via_in_pad: bool) {
        let Some(net) = pad.net else {
            return;
        };
        let pad_layers = pad.layers;
        // Without a hole, a pad on several layers (an edge-mount
        // connector's top and bottom pads) does not join them: it is
        // reached on its footprint's side, and layer changes need a via.
        let mut terminal_layers = pad_layers;
        if pad.expr.child("drill").is_none() && pad_layers.count_ones() > 1 {
            let side = form_atom(item, "layer", 1)
                .and_then(|name| ctx.layers.index(name))
                .map(|layer| 1 << layer)
                .filter(|mask: &core::LayerMask| mask & pad_layers != 0);
            terminal_layers = side.unwrap_or(pad_layers.isolate_lowest_one());
        }
        // A via in the pad reaches it on every layer.
        if via_in_pad {
            terminal_layers = ctx.layers.all();
        }
        // A custom pad connects through its anchor only.
        let contact = match (
            pad.expr.children().get(3).and_then(Expr::atom),
            &pad.lowered.geometry,
        ) {
            (Some("custom"), ObstacleGeometry::Union { parts }) if parts.len() > 1 => {
                Some(shape(&parts[0]))
            }
            _ => None,
        };
        self.nets[net as usize].terminals.push(core::Terminal {
            anchor: pad.lowered.center,
            layers: terminal_layers,
            pad: pad.obstacle,
            contact,
            label: pad.label,
        });
    }
}

/// The drilled hole of a pad: a circle, or a capsule for a slot
/// (`(drill oval 1 7)`), centred on the pad.
fn drill_shape(pad: &Expr, center: [f64; 2]) -> Result<Option<core::Shape>, String> {
    let Some(drill) = pad.child("drill") else {
        return Ok(None);
    };
    let values: Vec<f64> = drill
        .children()
        .iter()
        .skip(1)
        .filter_map(Expr::atom)
        .filter_map(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .collect();
    let Some(&diameter) = values.first().filter(|diameter| **diameter > 0.0) else {
        return Ok(None);
    };
    let (sin, cos) = quarter_sin_cos(-form_at(pad)?[2]);
    Ok(Some(match values.get(1) {
        Some(&height) if (height - diameter).abs() > 1.0e-9 => {
            let (long, short) = (diameter.max(height), diameter.min(height));
            let axis = if diameter >= height {
                [1.0, 0.0]
            } else {
                [0.0, 1.0]
            };
            let half = (long - short) / 2.0;
            let offset = [
                half * (axis[0] * cos - axis[1] * sin),
                half * (axis[0] * sin + axis[1] * cos),
            ];
            core::Shape::Capsule {
                start: [center[0] - offset[0], center[1] - offset[1]],
                end: [center[0] + offset[0], center[1] + offset[1]],
                radius: short / 2.0,
            }
        }
        _ => core::Shape::Circle {
            center,
            radius: diameter / 2.0,
        },
    }))
}

/// KiCad's frame for a pad's thermal spokes (`core::PadSpokes`): its shape
/// position, size at orientation 0, absolute orientation and spoke angle
/// (the pad's `thermal_bridge_angle`, else 45 degrees for a circle and 90
/// for any other shape, a custom pad's by its anchor). A custom pad's
/// spokes run from the centre of its copper's box at orientation 0 (its
/// anchor and primitives, `custom_pad_local_geometry`) to that box grown
/// by the gap, at its spoke angle, turned with the pad: KiCad's filler
/// takes the dummy pad's bounding box (SNSP-CPU-01's P7.1, a JST pad with
/// a circle anchor in a polygon: diagonal spokes, none landing, no
/// finding; the model's along the box's axes landed one into an island,
/// a starved pad KiCad does not report).
fn pad_spokes_frame(pad: &Expr, center: [f64; 2]) -> Result<core::PadSpokes, String> {
    let pad_at = form_at(pad)?;
    let mut size = form_xy(pad, "size")?;
    let shape = pad.children().get(3).and_then(Expr::atom).unwrap_or("rect");
    let anchor = if shape == "custom" {
        pad.child("options")
            .and_then(|options| form_atom(options, "anchor", 1))
            .unwrap_or("circle")
    } else {
        shape
    };
    let offset = match pad
        .child("drill")
        .filter(|drill| drill.child("offset").is_some())
    {
        Some(drill) => {
            let (x, y) = (
                form_f64(drill, "offset", 1).unwrap_or(0.0),
                form_f64(drill, "offset", 2).unwrap_or(0.0),
            );
            let (sine, cosine) = quarter_sin_cos(-pad_at[2]);
            [x * cosine - y * sine, x * sine + y * cosine]
        }
        None => [0.0, 0.0],
    };
    let mut offset = offset;
    if shape == "custom" {
        let bounds = super::shape(&custom_pad_local_geometry(pad)?).aabb();
        let middle = [
            (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
            (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
        ];
        size = [
            bounds.maximum[0] - bounds.minimum[0],
            bounds.maximum[1] - bounds.minimum[1],
        ];
        let turned = rotate_vector(middle, -pad_at[2]);
        offset = [offset[0] + turned[0], offset[1] + turned[1]];
    }
    Ok(core::PadSpokes {
        center: [center[0] + offset[0], center[1] + offset[1]],
        half: [size[0] / 2.0, size[1] / 2.0],
        orientation: pad_at[2],
        spoke_angle: form_f64(pad, "thermal_bridge_angle", 1).unwrap_or(if anchor == "circle" {
            45.0
        } else {
            90.0
        }),
        circular: shape == "circle" || (shape == "oval" && (size[0] - size[1]).abs() < 1.0e-9),
        custom: shape == "custom",
    })
}

/// A KiCad 9 padstack's other layers (`(padstack (mode front_inner_back)
/// (layer "Inner" ...) (layer "B.Cu" ...))`, or `(mode custom)` naming
/// copper layers): the layers each entry draws and the pad with the
/// entry's shape, size and other forms in place of its own. The pad's own
/// forms are the front's (and, in custom mode, every layer's the stack
/// does not name).
fn padstack_variants(pad: &Expr, layers: &LayerTable) -> Vec<(core::LayerMask, Expr)> {
    let Some(stack) = pad.child("padstack") else {
        return Vec::new();
    };
    let back = layers.len() - 1;
    let inner: core::LayerMask = (1..back).fold(0, |mask, layer| mask | 1 << layer);
    stack
        .children()
        .iter()
        .filter(|entry| entry.head() == Some("layer"))
        .filter_map(|entry| {
            let name = entry.children().get(1)?.atom()?;
            let mask = match name {
                "Inner" => inner,
                "F.Cu" => return None,
                name => 1 << layers.index(name)?,
            };
            let Expr::List(mut parts) = pad.clone() else {
                return None;
            };
            for form in entry.children().iter().skip(2) {
                match form.head() {
                    Some("shape") => {
                        if let (Some(slot), Some(atom)) = (parts.get_mut(3), form.children().get(1))
                        {
                            *slot = atom.clone();
                        }
                    }
                    // A layer's shape offset (KiCad 9's padstack: OpenESC's
                    // motor pads, 1.8 x 0.6 mm at -0.4 mm inside and on the
                    // back, the front's 1.8 x 3 mm at the drill's -0.5 mm) is
                    // the drill's offset of that layer's copper, which the
                    // pad's shape reads: as a form of its own it was not read,
                    // and the back copper sat 0.1 mm off (KiCad: clearance
                    // 0.107-0.153 mm where the rule asks 0.16).
                    Some("offset") if parts.iter().any(|part| part.head() == Some("drill")) => {
                        if let Some(Expr::List(drill)) =
                            parts.iter_mut().find(|part| part.head() == Some("drill"))
                        {
                            match drill.iter_mut().find(|part| part.head() == Some("offset")) {
                                Some(offset) => *offset = form.clone(),
                                None => drill.push(form.clone()),
                            }
                        }
                    }
                    Some(head) => match parts.iter_mut().find(|part| part.head() == Some(head)) {
                        Some(part) => *part = form.clone(),
                        None => parts.push(form.clone()),
                    },
                    None => {}
                }
            }
            Some((mask, Expr::List(parts)))
        })
        .collect()
}
