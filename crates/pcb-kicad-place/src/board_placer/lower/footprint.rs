// Copyright (C) 2026 Toit contributors.

//! A footprint lowered to the placer's part: its body, the sides it
//! occupies, its pins and pads, and how it blocks beyond its body's box.

use super::*;
use pcb_placer as core;

/// One footprint lowered (`lower_footprint`): its part and what the
/// problem keeps beside it per footprint.
pub(super) struct LoweredFootprint {
    pub component: core::Component,
    pub pose: core::Pose,
    pub reference: String,
    pub at: [f64; 3],
    pub locked: bool,
    pub uuid: Option<String>,
    pub connector: bool,
    pub pad_offsets: BTreeMap<String, [f64; 2]>,
    pub pad_nets: Vec<PadNet>,
    pub library_name: String,
    pub pad_boxes: Vec<[f64; 4]>,
    pub keepout_box: Option<[f64; 4]>,
    pub mouth: Option<[f64; 2]>,
}

/// Lowers one footprint to a part: takes the board's facts from `context`
/// and numbers each net its pins meet first in `net_ids`.
pub(super) fn lower_footprint(
    context: &LowerContext,
    footprint: &Expr,
    net_ids: &mut BTreeMap<String, usize>,
) -> Result<LoweredFootprint, String> {
    let config = context.config;
    let at = form_at(footprint)?;
    let reference = footprint_reference(footprint).unwrap_or_default();
    let (body_center, body_size, round) = local_body(footprint)?;
    let sides = FootprintSides::of(footprint);
    let SidedBody {
        body_center,
        body_size,
        far_courtyard,
    } = two_sided_body(footprint, config, &sides, body_center, body_size)?;
    let FootprintPads {
        pins,
        through,
        mut far_side,
        mut far_copper,
        own_pads,
        hole_rings,
        pin_centers,
        named,
        pad_nets,
    } = lower_pads(context, footprint, at, &sides, net_ids)?;
    let graphics = outer_graphics(context, footprint, &sides)?;
    // Artwork (a logo) has neither pads nor a courtyard: it occupies the
    // sides its copper and mask graphics are on (its body is their box),
    // and nothing without any, rather than being a 1 mm wall at its
    // origin.
    let artwork = !footprint.children().iter().any(|child| {
        child.head() == Some("pad")
            || (matches!(
                child.head(),
                Some("fp_line" | "fp_rect" | "fp_arc" | "fp_circle" | "fp_poly")
            ) && form_atom(child, "layer", 1).is_some_and(|layer| layer.ends_with(".CrtYd")))
    });
    // A through-hole part sits on its footprint's side; on the other
    // side only its holes and pads (`far_side`) are in the way, and its
    // copper and mask graphics there.
    let (own_side, other_side) = sides.sides();
    let side = if artwork {
        match (!graphics.own.is_empty(), !graphics.far.is_empty()) {
            (false, false) => core::Side::Neither,
            (true, false) => own_side,
            (false, true) => other_side,
            (true, true) => core::Side::Both,
        }
    } else {
        far_copper.extend(graphics.far.iter().copied());
        far_side.extend(far_courtyard);
        own_side
    };
    let far_graphics = !artwork && (!graphics.far.is_empty() || far_courtyard.is_some());
    let angle_options = angle_options(config, at[2]);
    let copper_graphics = copper_graphic_boxes(footprint)?;
    let edge_inset = edge_inset(&own_pads, &copper_graphics, body_center, body_size);
    let shape = body_shape(
        context,
        footprint,
        &sides,
        BodyParts {
            body_center,
            body_size,
            round,
            artwork,
            own_pads: &own_pads,
            pin_centers: &pin_centers,
            graphics: &graphics,
        },
    )?;
    let slots = footprint_slots(footprint)?;
    let connector = through && pins.len() >= 4;
    let locked = footprint.child("locked").is_some()
        || footprint
            .children()
            .iter()
            .any(|child| child.atom() == Some("locked"));
    let halo = halo(config, &reference, pins.len());
    let tight = if config.tight_bodies == Some(true) {
        tight_body(
            footprint,
            sides.both_courtyards().then_some(sides.other_prefix),
        )?
    } else {
        None
    };
    let component = core::Component {
        name: reference.clone(),
        body_center,
        body_size,
        round,
        halo,
        pins,
        side,
        fixed: false,
        angle_options,
        far_side: if through || far_graphics {
            far_side
        } else {
            Vec::new()
        },
        far_copper,
        far_open: Default::default(),
        far_courtyard: if artwork {
            Vec::new()
        } else {
            far_courtyard.into_iter().collect()
        },
        hollow: shape.hollow,
        tight,
        edge_inset,
        courtyards: shape.courtyards,
        holes_inside: shape.holes_inside,
        pads: own_pads.clone(),
        copper_only: false,
        cutout_outline: Vec::new(),
        tight_hollow: Vec::new(),
        hole_rings,
        copper_graphics,
        courtless: shape.courtless.into_iter().collect(),
        slots,
    };
    let library_name = footprint
        .children()
        .get(1)
        .and_then(Expr::atom)
        .unwrap_or_default()
        .to_string();
    Ok(LoweredFootprint {
        component,
        pose: core::Pose {
            position: [at[0], at[1]],
            angle: at[2],
        },
        reference,
        at,
        locked,
        uuid: footprint_uuid(footprint),
        connector,
        pad_offsets: named,
        pad_nets,
        library_name,
        pad_boxes: own_pads,
        keepout_box: footprint_keepout_box(footprint, at)?,
        mouth: connector_mouth(footprint)?,
    })
}

