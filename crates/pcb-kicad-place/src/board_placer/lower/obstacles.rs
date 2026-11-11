// Copyright (C) 2026 Toit contributors.

//! The board's fixed obstacles as parts of the placer's problem: rule
//! areas, copper and mask graphics, texts and silkscreen marks, cutouts,
//! and the rule areas footprints bring.

use super::*;
use pcb_placer as core;

/// A fixed obstacle the input placed (`name`, a box of `body_size` on
/// `side`): no pins, pads or shapes of a part.
pub(super) fn fixed_obstacle(name: &str, body_size: [f64; 2], side: core::Side) -> core::Component {
    core::Component {
        name: name.into(),
        body_center: [0.0, 0.0],
        body_size,
        round: false,
        halo: 0.0,
        pins: Vec::new(),
        side,
        fixed: true,
        angle_options: vec![0.0],
        far_side: Vec::new(),
        far_copper: Vec::new(),
        far_open: Default::default(),
        far_courtyard: Vec::new(),
        hollow: Vec::new(),
        tight: None,
        edge_inset: 0.0,
        courtyards: Vec::new(),
        holes_inside: false,
        pads: Vec::new(),
        copper_only: false,
        cutout_outline: Vec::new(),
        tight_hollow: Vec::new(),
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
        courtless: Vec::new(),
        slots: Vec::new(),
    }
}

/// A rule area as a fixed obstacle for the placer, with its bounds and
/// whether it keeps bodies out: one that forbids footprints blocks bodies
/// on the sides of its layers; one that allows footprints but forbids pads
/// keeps copper out only, as a cutout does (bodies may reach over it).
/// `None` for other zones.
fn rule_area_obstacle(
    item: &Expr,
) -> Result<
    Option<(
        core::Component,
        core::Pose,
        pcb_router::geometry::Aabb,
        bool,
    )>,
    String,
