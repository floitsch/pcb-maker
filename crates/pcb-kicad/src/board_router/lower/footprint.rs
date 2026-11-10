// Copyright (C) 2026 Toit contributors.

//! A footprint's objects: its rule areas, its graphics and texts on copper
//! and mask, its pads (pad.rs), and what its pads give together (pin-row
//! gaps, a fan-out's part, the pads closed to vias, a net tie's copper).

use super::*;
use pcb_router as core;

/// How far beyond a track's reach a net tie's copper is cut back from its
/// pads' centres: room for the lattice node that ends the track (half the
/// coarsest pitch and more).
const NET_TIE_MARGIN: f64 = 0.1;

/// A net tie's copper graphic, waiting for its pads: shape, layers, label.
type TieCopper = (core::Shape, core::LayerMask, String);

/// A footprint as its pads are lowered: where it is, what joins its pads,
/// and what its pads have given so far.
pub(super) struct Footprint<'a> {
    pub item: &'a Expr,
    pub at: [f64; 3],
    pub reference: String,
    /// Whether it is a touch electrode (`touch_electrode`).
    pub electrode: bool,
    /// A net tie's pads (`net_tie_pad_groups`): its copper graphics join
    /// pads of several nets, which KiCad allows.
    pub tie_groups: Vec<Vec<String>>,
    /// Its net tie's copper (`TieCopper`), lowered after its pads.
    pub tie_copper: Vec<TieCopper>,
    /// Its net tie's pads of a net: centre, copper layers, net.
    pub tie_pads: Vec<([f64; 2], core::LayerMask, core::NetId)>,
    /// KiCad 9's `(duplicate_pad_numbers_are_jumpers yes)`.
    pub duplicates_are_jumpers: bool,
    /// KiCad 9's `(jumper_pad_groups ("1" "2") ...)`, by pad number.
    pub pad_groups: Vec<Vec<String>>,
    /// The footprint's pads with copper: centre, and for an SMD pad its
    /// copper's obstacles and area (`exposed_pad`).
    pub pads: Vec<PadBox>,
    /// Per pad of `pads`, its copper's obstacles (the pad, a padstack's
    /// other shapes).
    pub copper: Vec<Vec<usize>>,
}

impl<'a> Footprint<'a> {
    /// The footprint `item`'s place, reference and pad groups, or the
    /// board's error for a footprint whose forms cannot be read.
    fn new(ctx: &Context, item: &'a Expr) -> Result<Self, String> {
        if let Some(why) = footprint_form_error(item) {
            return Err(format!("the board has {why}"));
        }
        let at = form_at(item)?;
        let reference = footprint_reference(item).unwrap_or_default();
        let electrode = touch_electrode(item);
        let tie_groups = net_tie_groups(item, ctx.board_version);
        let duplicates_are_jumpers =
            form_atom(item, "duplicate_pad_numbers_are_jumpers", 1) == Some("yes");
        let pad_groups: Vec<Vec<String>> = item
            .child("jumper_pad_groups")
            .map(Expr::children)
            .unwrap_or_default()
            .iter()
            .skip(1)
            .map(|group| {
                group
                    .children()
                    .iter()
                    .filter_map(Expr::atom)
                    .map(|number| number.trim_matches('"').to_string())
                    .collect()
            })
            .collect();
        Ok(Self {
            item,
            at,
            reference,
            electrode,
            tie_groups,
            tie_copper: Vec::new(),
            tie_pads: Vec::new(),
            duplicates_are_jumpers,
            pad_groups,
            pads: Vec::new(),
            copper: Vec::new(),
        })
    }

    /// The shapes of one of the footprint's graphics, placed on the board.
    fn placed_shapes(&self, graphic: &Expr) -> Result<Vec<core::Shape>, String> {
        Ok(copper_graphic_shapes(graphic)?
            .into_iter()
            .map(|shape| place_shape(shape, self.at))
            .collect())
    }
}