/// Whether a footprint has a courtyard graphic on the side `prefix`
/// ("F." or "B.").
fn courtyard_on(footprint: &Expr, prefix: &str) -> bool {
    footprint.children().iter().any(|child| {
        matches!(
            child.head(),
            Some("fp_line" | "fp_rect" | "fp_arc" | "fp_circle" | "fp_poly")
        ) && form_atom(child, "layer", 1)
            .is_some_and(|layer| layer.starts_with(prefix) && layer.ends_with(".CrtYd"))
    })
}

/// The side a footprint sits on and where its courtyards are.
pub(super) struct FootprintSides {
    pub back: bool,
    /// The layer prefixes of its own side and the other ("F." or "B.").
    pub own_prefix: &'static str,
    pub other_prefix: &'static str,
    /// Whether it has a courtyard on its own side, and on the other.
    pub own_courtyard: bool,
    pub other_courtyard: bool,
}

impl FootprintSides {
    pub fn of(footprint: &Expr) -> FootprintSides {
        // A part without SMD pads whose only courtyard is on the other side
        // sits on that side (urchin's tenting puck: a front footprint with
        // its courtyard on B.CrtYd).
        let back = {
            let back = on_back(footprint);
            let (own, other) = if back { ("B.", "F.") } else { ("F.", "B.") };
            let smd = footprint.children().iter().any(|child| {
                child.head() == Some("pad")
                    && child.children().get(2).and_then(Expr::atom) == Some("smd")
            });
            back != (!smd && !courtyard_on(footprint, own) && courtyard_on(footprint, other))
        };
        let (own_prefix, other_prefix) = if back { ("B.", "F.") } else { ("F.", "B.") };
        FootprintSides {
            back,
            own_prefix,
            other_prefix,
            own_courtyard: courtyard_on(footprint, own_prefix),
            other_courtyard: courtyard_on(footprint, other_prefix),
        }
    }

    /// A courtyard on each side (a hot-swap switch, a reversible part).
    pub fn both_courtyards(&self) -> bool {
        self.own_courtyard && self.other_courtyard
    }

    /// The part's own side and the other.
    pub fn sides(&self) -> (core::Side, core::Side) {
        if self.back {
            (core::Side::Back, core::Side::Front)
        } else {
            (core::Side::Front, core::Side::Back)
        }
    }

    /// The copper and mask layers of the side the part is not on.
    pub fn other_layers(&self) -> (&'static str, &'static str) {
        if self.back {
            ("F.Cu", "F.Mask")
        } else {
            ("B.Cu", "B.Mask")
        }
    }
}

/// A footprint's body and the box it occupies on the other side
/// (`two_sided_body`).
struct SidedBody {
    body_center: [f64; 2],
    body_size: [f64; 2],
    far_courtyard: Option<[f64; 4]>,
}