> {
    let Some(keepout) = item.child("keepout") else {
        return Ok(None);
    };
    // A constraint keepout keeps bodies out whatever its rule area allows:
    // one meeting a fixed part's courtyard allows footprints so that KiCad
    // does not report that part (`apply_keepouts`).
    let bodies = form_atom(keepout, "footprints", 1) == Some("not_allowed")
        || form_atom(item, "name", 1).is_some_and(|name| name.starts_with(KEEPOUT_NAME));
    let pads = form_atom(keepout, "pads", 1) == Some("not_allowed");
    if !bodies && !pads {
        return Ok(None);
    }
    let Some(polygon) = item
        .child("polygon")
        .and_then(|polygon| polygon.child("pts"))
    else {
        return Ok(None);
    };
    let mut points = Vec::new();
    for point in polygon
        .children()
        .iter()
        .filter(|child| child.head() == Some("xy"))
    {
        let coordinate = |index: usize| -> Result<f64, String> {
            point
                .children()
                .get(index)
                .and_then(Expr::atom)
                .ok_or_else(|| "keepout point without coordinates".to_string())?
                .parse::<f64>()
                .map_err(|error| format!("invalid keepout coordinate: {error}"))
        };
        points.push([coordinate(1)?, coordinate(2)?]);
    }
    let Some(bounds) = points
        .iter()
        .map(|xy| pcb_router::geometry::Aabb {
            minimum: *xy,
            maximum: *xy,
        })
        .reduce(pcb_router::geometry::Aabb::union)
    else {
        return Ok(None);
    };
    let layers: Vec<&str> = item
        .child("layers")
        .map(|layers| {
            layers
                .children()
                .iter()
                .skip(1)
                .filter_map(Expr::atom)
                .collect()
        })
        .or_else(|| form_atom(item, "layer", 1).map(|layer| vec![layer]))
        .unwrap_or_default();
    let all = layers
        .iter()
        .any(|layer| layer.starts_with('*') || *layer == "F&B.Cu");
    let side = match (
        all || layers.contains(&"F.Cu"),
        all || layers.contains(&"B.Cu"),
    ) {
        (true, false) => core::Side::Front,
        (false, true) => core::Side::Back,
        // Inner layers only: no part sits there, but a through-hole pad
        // crosses them (hackrf's keepouts under its SMA pins on In1/In2:
        // KiCad found the moved connectors' ground pins in them). Kept out
        // as copper on both sides, an SMD pad too (it would not need to be).
        (false, false) if !layers.is_empty() && (bodies || !pads) => return Ok(None),
        (false, false) if !layers.is_empty() => core::Side::Both,
        _ => core::Side::Both,
    };
    let component = core::Component {
        copper_only: !bodies,
        cutout_outline: if bodies { Vec::new() } else { points },
        ..fixed_obstacle(
            "keepout",
            [
                bounds.maximum[0] - bounds.minimum[0],
                bounds.maximum[1] - bounds.minimum[1],
            ],
            side,
        )
    };
    let pose = core::Pose {
        position: [
            (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
            (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
        ],
        angle: 0.0,
    };
    Ok(Some((component, pose, bounds, bodies)))
}

/// Adds the board's rule areas that keep bodies or pads out as fixed
/// obstacles; returns the bounds of those that keep bodies out (but not
/// the constraints' own keepouts), which fix the parts reaching into them.
pub(super) fn add_board_rule_areas(
    context: &LowerContext,
    builder: &mut ProblemBuilder,
) -> Result<Vec<pcb_router::geometry::Aabb>, String> {
    // Rule areas that forbid footprints are obstacles too, and a part the
    // designer placed reaching into one (an antenna module at its keepout)
    // is there on purpose and stays. One that allows footprints but forbids
    // pads keeps copper out only (d20's back: KiCad flagged 27 of our pads
    // in it).
    let mut keepouts: Vec<pcb_router::geometry::Aabb> = Vec::new();
    for item in context
        .pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone"))
    {
        let Some((component, pose, bounds, bodies)) = rule_area_obstacle(item)? else {
            continue;
        };
        // A part inside a constraint keepout is not there on purpose.
        if bodies && !form_atom(item, "name", 1).is_some_and(|name| name.starts_with(KEEPOUT_NAME))
        {
            keepouts.push(bounds);
        }
        builder.push_obstacle(component, pose, "keepout");
    }
    Ok(keepouts)
}

/// The sides a board graphic's layers cover: 0 for the front's copper or
/// mask, 1 for the back's; sorted, each once.
pub(super) fn outer_copper(item: &Expr) -> Vec<usize> {
    let names: Vec<&str> = match form_atom(item, "layer", 1) {
        Some(name) => vec![name],
        None => item
            .child("layers")
            .map(Expr::children)
            .unwrap_or_default()
            .iter()
            .skip(1)
            .filter_map(Expr::atom)
            .collect(),
    };
    // A mask opening too: a part's pads under it would bridge (Sisu's
    // "REV B2" on B.Mask got two parts' pads in layout).
    let mut sides: Vec<usize> = names
        .into_iter()
        .filter_map(|name| match name {
            "F.Mask" => Some(0),
            "B.Mask" => Some(1),
            name => copper_layer_index(name),
        })
        .collect();
    sides.sort_unstable();
    sides.dedup();
    sides
}

/// Adds the board's copper and mask graphics (and texts on a mask layer)
/// as fixed obstacles, one per side they cover.
pub(super) fn add_copper_graphics(
    context: &LowerContext,
    builder: &mut ProblemBuilder,
) -> Result<(), String> {
    // Copper-layer graphics are part of the board: parts must not be
    // placed on top of them. They follow the footprints, so footprint
    // indices stay aligned with the board file. (Copper and silkscreen
    // texts move off the parts and the copper after placement instead:
    // see `labels`.)
    // KiCad 9 draws a graphic on several layers at once (`layers`):
    // Sisu's GND rectangles on F.Cu and F.Mask.
    for (item, layer) in context.pcb.children().iter().flat_map(|item| {
        outer_copper(item)
            .into_iter()
            .map(move |layer| (item, layer))
    }) {
        let on_mask = matches!(form_atom(item, "layer", 1), Some("F.Mask" | "B.Mask"));
        if !matches!(
            item.head(),
            Some("gr_line" | "gr_rect" | "gr_arc" | "gr_circle" | "gr_poly")
        ) && !(on_mask && item.head() == Some("gr_text"))
        {
            continue;
        }
        let Some(bounds) = board_router::copper_graphic_shapes(item)?
            .iter()
            .map(pcb_router::Shape::aabb)
            .reduce(pcb_router::geometry::Aabb::union)
        else {
            continue;
        };
        let component = fixed_obstacle(
            "copper graphic",
            [
                bounds.maximum[0] - bounds.minimum[0],
                bounds.maximum[1] - bounds.minimum[1],
            ],
            if layer == 0 {
                core::Side::Front
            } else {
                core::Side::Back
            },
        );
        let pose = core::Pose {
            position: [
                (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
                (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
            ],
            angle: 0.0,
        };
        builder.push_obstacle(component, pose, "copper graphic");
    }
    Ok(())
}

/// Adds the board's own texts and silkscreen graphics the input placed
/// (`marks`, with `keep_board_marks`) as obstacles on their side; returns
/// each with its part's index.
pub(super) fn add_board_marks(
    context: &LowerContext,
    builder: &mut ProblemBuilder,
) -> Result<Vec<(usize, BoardMark)>, String> {
    // The board's own texts and silkscreen graphics the input placed
    // (`marks`): obstacles on their side, as a keepout there.
    let mut marks: Vec<(usize, BoardMark)> = Vec::new();
    if context.config.keep_board_marks {
        for mark in board_marks(context.pcb, outline_box(context.pcb))? {
            let (component, pose) = mark_obstacle(&mark);
            let reference = if mark.copper {
                "copper text"
            } else {
                "silkscreen mark"
            };
            marks.push((builder.problem.components.len(), mark));
            builder.push_obstacle(component, pose, reference);
        }
    }
    Ok(marks)
}

/// The cutouts' parts (`add_cutouts`).
pub(super) struct Cutouts {
    /// Those an `outline` constraint cut, which keep bodies off too.
    pub solid: Vec<usize>,
    /// Those that are a footprint's own closed graphic, with that
    /// footprint: they go with the part where it moves.
    pub owned: Vec<(usize, usize)>,
}

/// Adds the board's cutouts as fixed obstacles (copper keeps the edge
/// margin from them).
pub(super) fn add_cutouts(context: &LowerContext, builder: &mut ProblemBuilder) -> Cutouts {
    let loops = &context.loops;
    // Cutouts are board edge too: parts keep the edge margin from them.
    // Those an `outline` constraint cut (the board's own graphics, which
    // it replaced) keep bodies off as well.
    let outline_given = context
        .user_constraints
        .is_some_and(|constraints| constraints.outline.is_some());
    let mut cutouts = Cutouts {
        solid: Vec::new(),
        owned: Vec::new(),
    };
    for (number, cutout) in loops.cutouts.iter().enumerate() {
        let (mut minimum, mut maximum) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for point in cutout {
            for axis in 0..2 {
                minimum[axis] = minimum[axis].min(point[axis]);
                maximum[axis] = maximum[axis].max(point[axis]);
            }
        }
        if minimum[0] > maximum[0] {
            continue;
        }
        let index = builder.problem.components.len();
        if outline_given && loops.board_cutouts.get(number).copied().unwrap_or(false) {
            cutouts.solid.push(index);
        }
        if let Some(Some(owner)) = loops.cutout_owners.get(number) {
            cutouts.owned.push((index, *owner));
        }
        // The hole itself: the copper keeps the edge margin from it in the
        // legality test, which looser levels bring down to the rules'
        // clearance. Grown here too, the margin counted twice and never
        // relaxed (katia: parts found no room beside its cutouts).
        let component = core::Component {
            copper_only: true,
            cutout_outline: cutout.clone(),
            ..fixed_obstacle(
                "cutout",
                [maximum[0] - minimum[0], maximum[1] - minimum[1]],
                core::Side::Both,
            )
        };
        let pose = core::Pose {
            position: [
                (minimum[0] + maximum[0]) / 2.0,
                (minimum[1] + maximum[1]) / 2.0,
            ],
            angle: 0.0,
        };
        builder.push_obstacle(component, pose, "cutout");
    }
    cutouts
}

/// Adds a fixed footprint's own rule areas as obstacles, and a movable
/// one's to its far side (bodies) or far copper (pads only) where they
/// keep the other side.
pub(super) fn add_footprint_rule_areas(
    context: &LowerContext,
    builder: &mut ProblemBuilder,
) -> Result<(), String> {
    // A fixed footprint's own rule areas (an antenna module's keepout, in
    // board coordinates) are obstacles like the board's: PixelWave's
    // ESP32 on the back keeps parts out of its antenna area on both sides,
    // and KiCad flagged 50 pads and parts we put there. A movable part's
    // keepout moves with it: it blocks the other side as far-side box, or,
    // where it forbids pads but not footprints, as far-side copper
    // (MokyaLora's U.FL on the back keeps pads out on the front).
    for (index, footprint) in context.footprints().enumerate() {
        for zone in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("zone"))
        {
            let Some((component, pose, _, bodies)) = rule_area_obstacle(zone)? else {
                continue;
            };
            if builder.problem.components[index].fixed {
                // Locked like every obstacle, so that the per-part lists
                // keep the parts' length (the constraints read the locks
                // before).
                builder.push_obstacle(component, pose, "keepout");
                continue;
            }
            let problem = &mut builder.problem;
            if component
                .side
                .collides(match problem.components[index].side {
                    core::Side::Front => core::Side::Back,
                    core::Side::Back => core::Side::Front,
                    _ => core::Side::Neither,
                })
            {
                let local = local_box(&component, pose, builder.source_at[index]);
                if bodies {
                    problem.components[index].far_side.push(local);
                } else {
                    problem.components[index].far_copper.push(local);
                }
            }
        }
    }
    Ok(())
}

/// The box of an obstacle (`component` at `pose`, in board coordinates)
/// in the frame of a footprint at `at`.
pub(super) fn local_box(component: &core::Component, pose: core::Pose, at: [f64; 3]) -> [f64; 4] {
    let half = [component.body_size[0] / 2.0, component.body_size[1] / 2.0];
    let corners = [
        [-half[0], -half[1]],
        [half[0], -half[1]],
        [half[0], half[1]],
        [-half[0], half[1]],
    ];
    let local = corners.map(|corner| {
        core::problem::rotate(
            [
                pose.position[0] + corner[0] - at[0],
                pose.position[1] + corner[1] - at[1],
            ],
            at[2],
        )
    });
    local.iter().fold(
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
}