impl Builder {
    /// A footprint: its rule areas, graphics, texts and pads, then what its
    /// pads give together (pin-row gaps, a fan-out's part, the pads closed
    /// to vias) and its net tie's copper.
    pub(super) fn footprint(&mut self, ctx: &Context, item: &Expr) -> Result<(), String> {
        let mut footprint = Footprint::new(ctx, item)?;
        self.footprint_rule_areas(ctx, item, &footprint.reference)?;
        self.footprint_graphics(ctx, &mut footprint)?;
        self.footprint_texts(ctx, &footprint)?;
        self.footprint_ordinal += 1;
        for pad in item
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            self.pad(ctx, &mut footprint, pad)?;
        }
        self.footprint_pin_row_gaps(&footprint);
        self.fanout_part(ctx, &footprint);
        self.footprint_no_via_pads(ctx, &footprint);
        self.net_tie_copper(ctx, footprint);
        Ok(())
    }

    /// The footprint's graphics: mask openings (queued), Margin keepouts,
    /// and copper of no net (a net tie's held in `tie_copper`).
    fn footprint_graphics(
        &mut self,
        ctx: &Context,
        footprint: &mut Footprint,
    ) -> Result<(), String> {
        let item = footprint.item;
        let reference = &footprint.reference;
        // Copper drawn inside the footprint (a logo, a net tie's bridge)
        // belongs to no net.
        for graphic in item.children().iter().filter(|child| {
            matches!(
                child.head(),
                Some("fp_line" | "fp_arc" | "fp_rect" | "fp_circle" | "fp_poly")
            )
        }) {
            // KiCad 9 draws a graphic on several layers at once
            // (`(layers "B.Cu" "B.Mask")`: tuxkeychain's GRAPHIC3 logo,
            // copper and a mask opening in one polygon).
            let names = item_layer_names(graphic);
            // Mask graphics open the mask like the board's own (Sisu's 190
            // B.Mask polygons, link's connector).
            let sides = mask_sides(&names, ctx.layers.len());
            if !sides.is_empty() {
                let shapes = footprint.placed_shapes(graphic)?;
                if !shapes.is_empty() {
                    let expansion = form_f64(graphic, "solder_mask_margin", 1)
                        .ok()
                        .unwrap_or(ctx.mask.board_pad_to_mask)
                        .max(0.0);
                    // A touch electrode's own opening leaves its net the
                    // vias inside its pad (`grid.rs`, `own_opening`).
                    let kind = graphic.head().unwrap_or("graphic");
                    let kind = if footprint.electrode {
                        format!("{kind} of the touch electrode {reference}")
                    } else {
                        kind.to_string()
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
                }
            }
            if names.contains(&"Margin") {
                let shapes = footprint.placed_shapes(graphic)?;
                self.obstacles.extend(margin_obstacles(
                    shapes,
                    &ctx.layers,
                    ctx.config,
                    &format!("{reference} margin {}", graphic.head().unwrap_or("graphic")),
                ));
            }
            let copper = ctx.layers.mask_of(names.iter().copied())?;
            if copper == 0 {
                continue;
            }
            // A footprint graphic of a net is copper of no net to KiCad
            // 10.0.7 too: no pad or track connects to it, its own net's
            // copper keeps the clearance from it (kicad-cli: an antenna
            // polygon of net A against its own pad and an A track,
            // clearance findings at 0 mm and the pad unconnected).
            for shape in footprint.placed_shapes(graphic)? {
                let label = format!("{reference} copper {}", graphic.head().unwrap_or("graphic"));
                // A net tie's copper waits for its pads (`net_tie_copper`).
                if !footprint.tie_groups.is_empty() {
                    footprint.tie_copper.push((shape, copper, label));
                    continue;
                }
                self.obstacles.push(core::Obstacle {
                    shape,
                    layers: copper,
                    kind: core::ObstacleKind::Copper,
                    net: None,
                    clearance: ctx.no_net_clearance,
                    clearance_override: None,
                    plated: false,
                    blocks_tracks: true,
                    blocks_vias: true,
                    label,
                });
            }
        }
        Ok(())
    }

    /// The footprint's visible texts on copper as copper of no net, on a
    /// mask layer as openings (queued), as the board's are (jnftech's test
    /// points with their Value on F.Cu: KiCad's clearance and
    /// shorting_items against "Value field"). A text box (`fp_text_box`)
    /// is its box, glyphs inside.
    fn footprint_texts(&mut self, ctx: &Context, footprint: &Footprint) -> Result<(), String> {
        let reference = &footprint.reference;
        for text in footprint.item.children().iter().filter(|child| {
            matches!(child.head(), Some("fp_text" | "property" | "fp_text_box"))
                && child.child("layer").is_some()
        }) {
            let names = item_layer_names(text);
            let copper = ctx.layers.mask_of(names.iter().copied())?;
            let sides = mask_sides(&names, ctx.layers.len());
            if copper == 0 && sides.is_empty() {
                continue;
            }
            let (shapes, kind) = if text.head() == Some("fp_text_box") {
                (text_box_shapes(text)?, "text box")
            } else {
                let Some(shape) = footprint_text_box(text)? else {
                    continue;
                };
                (vec![shape], "text")
            };
            // A text box of fewer than three corners has no box
            // (`text_box_shapes`), nor has a hidden one: left out, as the
            // board's own are (`input_warnings` says so in the report).
            if shapes.is_empty() {
                continue;
            }
            let shapes: Vec<core::Shape> = shapes
                .into_iter()
                .map(|shape| place_shape(shape, footprint.at))
                .collect();
            for side in sides {
                self.mask_openings.push(MaskOpening {
                    shapes: shapes.clone(),
                    layer: side,
                    kind: format!("{reference} {kind}"),
                    margin: 0.0,
                    expansion: 0.0,
                });
            }
            if copper != 0 {
                self.obstacles
                    .extend(shapes.into_iter().map(|shape| core::Obstacle {
                        shape,
                        layers: copper,
                        kind: core::ObstacleKind::Copper,
                        net: None,
                        clearance: ctx.no_net_clearance,
                        clearance_override: None,
                        plated: false,
                        blocks_tracks: true,
                        blocks_vias: true,
                        label: format!("{reference} copper {kind}"),
                    }));
            }
        }
        Ok(())
    }

    /// The gaps between the footprint's adjacent SMD pads
    /// (`footprint_pin_row_gaps`).
    fn footprint_pin_row_gaps(&mut self, footprint: &Footprint) {
        self.pin_row_gaps.extend(footprint_pin_row_gaps(
            &footprint
                .pads
                .iter()
                .zip(&footprint.copper)
                .filter(|(pad, _)| pad.smd)
                .map(|(pad, copper)| {
                    let obstacle = &self.obstacles[copper[0]];
                    (pad.center, obstacle.layers, obstacle.net)
                })
                .collect::<Vec<_>>(),
        ));
    }

    /// With prepared fan-outs, a footprint of 16 SMD pads or more is a part
    /// for their plan: its SMD pads' copper and which are its exposed pad.
    fn fanout_part(&mut self, ctx: &Context, footprint: &Footprint) {
        if ctx.fanout_on && footprint.pads.iter().filter(|pad| pad.smd).count() >= 16 {
            let exposed = exposed_pad(&footprint.pads);
            self.fanout_parts.push(bga_fanout::FanoutPart {
                reference: footprint.reference.to_string(),
                pads: footprint
                    .pads
                    .iter()
                    .zip(&footprint.copper)
                    .enumerate()
                    .filter(|(_, (pad, _))| pad.smd)
                    .map(|(index, (_, copper))| (copper[0], exposed.contains(&index)))
                    .collect(),
            });
        }
    }

    /// The footprint's SMD pads no via may overlap: all but its exposed pad
    /// and a touch electrode's, unless every via is filled and capped.
    fn footprint_no_via_pads(&mut self, ctx: &Context, footprint: &Footprint) {
        if ctx.config.via_in_pad != Some(KiCadViaInPad::All(true)) {
            let exposed = exposed_pad(&footprint.pads);
            for (index, pad) in footprint.pads.iter().enumerate() {
                if pad.smd && !exposed.contains(&index) && !footprint.electrode {
                    self.no_via_pads.extend(&footprint.copper[index]);
                }
            }
        }
    }

    /// A net tie's copper, once its pads are known. KiCad 10.0.7 lets the
    /// tie's own nets touch it (kicad-cli: tracks of A and B ending on the
    /// tie's pads over its bridge, and an A track ending on the bridge's
    /// middle, find no clearance or short against it; only the other pad's
    /// clearance holds) and keeps every other net clear of it (the same
    /// footprint without `net_tie_pad_groups`: clearance and shorting_items
    /// against the polygon). One owner per lattice node cannot say "A or
    /// B", so the bridge is copper of no net less what lies within a
    /// track's reach of a tie pad's centre: a track of the pad's net ends
    /// there (neotron-pico's NT901 to NT912, every pad dead behind its own
    /// bridge before). Other nets keep clear of the rest and of the pads.
    fn net_tie_copper(&mut self, ctx: &Context, footprint: Footprint) {
        let tie_pads = footprint.tie_pads;
        for (shape, copper, label) in footprint.tie_copper {
            // The whole bridge keeps every via out, the tie's own nets'
            // too: a track of a tie net may touch the bridge, a drill may
            // not come within the hole clearance of it (KiCad's
            // hole_clearance: brown-studios/pdlc's EN via 0.164 mm from
            // NT1's bridge, carved away above for EN's tracks).
            self.obstacles.push(core::Obstacle {
                shape: shape.clone(),
                layers: copper,
                kind: core::ObstacleKind::Copper,
                net: None,
                clearance: ctx.no_net_clearance,
                clearance_override: None,
                plated: false,
                blocks_tracks: false,
                blocks_vias: true,
                label: format!("{label} (vias)"),
            });
            let carves: Vec<([f64; 2], f64)> = tie_pads
                .iter()
                .filter(|(_, pad_layers, _)| pad_layers & copper != 0)
                .map(|(center, _, net)| {
                    let class = &self.classes[self.nets[*net as usize].class];
                    (
                        *center,
                        class.clearance.max(ctx.no_net_clearance)
                            + class.trace_width / 2.0
                            + NET_TIE_MARGIN,
                    )
                })
                .collect();
            if let Some(shape) = carve_net_tie(shape, &carves) {
                self.obstacles.push(core::Obstacle {
                    shape,
                    layers: copper,
                    kind: core::ObstacleKind::Copper,
                    net: None,
                    clearance: ctx.no_net_clearance,
                    clearance_override: None,
                    plated: false,
                    blocks_tracks: true,
                    blocks_vias: true,
                    label,
                });
            }
        }
    }
}