/// The body of a footprint with a courtyard on each side: its own side's
/// courtyard (unless round), and the box of the other's, which it occupies
/// there. Takes the body `local_body` gave; returns the body and that box.
fn two_sided_body(
    footprint: &Expr,
    config: &KiCadBoardPlacerConfig,
    sides: &FootprintSides,
    mut body_center: [f64; 2],
    mut body_size: [f64; 2],
) -> Result<SidedBody, String> {
    // A footprint with a courtyard on each side (a hot-swap switch: the
    // socket's on the back, the switch's on the front; a reversible
    // part) takes its own side's courtyard as its body and the other's
    // as what it occupies there: KiCad checks courtyards side by side
    // (urchin's back switches claimed the front switch's square on the
    // back too, and found no room among the mounting holes and logos
    // the designer put between them).
    let other_prefix = sides.other_prefix;
    let mut far_courtyard = None;
    if sides.both_courtyards() {
        let (center, size, own_round, _) = body_on_side(footprint, ".CrtYd", Some(other_prefix))?;
        if !own_round {
            body_center = center;
            body_size = size;
        }
        far_courtyard = footprint
            .children()
            .iter()
            .filter(|child| {
                form_atom(child, "layer", 1).is_some_and(|layer| {
                    layer.starts_with(other_prefix) && layer.ends_with(".CrtYd")
                })
            })
            .map(graphic_box)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .reduce(|a, b| {
                [
                    a[0].min(b[0]),
                    a[1].min(b[1]),
                    a[2].max(b[2]),
                    a[3].max(b[3]),
                ]
            })
            // Where the project lets courtyards overlap, KiCad does not
            // check the other side's either (ErgoSNM's diodes sit in
            // the keys' socket outlines on the back).
            .filter(|_| config.tight_bodies != Some(true));
    }
    Ok(SidedBody {
        body_center,
        body_size,
        far_courtyard,
    })
}

/// A footprint's pads as the placer holds them (`lower_pads`).
struct FootprintPads {
    pins: Vec<core::Pin>,
    /// Whether a pad goes through the board or lies on the other side.
    through: bool,
    /// What the part occupies on the other side: boxes that keep bodies
    /// off, and boxes that keep copper off.
    far_side: Vec<[f64; 4]>,
    far_copper: Vec<[f64; 4]>,
    /// Every pad's box (with a non-plated hole's clearance ring).
    own_pads: Vec<[f64; 4]>,
    /// The non-plated holes with the clearance ring they keep from
    /// other copper (`keep_away`).
    hole_rings: Vec<[f64; 4]>,
    pin_centers: Vec<[f64; 2]>,
    /// Every pad by name (its first anchor), for constraints that point at
    /// a pin.
    named: BTreeMap<String, [f64; 2]>,
    /// Every pad with a net (number, net, centre, box), for the pair rows.
    pad_nets: Vec<PadNet>,
}

/// A pad's box and its hole's in the footprint's frame (`pad_boxes`).
pub(super) struct PadBoxes {
    /// The pad (or hole) with the clearance a hole keeps.
    pub pad: [f64; 4],
    /// The hole alone (no clearance ring), at the pad's anchor.
    pub hole: [f64; 4],
    /// The clearance a non-plated hole keeps beyond the copper clearance.
    pub keep_away: f64,
}

/// Lowers a footprint's pads: their boxes, what they occupy on the other
/// side, and a pin per pad on a net that joins parts, numbering new nets
/// in `net_ids`.
fn lower_pads(
    context: &LowerContext,
    footprint: &Expr,
    at: [f64; 3],
    sides: &FootprintSides,
    net_ids: &mut BTreeMap<String, usize>,
) -> Result<FootprintPads, String> {
    let config = context.config;
    let mut pads = FootprintPads {
        pins: Vec::new(),
        through: false,
        far_side: Vec::new(),
        far_copper: Vec::new(),
        own_pads: Vec::new(),
        hole_rings: Vec::new(),
        pin_centers: Vec::new(),
        named: BTreeMap::new(),
        pad_nets: Vec::new(),
    };
    // A part whose plated holes hold leads standing out of the board:
    // a through-hole part (KiCad 6 on says so; older boards say nothing
    // for one, and then a hole of a via's size holds no lead). A
    // mounting hole's or an SMD part's plated holes hold none.
    let attributes: Vec<&str> = footprint
        .child("attr")
        .map(|attr| {
            attr.children()
                .iter()
                .skip(1)
                .filter_map(Expr::atom)
                .collect()
        })
        .unwrap_or_default();
    let leaded_part = attributes.contains(&"through_hole");
    let unattributed = footprint.child("attr").is_none();
    let holes_error = config.holes_in_courtyards_error == Some(true);
    // The copper layer of the side the part is not on: an edge-mount
    // connector's SMD pads there occupy that side too.
    let (other_copper, other_mask) = sides.other_layers();
    for pad in footprint
        .children()
        .iter()
        .filter(|child| child.head() == Some("pad"))
    {
        if let (Some(name), Ok(at)) = (pad.children().get(1).and_then(Expr::atom), form_at(pad)) {
            pads.named.entry(name.to_string()).or_insert([at[0], at[1]]);
        }
        let pad_type = pad.children().get(2).and_then(Expr::atom).unwrap_or("");
        let pad_at = form_at(pad)?;
        let boxes = pad_boxes(config, pad, footprint, pad_type, pad_at, at)?;
        let pad_box = boxes.pad;
        pads.own_pads.push(pad_box);
        if boxes.keep_away > 0.0 {
            pads.hole_rings.push(pad_box);
        }
        pads.pin_centers.push([pad_at[0], pad_at[1]]);
        let on_other_side = pad.child("layers").is_some_and(|layers| {
            layers
                .children()
                .iter()
                .skip(1)
                .filter_map(Expr::atom)
                .any(|layer| layer == other_copper || layer == "F&B.Cu")
        });
        if matches!(pad_type, "thru_hole" | "np_thru_hole") || on_other_side {
            pads.through = true;
            // What the part occupies on the other side. A lead through
            // the board (a plated hole of a part that is not SMD) keeps
            // off bodies there; a non-plated hole, an SMD part's vias
            // and its pads on that side only keep the copper clearance
            // from copper, unless the project forbids holes inside
            // courtyards (then the hole itself keeps off them too).
            let drill = pad
                .child("drill")
                .and_then(|drill| {
                    drill
                        .children()
                        .iter()
                        .skip(1)
                        .find_map(|item| item.atom()?.parse::<f64>().ok())
                })
                .unwrap_or(0.0);
            let lead = pad_type == "thru_hole" && (leaded_part || (unattributed && drill >= 0.6));
            if lead {
                pads.far_side.push(pad_box);
            } else {
                pads.far_copper.push(pad_box);
                if pad_type != "smd" && holes_error {
                    pads.far_side.push(boxes.hole);
                }
            }
            // Its mask opening there, where it reaches past the copper
            // clearance: other parts' pads on that side keep it the
            // web plus their expansion away (vaio_re's J1, a Pi Zero
            // socket on the front: its 2.3 mm mounting holes open the
            // mask 1.625 mm around on both sides, and a TSOT-23 and an
            // 0805 on the back under it bridged five times).
            let reach = far_mask_reach(
                pad,
                footprint,
                other_mask,
                context.board_mask_expansion,
                context.mask_web,
            ) - context.copper_clearance;
            if reach > boxes.keep_away + 1.0e-9 {
                let grow = reach - boxes.keep_away;
                pads.far_copper.push([
                    pad_box[0] - grow,
                    pad_box[1] - grow,
                    pad_box[2] + grow,
                    pad_box[3] + grow,
                ]);
            }
        }
        let Some(net) = node_net(pad)
            .filter(|raw| placer_net(raw))
            .map(|raw| context.net_names.normalize(raw))
        else {
            continue;
        };
        let next = net_ids.len();
        let pad_at = form_at(pad)?;
        if let Some(name) = pad.children().get(1).and_then(Expr::atom) {
            pads.pad_nets.push((
                name.to_string(),
                net.to_string(),
                [pad_at[0], pad_at[1]],
                pad_box,
            ));
        }
        let net = *net_ids.entry(net.to_string()).or_insert(next);
        pads.pins.push(core::Pin {
            offset: [pad_at[0], pad_at[1]],
            net,
        });
    }
    Ok(pads)
}