/// Whether a footprint is a touch electrode: its only pad, of a custom
/// shape, is copper art no part is soldered to. d20_pcb's J2 is a face of
/// the die, enclosed by the footprint's own frame on F.Cu and by a keepout
/// on B.Cu, and under the footprint's mask opening; the designer reaches
/// it with a via in its corner. Its pad holds its net's vias (not in
/// `no_via_pads`) and its own mask opening leaves them there.
pub(super) fn touch_electrode(footprint: &Expr) -> bool {
    let mut pads = footprint
        .children()
        .iter()
        .filter(|child| child.head() == Some("pad"));
    matches!(
        (pads.next(), pads.next()),
        (Some(pad), None) if pad.children().get(3).and_then(Expr::atom) == Some("custom")
    )
}

/// The pad numbers a footprint's net tie joins, by group: KiCad 7's
/// `(net_tie_pad_groups "1, 2" ...)`, or for a board before version
/// 20220817 a footprint whose keywords start with "net tie", all its pads
/// in one group (KiCad 10.0.7 loads ghoul's KiCad 6 NetTie-2_SMD_Pad0.2mm
/// so; kicad-cli: a tie by `(tags "net tie")` at 20220815 and before, by
/// "net tie short" too, not by "Net Tie", "foo net tie" or a description,
/// and not at 20220817 and after).
pub(super) fn net_tie_groups(footprint: &Expr, board_version: Option<u64>) -> Vec<Vec<String>> {
    if let Some(groups) = footprint.child("net_tie_pad_groups") {
        return groups
            .children()
            .iter()
            .skip(1)
            .filter_map(Expr::atom)
            .map(|group| {
                group
                    .trim_matches('"')
                    .split(',')
                    .map(|number| number.trim().to_string())
                    .filter(|number| !number.is_empty())
                    .collect::<Vec<_>>()
            })
            .filter(|group| group.len() > 1)
            .collect();
    }
    let legacy = board_version.is_some_and(|version| version < 20220817)
        && form_atom(footprint, "tags", 1)
            .is_some_and(|tags| tags.trim_matches('"').starts_with("net tie"));
    if !legacy {
        return Vec::new();
    }
    let mut numbers: Vec<String> = Vec::new();
    for pad in footprint
        .children()
        .iter()
        .filter(|child| child.head() == Some("pad"))
    {
        let number = pad
            .children()
            .get(1)
            .and_then(Expr::atom)
            .unwrap_or("")
            .trim_matches('"');
        if !number.is_empty() && !numbers.iter().any(|known| known == number) {
            numbers.push(number.to_string());
        }
    }
    if numbers.len() > 1 {
        vec![numbers]
    } else {
        Vec::new()
    }
}