/// A pad's box in its footprint's frame (`pad_at` the pad's anchor and
/// angle, `at` the footprint's): its copper, moved by a drill offset and
/// grown by its custom primitives, with the clearance a non-plated hole
/// keeps; and its hole's box.
pub(super) fn pad_boxes(
    config: &KiCadBoardPlacerConfig,
    pad: &Expr,
    footprint: &Expr,
    pad_type: &str,
    pad_at: [f64; 3],
    at: [f64; 3],
) -> Result<PadBoxes, String> {
    // The pad (or hole) with the clearance a hole keeps, in the
    // footprint's frame.
    let size = pad_extent(pad);
    let (sin, cos) = quarter_sin_cos(-(pad_at[2] - at[2]));
    // A non-plated hole keeps its own clearance, and the board's
    // hole clearance, from other copper: the clearance tests add
    // the copper clearance to the box, so the box takes the rest.
    let keep_away = if pad_type == "np_thru_hole" {
        local_clearance::pad_clearance(pad, footprint)?
            .max(
                config.hole_clearance_mm.unwrap_or(0.0) - config.copper_clearance_mm.unwrap_or(0.2),
            )
            .max(0.0)
    } else {
        0.0
    };
    let half = [
        (cos.abs() * size[0] + sin.abs() * size[1]) / 2.0 + keep_away,
        (sin.abs() * size[0] + cos.abs() * size[1]) / 2.0 + keep_away,
    ];
    // A drill offset moves the copper away from the hole (castellated
    // module pads): the box covers both.
    let offset = pad
        .child("drill")
        .and_then(|drill| drill.child("offset"))
        .and_then(|offset| {
            Some([
                expression_coordinate(offset, 1, "pad offset x").ok()?,
                expression_coordinate(offset, 2, "pad offset y").ok()?,
            ])
        })
        .unwrap_or([0.0, 0.0]);
    let copper = [
        pad_at[0] + cos * offset[0] - sin * offset[1],
        pad_at[1] + sin * offset[0] + cos * offset[1],
    ];
    let mut pad_box = [
        (copper[0] - half[0]).min(pad_at[0] - half[0].min(half[1])),
        (copper[1] - half[1]).min(pad_at[1] - half[0].min(half[1])),
        (copper[0] + half[0]).max(pad_at[0] + half[0].min(half[1])),
        (copper[1] + half[1]).max(pad_at[1] + half[0].min(half[1])),
    ];
    // A custom pad's copper is its primitives too (OpenFC's test
    // pads: a 1 mm anchor and a 2.2 x 1.6 polygon).
    for (point, radius) in custom_pad_points(pad) {
        let at = [
            pad_at[0] + cos * point[0] - sin * point[1],
            pad_at[1] + sin * point[0] + cos * point[1],
        ];
        pad_box = [
            pad_box[0].min(at[0] - radius),
            pad_box[1].min(at[1] - radius),
            pad_box[2].max(at[0] + radius),
            pad_box[3].max(at[1] + radius),
        ];
    }
    // The hole alone (no clearance ring), at the pad's anchor.
    let hole_half = (half[0] - keep_away).min(half[1] - keep_away).max(0.0);
    let hole_box = [
        pad_at[0] - hole_half,
        pad_at[1] - hole_half,
        pad_at[0] + hole_half,
        pad_at[1] + hole_half,
    ];
    Ok(PadBoxes {
        pad: pad_box,
        hole: hole_box,
        keep_away,
    })
}

/// A footprint's copper and mask graphics on the outer layers
/// (`outer_graphics`).
pub(super) struct OuterGraphics {
    /// On the part's own side, or on an outer layer of either side that
    /// is not the other side's (`F.Cu`, `B.Cu`, `F.Mask`, `B.Mask`).
    pub own: Vec<[f64; 4]>,
    /// The mask openings among them, grown to KiCad's mask margin beyond
    /// the bodies' spacing.
    pub own_mask: Vec<[f64; 4]>,
    /// On the copper or mask layer of the side the part is not on.
    pub far: Vec<[f64; 4]>,
}

/// The boxes of a footprint's copper and mask graphics on the outer
/// layers, by side.
fn outer_graphics(
    context: &LowerContext,
    footprint: &Expr,
    sides: &FootprintSides,
) -> Result<OuterGraphics, String> {
    // Copper and mask graphics on the outer layers of the side the part
    // is not on occupy that side as well: Castor's LOGO footprint sits
    // on the front with its copper polygon on B.Cu, and a part placed
    // on the back over it was shorted and bridged.
    let (other_copper, other_mask) = sides.other_layers();
    let mask_growth = context.mask_growth;
    let mut graphics = OuterGraphics {
        own: Vec::new(),
        own_mask: Vec::new(),
        far: Vec::new(),
    };
    for child in footprint.children() {
        let Some(layer) = form_atom(child, "layer", 1) else {
            continue;
        };
        if !layer.ends_with(".Cu") && !layer.ends_with(".Mask") {
            continue;
        }
        let Some(graphic) = graphic_box(child)? else {
            continue;
        };
        if layer == other_copper || layer == other_mask {
            graphics.far.push(graphic);
        } else if matches!(layer, "F.Cu" | "B.Cu" | "F.Mask" | "B.Mask") {
            graphics.own.push(graphic);
            if layer.ends_with(".Mask") {
                graphics.own_mask.push([
                    graphic[0] - mask_growth,
                    graphic[1] - mask_growth,
                    graphic[2] + mask_growth,
                    graphic[3] + mask_growth,
                ]);
            }
        }
    }
    Ok(graphics)
}

/// The angles a part may take from its input angle `angle`: that one alone
/// with `keep_rotation`, else its four quarter turns (and the board's
/// four where it sits at an odd angle).
pub(super) fn angle_options(config: &KiCadBoardPlacerConfig, angle: f64) -> Vec<f64> {
    if config.tuning.keep_rotation {
        vec![angle]
    } else {
        let mut angles: Vec<f64> = (0..4)
            .map(|quarter| normalize_angle(angle + 90.0 * quarter as f64))
            .collect();
        // A part at an odd angle (a thumb cluster's keys at -30) may
        // also turn square to the board: the placer's bodies are boxes
        // around the turned part, half as large again at 30 degrees
        // (ErgoSNM: two keys found no room).
        if normalize_angle(angle) % 90.0 > 1.0e-6 {
            angles.extend([0.0, 90.0, 180.0, 270.0]);
        }
        // In a fixed order, not from the board's angle: the searches
        // walk them in this order (`core::orient`).
        if config.tuning.orientation_search {
            core::orient::sort_angles(&mut angles);
        }
        angles
    }
}

/// The footprint's copper graphics on any copper layer (KiCad 9's
/// `(layers "B.Cu" "B.Mask")` too): copper the edge and cutouts keep
/// their clearance from (`Component::copper_graphics`).
fn copper_graphic_boxes(footprint: &Expr) -> Result<Vec<[f64; 4]>, String> {
    let mut copper_graphics = Vec::new();
    for child in footprint.children() {
        let on_copper = form_atom(child, "layer", 1).is_some_and(|layer| layer.ends_with(".Cu"))
            || child.child("layers").is_some_and(|layers| {
                layers
                    .children()
                    .iter()
                    .skip(1)
                    .filter_map(Expr::atom)
                    .any(|layer| layer.trim_matches('"').ends_with(".Cu"))
            });
        if on_copper && let Some(graphic) = graphic_box(child)? {
            copper_graphics.push(graphic);
        }
    }
    Ok(copper_graphics)
}

/// The box around `boxes`, `[min x, min y, max x, max y]`; infinite and
/// inverted for none.
fn bounds_of(boxes: &[[f64; 4]]) -> [f64; 4] {
    boxes.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |bounds, pad| {
            [
                bounds[0].min(pad[0]),
                bounds[1].min(pad[1]),
                bounds[2].max(pad[2]),
                bounds[3].max(pad[3]),
            ]
        },
    )
}

/// How far the copper (`own_pads` and the footprint's copper graphics,
/// `copper_graphic_boxes`) stays inside the body: the body may come that
/// much closer to the board edge. Zero for a footprint without pads.
fn edge_inset(
    own_pads: &[[f64; 4]],
    copper_graphics: &[[f64; 4]],
    body_center: [f64; 2],
    body_size: [f64; 2],
) -> f64 {
    if own_pads.is_empty() {
        return 0.0;
    }
    // Copper graphics of the footprint count as copper too, on one copper
    // layer or several (KiCad 9's `(layers "F.Cu" "F.Mask")`).
    let pads = bounds_of(&[bounds_of(own_pads), bounds_of(copper_graphics)]);
    let body = [
        body_center[0] - body_size[0] / 2.0,
        body_center[1] - body_size[1] / 2.0,
        body_center[0] + body_size[0] / 2.0,
        body_center[1] + body_size[1] / 2.0,
    ];
    [
        pads[0] - body[0],
        pads[1] - body[1],
        body[2] - pads[2],
        body[3] - pads[3],
    ]
    .into_iter()
    .fold(f64::INFINITY, f64::min)
    .max(0.0)
}

/// What `body_shape` reads of a lowered footprint.
struct BodyParts<'a> {
    body_center: [f64; 2],
    body_size: [f64; 2],
    round: bool,
    artwork: bool,
    own_pads: &'a [[f64; 4]],
    pin_centers: &'a [[f64; 2]],
    graphics: &'a OuterGraphics,
}

/// How a footprint blocks beyond its body's box (`body_shape`).
struct BodyShape {
    /// Its courtyard's pieces (`Component::courtyards`).
    courtyards: Vec<[f64; 4]>,
    /// Whether holes may lie inside its courtyard.
    holes_inside: bool,
    /// The boxes it blocks with in place of its body's (`Component::hollow`).
    hollow: Vec<[f64; 4]>,
    /// Without a courtyard, the body a courtyard would give it.
    courtless: Option<[f64; 4]>,
}