/// A net tie's copper graphic less the discs (centre, radius) its pads'
/// nets need to end their tracks: the graphic itself when no disc touches
/// it, else its cells (of 0.02 mm, coarser for a large graphic) whose
/// whole square lies inside no disc, as rectangles per row; None when
/// nothing is left.
pub(super) fn carve_net_tie(shape: core::Shape, carves: &[([f64; 2], f64)]) -> Option<core::Shape> {
    let bounds = shape.aabb();
    let width = bounds.maximum[0] - bounds.minimum[0];
    let height = bounds.maximum[1] - bounds.minimum[1];
    let mut step = 0.02_f64;
    while (width / step).ceil() * (height / step).ceil() > 250_000.0 {
        step *= 2.0;
    }
    let columns = ((width / step).ceil() as usize).max(1);
    let rows = ((height / step).ceil() as usize).max(1);
    let half_diagonal = step * std::f64::consts::FRAC_1_SQRT_2;
    let mut carved = false;
    let mut parts: Vec<core::Shape> = Vec::new();
    for row in 0..rows {
        let y = bounds.minimum[1] + (row as f64 + 0.5) * step;
        let mut run: Option<usize> = None;
        for column in 0..=columns {
            let kept = column < columns && {
                let x = bounds.minimum[0] + (column as f64 + 0.5) * step;
                let inside = shape.contains([x, y]);
                let free = carves.iter().all(|(center, radius)| {
                    (x - center[0]).hypot(y - center[1]) >= radius + half_diagonal
                });
                carved |= inside && !free;
                inside && free
            };
            match (kept, run) {
                (true, None) => run = Some(column),
                (false, Some(start)) => {
                    let x0 = bounds.minimum[0] + start as f64 * step;
                    let x1 = bounds.minimum[0] + column as f64 * step;
                    let (y0, y1) = (y - step / 2.0, y + step / 2.0);
                    parts.push(core::Shape::Polygon {
                        points: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
                    });
                    run = None;
                }
                _ => {}
            }
        }
    }
    if !carved {
        return Some(shape);
    }
    match parts.len() {
        0 => None,
        1 => parts.pop(),
        _ => Some(core::Shape::Union { parts }),
    }
}