/// A footprint's courtyard pieces, the shape it blocks with where its
/// courtyard is far from rectangular or missing, and the body a missing
/// courtyard would give it.
fn body_shape(
    context: &LowerContext,
    footprint: &Expr,
    sides: &FootprintSides,
    parts: BodyParts,
) -> Result<BodyShape, String> {
    let both_courtyards = sides.both_courtyards();
    let mut courtyard =
        courtyard_shapes_on_side(footprint, both_courtyards.then_some(sides.other_prefix))?;
    // A footprint without any courtyard blocks with its copper (pads,
    // copper and mask graphics) alone, and holes may lie inside it:
    // KiCad checks its copper, not a box around it (hackclub
    // keyboar_'s Kailh sockets have no courtyard; the designer put
    // every switch's diode between its pads).
    let courtless = !sides.own_courtyard
        && !sides.other_courtyard
        && !parts.own_pads.is_empty()
        && !parts.artwork;
    // Unless its pads stand in rows of pins: then a body fills their
    // box (`solid_pin_rows`), and the part blocks with that box.
    let solid =
        courtless && context.config.tuning.solid_pin_rows && pads_in_pin_rows(parts.pin_centers);
    let hollow = hollow_shape(
        context,
        footprint,
        sides,
        &parts,
        courtless,
        solid,
        &mut courtyard.0,
    )?;
    let courtless_body = if courtless {
        courtless_body(footprint, sides, &parts)?
    } else {
        None
    };
    Ok(BodyShape {
        courtyards: courtyard.0,
        holes_inside: courtyard.1 || (courtless && !solid),
        hollow,
        courtless: courtless_body,
    })
}

/// The boxes a footprint blocks with in place of its body's box (none
/// where the box is its shape): a solid part's pad box and graphics, a
/// courtless part's pads and graphics, or a courtyard far from
/// rectangular as strips with the pads (then also its strips as its
/// courtyard's pieces, in `courtyards`, where it had none).
fn hollow_shape(
    context: &LowerContext,
    footprint: &Expr,
    sides: &FootprintSides,
    parts: &BodyParts,
    courtless: bool,
    solid: bool,
    courtyards: &mut Vec<[f64; 4]>,
) -> Result<Vec<[f64; 4]>, String> {
    let (body_center, body_size) = (parts.body_center, parts.body_size);
    let (own_pads, own_graphic_boxes) = (parts.own_pads, &parts.graphics.own);
    let mask_growth = context.mask_growth;
    // A courtyard far from rectangular (a hot-swap socket's outline)
    // blocks with its own shape, as strips, and the pads: ghoul's
    // designer put resistor networks, regulators and mounting holes in
    // the corners of the switches' boxes, outside their courtyards.
    let mut shaped = Vec::new();
    if solid {
        // The body is that box already, unless copper or mask graphics
        // reach past it.
        if !own_graphic_boxes.is_empty() {
            let pads = bounds_of(own_pads);
            shaped = vec![pads];
            shaped.extend(own_graphic_boxes.iter().copied());
        }
    } else if courtless {
        shaped = own_pads.to_vec();
        shaped.extend(own_graphic_boxes.iter().copied());
    } else if !parts.round && !parts.artwork {
        let slabs = courtyard_slabs(
            footprint,
            sides.both_courtyards().then_some(sides.other_prefix),
        )?;
        let area: f64 = slabs.iter().map(|b| (b[2] - b[0]) * (b[3] - b[1])).sum();
        if !slabs.is_empty() && area < 0.75 * body_size[0] * body_size[1] {
            if slabs.len() > 1 && courtyards.is_empty() {
                *courtyards = slabs.clone();
            }
            shaped = slabs;
            shaped.extend(own_pads.iter().copied());
            // Its mask openings beside the courtyard block too: they
            // are in the body's box, which this shape stands in for,
            // and another part's pads there bridge the mask
            // (`mask_openings`). Held to the body's box (grown as they
            // are): a stroke's half width past it would widen every
            // neighbour search (`far_reach`).
            if context.config.tuning.mask_openings {
                let body = [
                    body_center[0] - body_size[0] / 2.0 - mask_growth,
                    body_center[1] - body_size[1] / 2.0 - mask_growth,
                    body_center[0] + body_size[0] / 2.0 + mask_growth,
                    body_center[1] + body_size[1] / 2.0 + mask_growth,
                ];
                shaped.extend(parts.graphics.own_mask.iter().map(|opening| {
                    [
                        opening[0].max(body[0]),
                        opening[1].max(body[1]),
                        opening[2].min(body[2]),
                        opening[3].min(body[3]),
                    ]
                }));
            }
        }
    }
    Ok(shaped)
}

/// Without a courtyard, the body a courtyard would give the part: its
/// pads, copper and mask graphics and its fabrication outline (else its
/// silkscreen outline) on its side.
fn courtless_body(
    footprint: &Expr,
    sides: &FootprintSides,
    parts: &BodyParts,
) -> Result<Option<[f64; 4]>, String> {
    // KiCad's courtyard clearance is 0 (no board of the benchmark sets a
    // `courtyard_clearance` rule). Two such parts may not overlap them
    // (`Component::courtless`): Goga64 ULK's PG1316 switches have no
    // courtyard, and a thumb row lay on the grid's last row with its
    // pads between theirs.
    let outline_boxes = |suffix: &str| -> Result<Vec<[f64; 4]>, String> {
        let mut boxes = Vec::new();
        for child in footprint.children() {
            if form_atom(child, "layer", 1)
                .is_some_and(|layer| layer.starts_with(sides.own_prefix) && layer.ends_with(suffix))
                && let Some(graphic) = graphic_box(child)?
            {
                boxes.push(graphic);
            }
        }
        Ok(boxes)
    };
    let mut outline = outline_boxes(".Fab")?;
    if outline.is_empty() {
        outline = outline_boxes(".SilkS")?;
    }
    Ok(parts
        .own_pads
        .iter()
        .chain(&parts.graphics.own)
        .chain(&outline)
        .copied()
        .reduce(|a, b| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]
        }))
}

/// The footprint's own closed Edge.Cuts loops (a switch's peg holes):
/// holes that move with it (`Component::slots`).
fn footprint_slots(footprint: &Expr) -> Result<Vec<[f64; 4]>, String> {
    let mut slots = Vec::new();
    for child in footprint.children() {
        if matches!(child.head(), Some("fp_circle" | "fp_rect" | "fp_poly"))
            && form_atom(child, "layer", 1) == Some("Edge.Cuts")
            && let Some(graphic) = graphic_box(child)?
        {
            slots.push(graphic);
        }
    }
    Ok(slots)
}

/// A part's halo (room kept around it for its tracks): the config's
/// override for `reference`, else a track pitch per `pins_per_halo_track`
/// of its `pins`, at most `maximum_halo_mm`, scaled.
pub(super) fn halo(config: &KiCadBoardPlacerConfig, reference: &str, pins: usize) -> f64 {
    if let Some(halo) = config.tuning.halo_overrides_mm.get(reference) {
        *halo
    } else if config.tuning.pins_per_halo_track > 0.0 {
        ((pins as f64 / config.tuning.pins_per_halo_track).ceil() * config.tuning.track_pitch_mm)
            .min(config.tuning.maximum_halo_mm)
            * config.tuning.halo_scale
    } else {
        0.0
    }
}

/// How far a pad's mask opening on the side `mask` (the side its part is
/// not on) keeps other copper's openings there, from the pad's copper:
/// its own expansion (its `solder_mask_margin`, else its footprint's)
/// plus the other opening's (the board's `pad_to_mask_clearance`) plus
/// the minimum web (KiCad 10.0.7's solder_mask_bridge: two openings
/// closer than the web bridge their items). Zero for a pad that does not
/// open the mask there, and for one without a margin of its own: the
/// board's expansion and web alone (`pad_mask_gap`, which parts on one
/// side keep) brought no far-side bridge in the 2026-10-27 sweep, and
/// would move every through-hole part on 20-odd boards.
pub(super) fn far_mask_reach(
    pad: &Expr,
    footprint: &Expr,
    mask: &str,
    board_expansion: f64,
    web: f64,
) -> f64 {
    let opens = pad.child("layers").is_some_and(|layers| {
        layers
            .children()
            .iter()
            .skip(1)
            .filter_map(Expr::atom)
            .any(|layer| layer == mask || layer == "*.Mask" || layer == "F&B.Mask")
    });
    if !opens {
        return 0.0;
    }
    let Some(expansion) = form_f64(pad, "solder_mask_margin", 1)
        .ok()
        .or_else(|| form_f64(footprint, "solder_mask_margin", 1).ok())
    else {
        return 0.0;
    };
    (expansion + board_expansion + web).max(0.0)
}
