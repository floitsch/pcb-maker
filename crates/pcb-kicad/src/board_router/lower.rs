// Copyright (C) 2026 Toit contributors.

//! Lowering a parsed KiCad board to the core router's board: layers,
//! nets, pads, obstacles, pours and rule areas.

use super::geometry::polygon_area;
use super::*;
use pcb_router as core;

pub(super) fn shape(geometry: &ObstacleGeometry) -> core::Shape {
    match geometry {
        ObstacleGeometry::Circle { center, radius } => core::Shape::Circle {
            center: *center,
            radius: *radius,
        },
        ObstacleGeometry::Segment { start, end, radius } => core::Shape::Capsule {
            start: *start,
            end: *end,
            radius: *radius,
        },
        ObstacleGeometry::Rectangle {
            center,
            half_size,
            angle_degrees,
        } => core::Shape::rectangle(*center, *half_size, *angle_degrees),
        ObstacleGeometry::Polygon { points } => core::Shape::Polygon {
            points: points.clone(),
        },
        ObstacleGeometry::Union { parts } => core::Shape::Union {
            parts: parts.iter().map(shape).collect(),
        },
    }
}

/// The board's copper layers in stack order: front, inner 1..n, back.
pub(crate) struct LayerTable {
    pub names: Vec<String>,
}

impl LayerTable {
    pub fn from_pcb(pcb: &Expr) -> Result<Self, String> {
        let mut names: Vec<String> = pcb
            .child("layers")
            .map(Expr::children)
            .unwrap_or_default()
            .iter()
            .filter_map(|entry| entry.children().get(1).and_then(Expr::atom))
            .filter(|name| name.ends_with(".Cu"))
            .map(str::to_owned)
            .collect();
        let rank = |name: &str| -> (u8, u32) {
            match name {
                "F.Cu" => (0, 0),
                "B.Cu" => (2, 0),
                _ => (
                    1,
                    name.trim_start_matches("In")
                        .trim_end_matches(".Cu")
                        .parse()
                        .unwrap_or(u32::MAX),
                ),
            }
        };
        names.sort_by_key(|name| rank(name));
        if names.first().map(String::as_str) != Some("F.Cu")
            || names.last().map(String::as_str) != Some("B.Cu")
            || names.len() > 32
        {
            return Err(format!(
                "unsupported copper layer stack {names:?}: expected F.Cu, inner layers, B.Cu"
            ));
        }
        Ok(Self { names })
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|layer| layer == name)
    }

    /// Every copper layer: up to 32, a bit each (`1 << 32` would wrap
    /// to none in a release build).
    pub fn all(&self) -> core::LayerMask {
        if self.len() >= 32 {
            core::LayerMask::MAX
        } else {
            (1 << self.len()) - 1
        }
    }

    /// Mask for a KiCad layer list such as `("F.Cu" "In1.Cu")`, `*.Cu`
    /// (all copper) or `F&B.Cu` (both outer layers).
    pub fn mask_of<'a>(
        &self,
        names: impl Iterator<Item = &'a str>,
    ) -> Result<core::LayerMask, String> {
        let mut mask = 0;
        for name in names {
            match name {
                "*.Cu" => mask |= self.all(),
                "F&B.Cu" => mask |= 1 | 1 << (self.len() - 1),
                // A copper layer the stackup does not have (a footprint
                // made for more layers) carries nothing, as in KiCad.
                name if name.ends_with(".Cu") => {
                    if let Some(index) = self.index(name) {
                        mask |= 1 << index;
                    }
                }
                _ => {}
            }
        }
        Ok(mask)
    }

    /// Mask of an item with either a `layer` or a `layers` form.
    pub fn mask_of_item(&self, item: &Expr) -> Result<core::LayerMask, String> {
        if let Some(layer) = form_atom(item, "layer", 1) {
            return self.mask_of(std::iter::once(layer));
        }
        let names: Vec<&str> = item
            .child("layers")
            .map(Expr::children)
            .unwrap_or_default()
            .iter()
            .skip(1)
            .filter_map(Expr::atom)
            .collect();
        self.mask_of(names.into_iter())
    }
}

/// The copper layers of a via: a through via's every layer, a blind,
/// buried or micro via's (`(via blind ... (layers "F.Cu" "In1.Cu"))`)
/// the layers from its first to its last. The router's own vias are
/// through vias; another via is copper on its own layers only.
fn via_layers(via: &Expr, layers: &LayerTable) -> core::LayerMask {
    let ends: Vec<usize> = via
        .child("layers")
        .map(Expr::children)
        .unwrap_or_default()
        .iter()
        .skip(1)
        .filter_map(Expr::atom)
        .filter_map(|name| layers.index(name.trim_matches('"')))
        .collect();
    match ends.as_slice() {
        [first, last] => {
            let (low, high) = ((*first).min(*last), (*first).max(*last));
            (low..=high).fold(0, |mask, layer| mask | 1 << layer)
        }
        _ => layers.all(),
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

pub(crate) struct Lowered {
    pub board: core::Board,
    /// The board file's expressions (`expression_count`): its lowering's
    /// work is `lowering_work_of` them.
    pub expressions: u64,
}

static LOWER_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

static LOWER_NANOS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The lowerings' modelled work in nanoseconds (`lowering_work`).
static LOWER_WORK_NANOS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A lowering's work in nanoseconds of the reference per expression of
/// the board file (an atom or a list: `expression_count`): measured on
/// the expansion tier's boards and the named boards, 162 lowerings at
/// one thread in each of two runs, the median 26 and 19 ns (10 % to
/// 90 %: 13 to 55 ns), 0.5 to 22 ms a lowering. Route mode's ladder
/// charges it with the attempt that lowers (docs/router.md, "All the
/// work charged").
pub(crate) const LOWERING_EXPRESSION_NANOS: f64 = 26.0;

/// The expressions (atoms and lists) of a parsed board file.
pub(crate) fn expression_count(pcb: &Expr) -> u64 {
    // A stack of the lists still to count, not recursion; atoms are
    // counted with their list.
    let (mut count, mut pending) = (1, vec![pcb]);
    while let Some(list) = pending.pop() {
        let items = list.children();
        count += items.len() as u64;
        pending.extend(items.iter().filter(|item| matches!(item, Expr::List(_))));
    }
    count
}

/// The work of one lowering of `pcb`, modelled, in seconds at this
/// process's rate (the reference's nanoseconds scaled as the pour
/// model's are).
pub(crate) fn lowering_work(pcb: &Expr) -> f64 {
    lowering_work_of(expression_count(pcb))
}

/// `lowering_work` of a board file of `expressions` expressions.
pub(crate) fn lowering_work_of(expressions: u64) -> f64 {
    expressions as f64 * LOWERING_EXPRESSION_NANOS / 1.0e9 * core::router::reference_time_scale()
}

/// The modelled work of every lowering of this process so far, seconds.
pub(crate) fn lowering_work_total() -> f64 {
    LOWER_WORK_NANOS.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1.0e9
}

/// Calls of `lower` in this process and their seconds (summed over
/// threads), for the board check's line.
pub(crate) fn lowering_totals() -> (u64, f64) {
    use std::sync::atomic::Ordering::Relaxed;
    (
        LOWER_CALLS.load(Relaxed),
        LOWER_NANOS.load(Relaxed) as f64 / 1.0e9,
    )
}

pub(crate) fn lower(
    pcb: &Expr,
    config: &KiCadBoardRouterConfig,
    connect_pours: bool,
) -> Result<Lowered, String> {
    use std::sync::atomic::Ordering::Relaxed;
    let started = std::time::Instant::now();
    let lowered = lower_board(pcb, config, connect_pours);
    LOWER_CALLS.fetch_add(1, Relaxed);
    LOWER_NANOS.fetch_add(started.elapsed().as_nanos() as u64, Relaxed);
    let expressions = expression_count(pcb);
    LOWER_WORK_NANOS.fetch_add((lowering_work_of(expressions) * 1.0e9) as u64, Relaxed);
    // What the lowering refuses is the board's fault (a form it cannot
    // read, a layer stack it cannot hold), as is a board the router's maps
    // cannot hold (its nets in 16 bits, its layers in 32): an input error,
    // not an internal one.
    let mut lowered = lowered.map_err(crate::verdict::input_error)?;
    lowered.expressions = expressions;
    if std::env::var_os("PCB_ROUTER_BUILD_TRACE").is_some() {
        let board = &lowered.board;
        eprintln!(
            "lowering trace: expressions {} obstacles {} nets {} terminals {} planes {} plane_vertices {} seconds {:.5}",
            expressions,
            board.obstacles.len(),
            board.nets.len(),
            board
                .nets
                .iter()
                .map(|net| net.terminals.len())
                .sum::<usize>(),
            board.planes.len(),
            board
                .planes
                .iter()
                .map(|plane| plane.polygon.len())
                .sum::<usize>(),
            started.elapsed().as_secs_f64()
        );
    }
    core::grid::check_capacity(&lowered.board).map_err(crate::verdict::input_error)?;
    // A lattice the memory cannot hold, before any map is allocated.
    check_lattice_fits(&lowered.board, config)?;
    Ok(lowered)
}

fn lower_board(
    pcb: &Expr,
    config: &KiCadBoardRouterConfig,
    connect_pours: bool,
) -> Result<Lowered, String> {
    // The board's net facts; noted on this thread too, for the writer and
    // the project rules that read them after (`note_net_names`).
    let net_names = crate::note_net_names(pcb);
    let board_version = form_atom(pcb, "version", 1).and_then(|version| version.parse().ok());
    // KiCad 9 and before name the net of a track, via or graphic by code.
    let net_codes = crate::net_codes(pcb);
    let loops = outline::board_loops(pcb)?;
    let layers = LayerTable::from_pcb(pcb)?;
    let mut classes: Vec<core::RuleClass> = Vec::new();
    let mut nets: Vec<core::Net> = Vec::new();
    let mut net_ids = BTreeMap::<String, core::NetId>::new();
    let mut obstacles = Vec::new();
    // Rule areas that keep zone fills out: (layers, polygon).
    let mut pour_keepouts: Vec<PourKeepout> = Vec::new();
    let mut solid_pads: Vec<String> = Vec::new();
    // Regions where a pour stays whole (`keep_whole`): their rule areas'
    // pour net, layers and polygon, lowered once the nets are known.
    let mut keep_whole: Vec<KeepWholeRegion> = Vec::new();
    // Rule areas the fill passes (`core::Board::fill_passes`).
    let mut fill_passes: Vec<usize> = Vec::new();
    // Openings in the solder mask (graphics on a mask layer), placed once
    // the pads under them are known.
    // The margin is how far the opening reaches past its shapes (a
    // mask-only pad's mask expansion); the last value is the aperture's
    // mask expansion as KiCad's bridge test takes it (a graphic's own
    // `solder_mask_margin`, else the board's `pad_to_mask_clearance`).
    let mut mask_openings: Vec<(Vec<core::Shape>, usize, String, f64, f64)> = Vec::new();
    // Board copper graphics, with the net they carry, placed once the nets
    // of the pads are known.
    let mut copper_graphics: Vec<CopperGraphic> = Vec::new();
    let mut isolated_pads: Vec<String> = Vec::new();
    // The same per pad, by obstacle index.
    let mut solid_pad_obstacles: Vec<usize> = Vec::new();
    // Per pad (by obstacle index), its footprint's jumper group: KiCad 9's
    // `(duplicate_pad_numbers_are_jumpers yes)` joins the pads of one
    // number, `(jumper_pad_groups ("1" "2") ...)` the pads of the numbers
    // of a group, without copper (kicad-cli 10.0.7: no unconnected item
    // between them).
    let mut jumper_groups: std::collections::HashMap<usize, (usize, String)> =
        std::collections::HashMap::new();
    let mut footprint_ordinal = 0usize;
    let mut isolated_pad_obstacles: Vec<usize> = Vec::new();
    // A padstack pad's copper on the layers its stack names, by the pad's
    // own obstacle: merged into the pad's terminal (`MergedPad::padstack`).
    let mut padstack_copper: Vec<(usize, usize)> = Vec::new();
    // SMD pads no via may overlap (`core::Board::no_via_pads`): every SMD
    // pad's copper but an exposed pad's, unless the board's vias are
    // filled and capped (`via_in_pad`).
    let mut no_via_pads: Vec<usize> = Vec::new();
    // The gaps between adjacent SMD pads of a footprint's pin rows
    // (`core::Board::pin_row_gaps`, `pin_row_gaps`).
    let mut pin_row_gaps: Vec<core::PinRowGap> = Vec::new();
    let mut pad_spokes: std::collections::HashMap<usize, core::PadSpokes> =
        std::collections::HashMap::new();
    // With `use_narrow_signals`, signal nets (not poured, not named as a
    // ground or a rail) route at the project's smallest predefined width.
    let poured: std::collections::HashSet<String> = if config.use_narrow_signals == Some(true) {
        pours(pcb, &layers)?
            .into_iter()
            .map(|pour| pour.net)
            .collect()
    } else {
        Default::default()
    };

    // Differential pairs (`diff_pairs`): a pair's nets keep off the layers
    // the custom rules disallow their tracks on (`disallow track`: ducktop2's
    // ground and impedance layers), and off those where a custom rule's
    // track width bounds do not hold the width they route at (ducktop2's
    // DIFF_90: 0.1796 mm outside, at most 0.111 on In2.Cu; a class has one
    // width), as the pair's coupled runs do; the stubs and the fallback
    // route them in their class.
    let pairs_on = config.diff_pairs_on() && config.engine.as_deref() != Some("topological");
    let pair_layers = |name: &str, width: f64| -> core::LayerMask {
        let Some(rule) = config.diff_pair_rules.get(name).filter(|_| pairs_on) else {
            return 0;
        };
        let count = rule.disallowed.len().min(32);
        let other_width = |layer: usize| {
            rule.width_bounds
                .get(layer)
                .is_some_and(|(min, max)| width < min - 1.0e-6 || width > max + 1.0e-6)
        };
        let allowed = (0..count)
            .filter(|layer| !rule.disallowed[*layer] && !other_width(*layer))
            .fold(0, |mask: core::LayerMask, layer| mask | (1 << layer));
        if allowed.count_ones() as usize == count {
            0
        } else {
            allowed
        }
    };
    let mut net_id = |name: &str,
                      nets: &mut Vec<core::Net>,
                      classes: &mut Vec<core::RuleClass>|
     -> Result<core::NetId, String> {
        if let Some(id) = net_ids.get(name) {
            return Ok(*id);
        }
        let rules = config
            .connection_rules
            .get(name)
            .or(config.default_rules.as_ref())
            .ok_or_else(|| format!("no routing rules for connection {name:?}"))?;
        let narrow = match (config.use_narrow_signals, config.narrow_signal_mm) {
            (Some(true), Some(width))
                if width < rules.trace_width_mm
                    && !poured.contains(name)
                    && !crate::quality::ground(name)
                    && !crate::quality::rail_name(name) =>
            {
                Some(width)
            }
            _ => None,
        };
        let class = core::RuleClass {
            trace_width: narrow
                .unwrap_or(rules.trace_width_mm)
                .max(config.width_floor()),
            clearance: rules.clearance_mm,
            via_diameter: rules.via_size_mm,
            via_drill: rules.via_drill_mm,
            layers: pair_layers(name, rules.trace_width_mm),
        };
        let class_index = classes
            .iter()
            .position(|existing| *existing == class)
            .unwrap_or_else(|| {
                classes.push(class);
                classes.len() - 1
            });
        let id = nets.len() as core::NetId;
        nets.push(core::Net {
            name: name.to_string(),
            class: class_index,
            terminals: Vec::new(),
        });
        net_ids.insert(name.to_string(), id);
        Ok(id)
    };

    // A pad's solder mask aperture (the pad grown by its mask expansion)
    // bares the copper around it: another net's copper near it bridges.
    // Only reaches past the copper clearance matter. The board's mask
    // expansion is `pad_to_mask_clearance` (KiCad's setup has no
    // `solder_mask_margin`; the rule below read that key until 2026-10-15,
    // so only pads and footprints with their own margin had an aperture).
    let board_pad_to_mask = pcb
        .child("setup")
        .and_then(|setup| form_f64(setup, "pad_to_mask_clearance", 1).ok())
        .unwrap_or(0.0);
    let min_mask_web = pcb
        .child("setup")
        .and_then(|setup| form_f64(setup, "solder_mask_min_width", 1).ok())
        .unwrap_or(0.0)
        .max(0.0);
    let pad_mask_reach = if min_mask_web > 0.0 {
        min_mask_web / 2.0 + 0.01
    } else {
        0.0
    };
    let mask_to_copper = config.solder_mask_to_copper_mm.unwrap_or(0.0).max(0.0);
    let tented = vias_tented(pcb);
    // KiCad's exact rule where its bridges are errors; the rule of before
    // where the designer rated them lower.
    let strict_mask = !matches!(
        config.solder_mask_bridge_severity.as_deref(),
        Some("warning" | "ignore")
    );
    let smallest_clearance = config
        .connection_rules
        .values()
        .chain(config.default_rules.iter())
        .map(|rules| rules.clearance_mm)
        .fold(f64::INFINITY, f64::min);
    // Copper of no net (a logo, a footprint's graphic) is in KiCad's
    // Default class: other copper keeps the larger of both clearances
    // (tuxkeychain's GRAPHIC3).
    let no_net_clearance = config
        .default_rules
        .as_ref()
        .map_or(0.0, |rules| rules.clearance_mm);
    for item in pcb.children() {
        match item.head() {
            Some("footprint") => {
                if let Some(why) = footprint_form_error(item) {
                    return Err(format!("the board has {why}"));
                }
                let footprint_at = form_at(item)?;
                let reference = footprint_reference(item).unwrap_or_default();
                let electrode = touch_electrode(item);
                // Rule areas inside the footprint (an antenna keepout) keep
                // tracks and vias out like board-level ones.
                for zone in item
                    .children()
                    .iter()
                    .filter(|child| child.head() == Some("zone") && is_rule_area(child))
                {
                    if let Some(obstacle) =
                        rule_area_obstacle(zone, &layers, &format!("{reference} keepout"))?
                    {
                        obstacles.push(obstacle);
                    }
                    if let Some(keepout) = pour_keepout(zone, &layers)? {
                        pour_keepouts.push(keepout);
                    }
                }
                // A net tie's pads (`net_tie_pad_groups`): its copper
                // graphics join pads of several nets, which KiCad allows.
                let tie_groups = net_tie_groups(item, board_version);
                let mut tie_copper: Vec<(core::Shape, core::LayerMask, String)> = Vec::new();
                let mut tie_pads: Vec<([f64; 2], core::LayerMask, core::NetId)> = Vec::new();
                // Copper drawn inside the footprint (a logo, a net tie's
                // bridge) belongs to no net.
                for graphic in item.children().iter().filter(|child| {
                    matches!(
                        child.head(),
                        Some("fp_line" | "fp_arc" | "fp_rect" | "fp_circle" | "fp_poly")
                    )
                }) {
                    // KiCad 9 draws a graphic on several layers at once
                    // (`(layers "B.Cu" "B.Mask")`: tuxkeychain's GRAPHIC3
                    // logo, copper and a mask opening in one polygon).
                    let names = item_layer_names(graphic);
                    // Mask graphics open the mask like the board's own
                    // (Sisu's 190 B.Mask polygons, link's connector).
                    let sides = mask_sides(&names, layers.len());
                    if !sides.is_empty() {
                        let shapes: Vec<core::Shape> = copper_graphic_shapes(graphic)?
                            .into_iter()
                            .map(|shape| place_shape(shape, footprint_at))
                            .collect();
                        if !shapes.is_empty() {
                            let expansion = form_f64(graphic, "solder_mask_margin", 1)
                                .ok()
                                .unwrap_or(board_pad_to_mask)
                                .max(0.0);
                            // A touch electrode's own opening leaves its
                            // net the vias inside its pad (`grid.rs`,
                            // `own_opening`).
                            let kind = graphic.head().unwrap_or("graphic");
                            let kind = if electrode {
                                format!("{kind} of the touch electrode {reference}")
                            } else {
                                kind.to_string()
                            };
                            for side in sides {
                                mask_openings.push((
                                    shapes.clone(),
                                    side,
                                    kind.clone(),
                                    0.0,
                                    expansion,
                                ));
                            }
                        }
                    }
                    if names.contains(&"Margin") {
                        let shapes: Vec<core::Shape> = copper_graphic_shapes(graphic)?
                            .into_iter()
                            .map(|shape| place_shape(shape, footprint_at))
                            .collect();
                        obstacles.extend(margin_obstacles(
                            shapes,
                            &layers,
                            config,
                            &format!("{reference} margin {}", graphic.head().unwrap_or("graphic")),
                        ));
                    }
                    let copper = layers.mask_of(names.iter().copied())?;
                    if copper == 0 {
                        continue;
                    }
                    // A footprint graphic of a net is copper of no net to
                    // KiCad 10.0.7 too: no pad or track connects to it, its
                    // own net's copper keeps the clearance from it (kicad-cli:
                    // an antenna polygon of net A against its own pad and an
                    // A track, clearance findings at 0 mm and the pad
                    // unconnected).
                    for shape in copper_graphic_shapes(graphic)? {
                        let shape = place_shape(shape, footprint_at);
                        let label =
                            format!("{reference} copper {}", graphic.head().unwrap_or("graphic"));
                        // A net tie's copper waits for its pads (below).
                        if !tie_groups.is_empty() {
                            tie_copper.push((shape, copper, label));
                            continue;
                        }
                        obstacles.push(core::Obstacle {
                            shape,
                            layers: copper,
                            kind: core::ObstacleKind::Copper,
                            net: None,
                            clearance: no_net_clearance,
                            clearance_override: None,
                            blocks_tracks: true,
                            blocks_vias: true,
                            label,
                        });
                    }
                }
                // The footprint's visible texts on copper are copper of no
                // net, on a mask layer openings, as the board's are
                // (jnftech's test points with their Value on F.Cu: KiCad's
                // clearance and shorting_items against "Value field").
                // A text box (`fp_text_box`) is its box, glyphs inside.
                for text in item.children().iter().filter(|child| {
                    matches!(child.head(), Some("fp_text" | "property" | "fp_text_box"))
                        && child.child("layer").is_some()
                }) {
                    let names = item_layer_names(text);
                    let copper = layers.mask_of(names.iter().copied())?;
                    let sides = mask_sides(&names, layers.len());
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
                    // (`text_box_shapes`), nor has a hidden one: left out,
                    // as the board's own are (`input_warnings` says so in
                    // the report).
                    if shapes.is_empty() {
                        continue;
                    }
                    let shapes: Vec<core::Shape> = shapes
                        .into_iter()
                        .map(|shape| place_shape(shape, footprint_at))
                        .collect();
                    for side in sides {
                        mask_openings.push((
                            shapes.clone(),
                            side,
                            format!("{reference} {kind}"),
                            0.0,
                            0.0,
                        ));
                    }
                    if copper != 0 {
                        obstacles.extend(shapes.into_iter().map(|shape| core::Obstacle {
                            shape,
                            layers: copper,
                            kind: core::ObstacleKind::Copper,
                            net: None,
                            clearance: no_net_clearance,
                            clearance_override: None,
                            blocks_tracks: true,
                            blocks_vias: true,
                            label: format!("{reference} copper {kind}"),
                        }));
                    }
                }
                footprint_ordinal += 1;
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
                // The footprint's pads with copper: centre, and for an SMD
                // pad its copper's obstacles and area (`exposed_pad`).
                let mut footprint_pads: Vec<PadBox> = Vec::new();
                // Per pad of `footprint_pads`, its copper's obstacles (the
                // pad, a padstack's other shapes).
                let mut footprint_copper: Vec<Vec<usize>> = Vec::new();
                for pad in item
                    .children()
                    .iter()
                    .filter(|child| child.head() == Some("pad"))
                {
                    let pad_name = pad.children().get(1).and_then(Expr::atom).unwrap_or("");
                    let label = format!("{reference}.{}", pad_name.trim_matches('"'));
                    // KiCad's zone connection: the pad's own, else its
                    // footprint's (0 none, 1 thermal, 2 solid).
                    let zone_connect = form_atom(pad, "zone_connect", 1)
                        .or_else(|| form_atom(item, "zone_connect", 1));
                    match zone_connect {
                        Some("0") => isolated_pads.push(label.clone()),
                        Some("2") => solid_pads.push(label.clone()),
                        _ => {}
                    }
                    let lowered = lower_pad(pad, footprint_at)?;
                    let pad_type = pad.children().get(2).and_then(Expr::atom).unwrap_or("");
                    // Plated holes of pads without a net are just holes to
                    // everyone; the board's hole clearance applies. Holes of
                    // mechanical (unplated) pads always are.
                    let hole = (pad_type == "thru_hole"
                        && !node_net(pad).is_some_and(|raw| net_names.routable(raw)))
                        || pad_type == "np_thru_hole";
                    if hole && let Some(shape) = drill_shape(pad, lowered.center)? {
                        // KiCad 10.0.7's copper_edge_clearance takes an
                        // unplated slot (a milled hole, its drill sizes
                        // differ) for board edge: copper keeps the edge
                        // clearance from it (facade60's H5 and H8, the USB-C
                        // shell slots of buganini's J1 and J3). A round hole
                        // keeps only the hole clearance.
                        let edge = if pad_type == "np_thru_hole"
                            && matches!(shape, core::Shape::Capsule { .. })
                        {
                            config.edge_clearance_mm
                        } else {
                            0.0
                        };
                        obstacles.push(core::Obstacle {
                            shape,
                            layers: layers.all(),
                            kind: core::ObstacleKind::Hole,
                            net: None,
                            clearance: local_clearance::pad_clearance(pad, item)?.max(edge),
                            clearance_override: None,
                            blocks_tracks: true,
                            blocks_vias: true,
                            label: if pad_type == "np_thru_hole" {
                                format!("{label} hole")
                            } else {
                                format!("{label} plated hole")
                            },
                        });
                    }
                    // A plated hole is copper: a through-hole pad that
                    // lists no copper layer (`(layers "*.Mask")`, aaronds's
                    // ESP32 DevKitC mounting pins) is copper on every layer
                    // to KiCad 10.0.7 (kicad-cli: shorting_items on F.Cu,
                    // an inner layer and B.Cu), one listing some only on
                    // those.
                    let mut pad_layers = layers.mask_of_item(pad)?;
                    if pad_type == "thru_hole" && pad_layers == 0 {
                        pad_layers = layers.all();
                    }
                    if pad_layers == 0 {
                        // A pad with a mask opening and no copper (hackrf's
                        // mask-only test points over a track) bares the
                        // copper under it: KiCad reports a solder mask
                        // bridge when copper of a second net lies there. It
                        // is an opening like a mask graphic, grown by the
                        // pad's mask expansion.
                        if config.mask_only_pad_openings != Some(false) {
                            let names: Vec<&str> = pad
                                .child("layers")
                                .map(Expr::children)
                                .unwrap_or_default()
                                .iter()
                                .skip(1)
                                .filter_map(Expr::atom)
                                .collect();
                            let front = names
                                .iter()
                                .any(|name| matches!(*name, "F.Mask" | "*.Mask" | "F&B.Mask"));
                            let back = names
                                .iter()
                                .any(|name| matches!(*name, "B.Mask" | "*.Mask" | "F&B.Mask"));
                            let margin = form_f64(pad, "solder_mask_margin", 1)
                                .ok()
                                .or_else(|| form_f64(item, "solder_mask_margin", 1).ok())
                                .unwrap_or(board_pad_to_mask)
                                .max(0.0);
                            let kind = format!("pad {label}");
                            for (open, side) in [(front, 0), (back, layers.len() - 1)] {
                                if open {
                                    mask_openings.push((
                                        vec![shape(&lowered.geometry)],
                                        side,
                                        kind.clone(),
                                        margin,
                                        margin,
                                    ));
                                }
                            }
                        }
                        continue;
                    }
                    let net_name = node_net(pad).map(|raw| net_names.normalize(raw));
                    let net = node_net(pad)
                        .filter(|raw| net_names.routable(raw))
                        .map(|raw| net_names.normalize(raw))
                        .map(|name| net_id(name, &mut nets, &mut classes))
                        .transpose()?;
                    // A positive pad or footprint override replaces the
                    // net class's clearance in KiCad, also when smaller.
                    let clearance_override = Some(local_clearance::pad_clearance(pad, item)?)
                        .filter(|value| *value > 0.0);
                    let mut clearance = 0.0;
                    if net.is_none() {
                        // A pad the router never connects (single-pin net,
                        // unconnected pin, no net) still keeps its net
                        // class's clearance in KiCad's DRC.
                        let rules = net_name
                            .and_then(|name| config.connection_rules.get(name))
                            .or(config.default_rules.as_ref());
                        if let Some(rules) = rules {
                            clearance = rules.clearance_mm;
                        }
                    }
                    let pad_obstacle = obstacles.len();
                    let number = pad_name.trim_matches('"');
                    if let Some(net) = net
                        && tie_groups
                            .iter()
                            .any(|group| group.iter().any(|member| member == number))
                    {
                        tie_pads.push((lowered.center, pad_layers, net));
                    }
                    if let Some(group) = pad_groups
                        .iter()
                        .position(|group| group.iter().any(|member| member == number))
                    {
                        jumper_groups
                            .insert(pad_obstacle, (footprint_ordinal, format!("group {group}")));
                    } else if duplicates_are_jumpers && !number.is_empty() {
                        jumper_groups.insert(
                            pad_obstacle,
                            (footprint_ordinal, format!("number {number}")),
                        );
                    }
                    match zone_connect {
                        Some("0") => isolated_pad_obstacles.push(pad_obstacle),
                        Some("2") => solid_pad_obstacles.push(pad_obstacle),
                        _ => {}
                    }
                    if let Ok(frame) = pad_spokes_frame(pad, lowered.center) {
                        pad_spokes.insert(pad_obstacle, frame);
                    }
                    // A KiCad 9 padstack draws the pad in another shape or
                    // size on its inner or back layers (adrianmarple's
                    // Raspberry Pi header: 1.2 mm on F.Cu, 1.778 on B.Cu;
                    // dennisss's lens mounts: 4 mm and 3): the pad's own
                    // shape is copper only on the layers the stack does not
                    // name (F.Cu always), each named layer has its entry's
                    // shape, a copper obstacle of the pad's net merged into
                    // the pad's terminal (`MergedPad::padstack`). A smaller
                    // back shape under the front one left phantom copper:
                    // a B.Cu track ending there was "connected" to the
                    // model and not to KiCad. (A pad whose every layer the
                    // stack names keeps its own shape on all of them.)
                    let variants: Vec<(core::LayerMask, Expr)> = padstack_variants(pad, &layers)
                        .into_iter()
                        .map(|(mask, variant)| (mask & pad_layers, variant))
                        .filter(|(mask, _)| *mask != 0)
                        .collect();
                    let named = variants.iter().fold(0, |union, (mask, _)| union | mask);
                    let own_layers = match pad_layers & !named {
                        0 => pad_layers,
                        own => own,
                    };
                    obstacles.push(core::Obstacle {
                        shape: shape(&lowered.geometry),
                        layers: own_layers,
                        kind: core::ObstacleKind::Copper,
                        net,
                        clearance,
                        clearance_override,
                        blocks_tracks: true,
                        blocks_vias: true,
                        label: label.clone(),
                    });
                    let smd = matches!(pad_type, "smd" | "connect");
                    footprint_pads.push(PadBox {
                        number: number.to_string(),
                        center: lowered.center,
                        smd,
                        bounds: {
                            let bounds = obstacles[pad_obstacle].shape.aabb();
                            [bounds.minimum, bounds.maximum]
                        },
                    });
                    footprint_copper.push(vec![pad_obstacle]);
                    let mut back_geometry = None;
                    for (mask, variant) in variants {
                        let geometry = lower_pad(&variant, footprint_at)?.geometry;
                        if mask & (1 << (layers.len() - 1)) != 0 {
                            back_geometry = Some(geometry.clone());
                        }
                        let variant_obstacle = obstacles.len();
                        if let Some(own) = footprint_copper.last_mut() {
                            own.push(variant_obstacle);
                        }
                        if own_layers != pad_layers {
                            // The pad's copper on those layers: its zone
                            // connection, its spokes' frame in its own
                            // size, its terminal's.
                            match zone_connect {
                                Some("0") => isolated_pad_obstacles.push(variant_obstacle),
                                Some("2") => solid_pad_obstacles.push(variant_obstacle),
                                _ => {}
                            }
                            if let Ok(frame) = pad_spokes_frame(&variant, lowered.center) {
                                pad_spokes.insert(variant_obstacle, frame);
                            }
                            if net.is_some() {
                                padstack_copper.push((pad_obstacle, variant_obstacle));
                            }
                        }
                        obstacles.push(core::Obstacle {
                            shape: shape(&geometry),
                            layers: mask,
                            kind: core::ObstacleKind::Copper,
                            net,
                            clearance,
                            clearance_override,
                            blocks_tracks: true,
                            blocks_vias: true,
                            label: format!("{label} padstack"),
                        });
                    }
                    // The pad's mask aperture, where it reaches past the
                    // copper clearance. A mask opening of the pad's net; its
                    // own net's copper never bridges (KiCad skips pairs of
                    // one net). KiCad's bridge test (10.0.7,
                    // DRC_TEST_PROVIDER_SOLDER_MASK): covered copper (a
                    // track, a tented via) bridges within the project's
                    // solder-mask-to-copper clearance plus the pad's
                    // expansion (OpenESC 4in1's J1: margin 0.102 mm plus
                    // 0.005, tracks 0.105 mm from the pads bridged); an
                    // untented via is an opening itself and bridges within
                    // the minimum web plus both expansions. Kept from
                    // before, for a pad or footprint margin of its own:
                    // the margin plus half the web (a "Hole, 3mm"
                    // footprint, a ring of small pads with 0.1 mm margins
                    // whose apertures merge into a mask-free annulus).
                    let local_margin = form_f64(pad, "solder_mask_margin", 1)
                        .ok()
                        .or_else(|| form_f64(item, "solder_mask_margin", 1).ok());
                    let expansion = local_margin.unwrap_or(board_pad_to_mask);
                    let web_reach = local_margin
                        .filter(|margin| *margin > 0.0)
                        .map_or(0.0, |margin| margin + pad_mask_reach);
                    let covered = if strict_mask {
                        web_reach.max(mask_to_copper + expansion)
                    } else {
                        web_reach
                    };
                    let floor = smallest_clearance.min(clearance_override.unwrap_or(f64::INFINITY))
                        - 1.0e-9;
                    let mask_names: Vec<&str> = pad
                        .child("layers")
                        .map(Expr::children)
                        .unwrap_or_default()
                        .iter()
                        .skip(1)
                        .filter_map(Expr::atom)
                        .collect();
                    let sides = [
                        (
                            0usize,
                            mask_names
                                .iter()
                                .any(|name| matches!(*name, "F.Mask" | "*.Mask" | "F&B.Mask")),
                        ),
                        (
                            layers.len() - 1,
                            mask_names
                                .iter()
                                .any(|name| matches!(*name, "B.Mask" | "*.Mask" | "F&B.Mask")),
                        ),
                    ];
                    for (index, (side, open)) in sides.into_iter().enumerate() {
                        // The looser rule: every outer copper layer of the
                        // pad, mask layer or not, both sides in one.
                        // A pad opens the mask on a side it has no copper on
                        // when its layers say so (c-lydia's U3: copper on
                        // B.Cu, the opening on F.Mask): KiCad bridges the
                        // pad to other nets' copper under that opening.
                        let bare_side = strict_mask && open && pad_layers & (1 << side) == 0;
                        let open = open || !strict_mask;
                        if !open
                            || (pad_layers & (1 << side) == 0 && !bare_side)
                            || (index == 1 && side == 0)
                        {
                            continue;
                        }
                        let via_reach = if tented[index] || !strict_mask {
                            covered
                        } else {
                            web_reach.max(min_mask_web + expansion + board_pad_to_mask)
                        };
                        // The back's aperture is the back copper's.
                        let side_geometry = match &back_geometry {
                            Some(geometry) if index == 1 => geometry,
                            _ => &lowered.geometry,
                        };
                        let aperture = |clearance: f64, blocks_tracks: bool, blocks_vias: bool| {
                            core::Obstacle {
                                shape: shape(side_geometry),
                                layers: 1 << side,
                                kind: core::ObstacleKind::Keepout,
                                net,
                                clearance,
                                clearance_override: None,
                                blocks_tracks,
                                blocks_vias,
                                label: format!("{label} mask aperture"),
                            }
                        };
                        // A reach at the copper clearance adds nothing (KiCad's
                        // tests are strict on both); the older rule keeps
                        // its own threshold (USB_Cable_Tester: 0.05 + 0.05
                        // mm against classes of 0.1 rerouted for nothing).
                        // Without copper on this side no copper clearance
                        // holds for it: the aperture is all there is.
                        let needed = |reach: f64| {
                            bare_side
                                || reach > floor + 2.0e-9
                                || (reach == web_reach && reach > floor)
                        };
                        if (via_reach - covered).abs() < 1.0e-9 {
                            if needed(covered) {
                                obstacles.push(aperture(covered, true, true));
                            }
                        } else {
                            if needed(covered) {
                                obstacles.push(aperture(covered, true, false));
                            }
                            if needed(via_reach) {
                                obstacles.push(aperture(via_reach, false, true));
                            }
                        }
                    }
                    // Other nets keep the hole clearance from a plated hole
                    // too, which reaches past a thin annular ring (a thermal
                    // via pad: 0.3 drill in 0.5 copper, eurorack-pmod's U1).
                    // Their vias as well (jetson's J2: 0.55 drill in 0.75
                    // copper, clearance 0.125, hole clearance 0.25); the
                    // pad's own net is exempt (`StaticMaps::via_owner`).
                    if pad_type == "thru_hole"
                        && net.is_some()
                        && let Some(shape) = drill_shape(pad, lowered.center)?
                    {
                        obstacles.push(core::Obstacle {
                            shape,
                            layers: layers.all(),
                            kind: core::ObstacleKind::Hole,
                            net,
                            clearance: 0.0,
                            clearance_override: None,
                            blocks_tracks: true,
                            blocks_vias: true,
                            label: format!("{label} plated hole"),
                        });
                    }
                    if let Some(net) = net {
                        // Without a hole, a pad on several layers (an
                        // edge-mount connector's top and bottom pads) does
                        // not join them: it is reached on its footprint's
                        // side, and layer changes need a via.
                        let mut terminal_layers = pad_layers;
                        if pad.child("drill").is_none() && pad_layers.count_ones() > 1 {
                            let side = form_atom(item, "layer", 1)
                                .and_then(|name| layers.index(name))
                                .map(|layer| 1 << layer)
                                .filter(|mask: &core::LayerMask| mask & pad_layers != 0);
                            terminal_layers = side.unwrap_or(pad_layers.isolate_lowest_one());
                        }
                        // A custom pad connects through its anchor only.
                        let contact = match (
                            pad.children().get(3).and_then(Expr::atom),
                            &lowered.geometry,
                        ) {
                            (Some("custom"), ObstacleGeometry::Union { parts })
                                if parts.len() > 1 =>
                            {
                                Some(shape(&parts[0]))
                            }
                            _ => None,
                        };
                        nets[net as usize].terminals.push(core::Terminal {
                            anchor: lowered.center,
                            layers: terminal_layers,
                            pad: pad_obstacle,
                            contact,
                            label,
                        });
                    }
                }
                pin_row_gaps.extend(footprint_pin_row_gaps(
                    &footprint_pads
                        .iter()
                        .zip(&footprint_copper)
                        .filter(|(pad, _)| pad.smd)
                        .map(|(pad, copper)| {
                            let obstacle = &obstacles[copper[0]];
                            (pad.center, obstacle.layers, obstacle.net)
                        })
                        .collect::<Vec<_>>(),
                ));
                if config.via_in_pad != Some(true) {
                    let exposed = exposed_pad(&footprint_pads);
                    for (index, pad) in footprint_pads.iter().enumerate() {
                        if pad.smd && !exposed.contains(&index) && !electrode {
                            no_via_pads.extend(&footprint_copper[index]);
                        }
                    }
                }
                // A net tie's copper: KiCad 10.0.7 lets the tie's own nets
                // touch it (kicad-cli: tracks of A and B ending on the tie's
                // pads over its bridge, and an A track ending on the bridge's
                // middle, find no clearance or short against it; only the
                // other pad's clearance holds) and keeps every other net
                // clear of it (the same footprint without
                // `net_tie_pad_groups`: clearance and shorting_items against
                // the polygon). One owner per lattice node cannot say "A or
                // B", so the bridge is copper of no net less what lies
                // within a track's reach of a tie pad's centre: a track of
                // the pad's net ends there (neotron-pico's NT901 to NT912,
                // every pad dead behind its own bridge before). Other nets
                // keep clear of the rest and of the pads.
                for (shape, copper, label) in tie_copper {
                    // The whole bridge keeps every via out, the tie's own
                    // nets' too: a track of a tie net may touch the bridge,
                    // a drill may not come within the hole clearance of
                    // it (KiCad's hole_clearance: brown-studios/pdlc's EN
                    // via 0.164 mm from NT1's bridge, carved away above
                    // for EN's tracks).
                    obstacles.push(core::Obstacle {
                        shape: shape.clone(),
                        layers: copper,
                        kind: core::ObstacleKind::Copper,
                        net: None,
                        clearance: no_net_clearance,
                        clearance_override: None,
                        blocks_tracks: false,
                        blocks_vias: true,
                        label: format!("{label} (vias)"),
                    });
                    let carves: Vec<([f64; 2], f64)> = tie_pads
                        .iter()
                        .filter(|(_, pad_layers, _)| pad_layers & copper != 0)
                        .map(|(center, _, net)| {
                            let class = &classes[nets[*net as usize].class];
                            (
                                *center,
                                class.clearance.max(no_net_clearance)
                                    + class.trace_width / 2.0
                                    + NET_TIE_MARGIN,
                            )
                        })
                        .collect();
                    if let Some(shape) = carve_net_tie(shape, &carves) {
                        obstacles.push(core::Obstacle {
                            shape,
                            layers: copper,
                            kind: core::ObstacleKind::Copper,
                            net: None,
                            clearance: no_net_clearance,
                            clearance_override: None,
                            blocks_tracks: true,
                            blocks_vias: true,
                            label,
                        });
                    }
                }
            }
            Some("segment") => {
                let Some(layer) = form_atom(item, "layer", 1).and_then(|name| layers.index(name))
                else {
                    continue;
                };
                let net = item_net(item, &net_codes)
                    .filter(|raw| net_names.routable(raw))
                    .map(|raw| net_names.normalize(raw))
                    .map(|name| net_id(name, &mut nets, &mut classes))
                    .transpose()?;
                obstacles.push(core::Obstacle {
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
                    blocks_tracks: true,
                    blocks_vias: true,
                    label: "existing track".into(),
                });
            }
            Some("via") => {
                let net = item_net(item, &net_codes)
                    .filter(|raw| net_names.routable(raw))
                    .map(|raw| net_names.normalize(raw))
                    .map(|name| net_id(name, &mut nets, &mut classes))
                    .transpose()?;
                obstacles.push(core::Obstacle {
                    shape: core::Shape::Circle {
                        center: form_xy(item, "at")?,
                        radius: form_f64(item, "size", 1)? / 2.0,
                    },
                    layers: via_layers(item, &layers),
                    kind: core::ObstacleKind::Copper,
                    net,
                    clearance: 0.0,
                    clearance_override: None,
                    blocks_tracks: true,
                    blocks_vias: true,
                    label: "existing via".into(),
                });
            }
            // An arc track: its chords, grown by the flattening's
            // tolerance so they hold the true arc (as copper graphics'
            // arcs are), fixed copper of its net like a segment.
            Some("arc") => {
                let Some(layer) = form_atom(item, "layer", 1).and_then(|name| layers.index(name))
                else {
                    continue;
                };
                let net = item_net(item, &net_codes)
                    .filter(|raw| net_names.routable(raw))
                    .map(|raw| net_names.normalize(raw))
                    .map(|name| net_id(name, &mut nets, &mut classes))
                    .transpose()?;
                let radius = form_f64(item, "width", 1)? / 2.0 + outline::ARC_TOLERANCE;
                let points = outline::arc_points(
                    form_xy(item, "start")?,
                    form_xy(item, "mid")?,
                    form_xy(item, "end")?,
                );
                for pair in points.windows(2) {
                    obstacles.push(core::Obstacle {
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
                        blocks_tracks: true,
                        blocks_vias: true,
                        label: "existing track".into(),
                    });
                }
            }
            // Board graphics: copper on the copper layers they name (KiCad
            // 9 draws them on several layers at once, with a net: Sisu's
            // GND rectangles on F.Cu and F.Mask), openings on the mask
            // layers. Graphics on a mask layer open the mask: copper of two
            // nets under one opening is a solder bridge (KiCad's
            // solder_mask_bridge), so routed copper stays out from under
            // them on that side.
            // A dimension or a text box on a copper layer is copper of no
            // net to KiCad (shrimp-18's "Dimension '10' on F.Cu": three
            // shorting_items); a text box on a mask layer opens it.
            Some("dimension" | "gr_dimension" | "gr_text_box") => {
                let copper = layers.mask_of_item(item)?;
                let text_box = item.head() == Some("gr_text_box");
                let sides = if text_box {
                    mask_sides(&item_layer_names(item), layers.len())
                } else {
                    Vec::new()
                };
                if copper == 0 && sides.is_empty() {
                    continue;
                }
                let shapes = if text_box {
                    text_box_shapes(item)?
                } else {
                    dimension_shapes(item)?
                };
                if shapes.is_empty() {
                    continue;
                }
                let kind = if text_box { "text box" } else { "dimension" };
                for side in sides {
                    mask_openings.push((shapes.clone(), side, kind.to_string(), 0.0, 0.0));
                }
                if copper != 0 {
                    copper_graphics.push(CopperGraphic {
                        shapes,
                        layers: copper,
                        net: None,
                        kind: kind.to_string(),
                        clearance: no_net_clearance,
                    });
                }
            }
            Some("gr_text" | "gr_line" | "gr_rect" | "gr_arc" | "gr_circle" | "gr_poly") => {
                let copper = layers.mask_of_item(item)?;
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
                let sides: Vec<usize> = names
                    .iter()
                    .filter_map(|name| match *name {
                        "F.Mask" => Some(0),
                        "B.Mask" => Some(layers.len() - 1),
                        _ => None,
                    })
                    .collect();
                // KiCad keeps the edge clearance from graphics on Margin
                // as from the board edge (copper_edge_clearance: "Segment
                // on Margin", michpro's PY32F002 board).
                if names.contains(&"Margin") && item.head() != Some("gr_text") {
                    obstacles.extend(margin_obstacles(
                        copper_graphic_shapes(item)?,
                        &layers,
                        config,
                        &format!("board margin ({})", item.head().unwrap_or("graphic")),
                    ));
                }
                if copper == 0 && sides.is_empty() {
                    continue;
                }
                let shapes = copper_graphic_shapes(item)?;
                let kind = item.head().unwrap_or("graphic").to_string();
                if copper != 0 {
                    let net = item_net(item, &net_codes)
                        .filter(|raw| net_names.routable(raw))
                        .map(|raw| net_names.normalize(raw).to_string());
                    let clearance = item_net(item, &net_codes)
                        .map(|raw| net_names.normalize(raw))
                        .and_then(|name| config.connection_rules.get(name))
                        .map_or(no_net_clearance, |rules| rules.clearance_mm);
                    let mut copper_shapes = shapes.clone();
                    if item.head() == Some("gr_text") {
                        copper_shapes.extend(cached_text_line_boxes(item)?);
                    }
                    copper_graphics.push(CopperGraphic {
                        shapes: copper_shapes,
                        layers: copper,
                        net,
                        kind: kind.clone(),
                        clearance,
                    });
                }
                let shapes = if strict_mask && !sides.is_empty() {
                    mask_graphic_shapes(item)?
                } else {
                    shapes
                };
                // KiCad's bridge test grows a shape by its mask expansion,
                // a text by nothing.
                let expansion = if item.head() == Some("gr_text") {
                    0.0
                } else {
                    form_f64(item, "solder_mask_margin", 1)
                        .ok()
                        .unwrap_or(board_pad_to_mask)
                        .max(0.0)
                };
                for side in sides {
                    mask_openings.push((shapes.clone(), side, kind.clone(), 0.0, expansion));
                }
            }
            Some("zone") if is_rule_area(item) => {
                if let Some(obstacle) = rule_area_obstacle(item, &layers, "unnamed rule area")? {
                    // Tracks out, pours in (a plane layer's `no tracks`
                    // area): KiCad fills it, so the pour model does too.
                    if obstacle.blocks_tracks && rule_area_allows_pours(item) {
                        fill_passes.push(obstacles.len());
                    }
                    obstacles.push(obstacle);
                }
                if let Some(region) = keep_whole_region(item, &layers)? {
                    keep_whole.push(region);
                }
                if let Some(keepout) = pour_keepout(item, &layers)? {
                    pour_keepouts.push(keepout);
                }
            }
            _ => {}
        }
    }
    // Pads of one number that overlap on a copper layer they share are one
    // pad to KiCad (a solder pad with holes in it, an exposed pad's thermal
    // vias): the largest is the terminal, the others just its copper.
    // OpenRX's panel hangs half of each wire pad's holes over the edge,
    // unreachable as terminals. Pads of one number at one place on
    // different layers without a hole (an edge connector's or a USB-C
    // shell's top and bottom pads) are two items to KiCad's connectivity,
    // which wants them joined by copper: two terminals (Knox Key's P1.A1:
    // a track on B.Cu reached the bottom pad, KiCad found the top one
    // unconnected). The overlap is transitive: an exposed pad on F.Cu and
    // its copy on B.Cu are one item through the thermal vias between them
    // (MSign's U1.21, Sisu's U2.11), a reversible diode's SMD pads on both
    // sides one through its through-hole pad (urchin's and
    // eternal-keypad's D_SMD_TH), so the pads of one number are grouped by
    // overlap and the largest of each group is its terminal.
    let mut merged_pads: Vec<core::MergedPad> = Vec::new();
    for (net_id, net) in nets.iter_mut().enumerate() {
        let area = |terminal: &core::Terminal| {
            let bounds = obstacles[terminal.pad].shape.aabb();
            (bounds.maximum[0] - bounds.minimum[0]) * (bounds.maximum[1] - bounds.minimum[1])
        };
        let count = net.terminals.len();
        let mut order: Vec<usize> = (0..count).collect();
        order.sort_by(|a, b| {
            area(&net.terminals[*b])
                .total_cmp(&area(&net.terminals[*a]))
                .then(a.cmp(b))
        });
        // Union by overlap; the root of a group is its first pad in `order`
        // (the largest), so it is the terminal.
        let mut rank = vec![0; count];
        for (position, &index) in order.iter().enumerate() {
            rank[index] = position;
        }
        let mut parent: Vec<usize> = (0..count).collect();
        fn root(parent: &mut [usize], mut index: usize) -> usize {
            while parent[index] != index {
                parent[index] = parent[parent[index]];
                index = parent[index];
            }
            index
        }
        // The group's root stays its first pad in `order`.
        let join = |parent: &mut [usize], a: usize, b: usize| {
            let (ra, rb) = (root(parent, a), root(parent, b));
            if rank[ra] < rank[rb] {
                parent[rb] = ra;
            } else {
                parent[ra] = rb;
            }
        };
        let overlap = |a: &core::Terminal, b: &core::Terminal| {
            let (pa, pb) = (&obstacles[a.pad], &obstacles[b.pad]);
            pa.layers & pb.layers != 0
                && (pa.shape.contains(b.anchor) || pb.shape.contains(a.anchor))
        };
        let mut by_label: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (index, terminal) in net.terminals.iter().enumerate() {
            by_label.entry(&terminal.label).or_default().push(index);
        }
        for group in by_label.values().filter(|group| group.len() > 1) {
            for (position, &a) in group.iter().enumerate() {
                for &b in &group[position + 1..] {
                    if !overlap(&net.terminals[a], &net.terminals[b]) {
                        continue;
                    }
                    join(&mut parent, a, b);
                }
            }
        }
        // Pads of a jumper group are one item to KiCad's connectivity.
        let mut by_jumper: BTreeMap<(usize, &str), Vec<usize>> = BTreeMap::new();
        for (index, terminal) in net.terminals.iter().enumerate() {
            if let Some((footprint, group)) = jumper_groups.get(&terminal.pad) {
                by_jumper
                    .entry((*footprint, group.as_str()))
                    .or_default()
                    .push(index);
            }
        }
        for group in by_jumper.values() {
            for &other in &group[1..] {
                join(&mut parent, group[0], other);
            }
        }
        let host: Vec<usize> = (0..count).map(|index| root(&mut parent, index)).collect();
        let keep: Vec<bool> = (0..count).map(|index| host[index] == index).collect();
        let mut renumbered = vec![usize::MAX; net.terminals.len()];
        let mut next = 0;
        for (index, kept) in keep.iter().enumerate() {
            if *kept {
                renumbered[index] = next;
                next += 1;
            }
        }
        for (index, terminal) in net.terminals.iter().enumerate() {
            if !keep[index] {
                merged_pads.push(core::MergedPad {
                    net: net_id as core::NetId,
                    host: renumbered[host[index]],
                    pad: terminal.pad,
                    anchor: terminal.anchor,
                    layers: obstacles[terminal.pad].layers,
                    padstack: false,
                });
            }
        }
        let mut index = 0;
        net.terminals.retain(|_| {
            index += 1;
            keep[index - 1]
        });
    }
    // A padstack pad's copper on the layers its stack names joins its
    // pad's terminal (the terminal the pad was merged into, if it was).
    for (own, variant) in padstack_copper {
        let Some(net) = obstacles[own].net else {
            continue;
        };
        let terminals = &nets[net as usize].terminals;
        let host = terminals
            .iter()
            .position(|terminal| terminal.pad == own)
            .map(|host| (host, terminals[host].anchor))
            .or_else(|| {
                merged_pads
                    .iter()
                    .find(|merged| merged.net == net && merged.pad == own)
                    .map(|merged| (merged.host, merged.anchor))
            });
        if let Some((host, anchor)) = host {
            merged_pads.push(core::MergedPad {
                net,
                host,
                pad: variant,
                anchor,
                layers: obstacles[variant].layers,
                padstack: true,
            });
        }
    }
    // Copper graphics belong to their net (other nets keep clear) or, without
    // one, to none.
    for (index, graphic) in copper_graphics.into_iter().enumerate() {
        let CopperGraphic {
            shapes,
            layers: copper,
            net,
            kind,
            clearance,
        } = graphic;
        let net = net.and_then(|name| net_ids_lookup(&nets, &name));
        // A filled graphic with a net is copper KiCad wants connected like
        // a pad (Sisu's antenna feed, a rectangle on /RF/ANT, stayed an
        // island): it is a terminal of its net too.
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
            // A stroke with a net (a line, an arc, a circle's ring) is an
            // item of KiCad's connectivity like a pad, which wants it joined
            // to the net (BleJoystick's arcs, the chessboards' rings, all
            // `(net N)` by code): a terminal at the middle of its first
            // piece, unless it lies on copper of a terminal of its net
            // already (a line drawn from a pad: one item with it).
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
            blocks_tracks: true,
            blocks_vias: true,
            label: format!("copper {kind}"),
        }));
    }
    // A mask opening exposes the copper under it: pads, or else the fill
    // of a pour. Over one net (a pad or a pour exposed on purpose) it is
    // that net's: it may route there, others would bridge to it. Over
    // several (J_LCD1's opening over its eight pads in Sisu) or none,
    // nothing new goes under it; pads inside leave it by their stubs. The
    // net is the whole graphic's.
    let pours = pours(pcb, &layers)?;
    // KiCad's bridge test drops mask slivers under the minimum web width:
    // copper of another net within half of it of an opening is exposed
    // (link: a track 0.025 mm from an F.Mask polygon bridged it to GND with
    // a 0.1 mm minimum; at 0.07 mm it did not).
    let mask_reach = pcb
        .child("setup")
        .and_then(|setup| form_f64(setup, "solder_mask_min_width", 1).ok())
        .map_or(0.0, |width| width / 2.0 + 0.01);
    // KiCad's bridge test (DRC_TEST_PROVIDER_SOLDER_MASK, KiCad 10)
    // exposes copper within the project's solder-mask-to-copper clearance
    // plus the aperture's expansion, and never less than the web rule
    // above.
    for (shapes, layer, kind, margin, expansion) in mask_openings {
        let reach = (mask_reach + margin).max(mask_to_copper + expansion);
        let Some(first) = shapes.first() else {
            continue;
        };
        let bounds = shapes
            .iter()
            .skip(1)
            .fold(first.aabb(), |bounds, shape| bounds.union(shape.aabb()))
            .inflated(margin);
        let mut exposed: std::collections::BTreeSet<core::NetId> = obstacles
            .iter()
            .filter(|obstacle| {
                obstacle.kind == core::ObstacleKind::Copper
                    && obstacle.layers & (1 << layer) != 0
                    && obstacle.shape.aabb().intersects(bounds)
            })
            .filter_map(|obstacle| obstacle.net)
            .collect();
        let center = [
            (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
            (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
        ];
        let under: Vec<&Pour> = pours
            .iter()
            .filter(|pour| {
                pour.layers & (1 << layer) != 0
                    && core::geometry::point_in_polygon(center, &pour.polygon)
            })
            .collect();
        // A pour's fill keeps its clearance from pads: it shows through
        // only where the opening has no pad of its own.
        if exposed.is_empty()
            && let Some(top) = under.iter().map(|pour| pour.priority).max()
        {
            exposed.extend(
                under
                    .iter()
                    .filter(|pour| pour.priority == top)
                    .filter_map(|pour| net_ids_lookup(&nets, &pour.net)),
            );
        }
        if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
            eprintln!(
                "mask opening ({kind}) on layer {layer}: {:?}, nets {exposed:?}",
                bounds
            );
        }
        // Over bare board an opening is no net's: copper of two nets under
        // one text bridges (jetson-nano's B.Mask title over three nets'
        // vias and tracks).
        let net = if exposed.len() == 1 {
            exposed.first().copied()
        } else {
            None
        };
        obstacles.extend(shapes.into_iter().map(|shape| core::Obstacle {
            shape,
            layers: 1 << layer,
            kind: core::ObstacleKind::Keepout,
            net,
            clearance: reach,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: format!("solder mask opening ({kind})"),
        }));
    }
    for cutout in &loops.cutouts {
        // A cutout is board edge to KiCad: the edge clearance applies, not
        // the hole clearance.
        obstacles.push(core::Obstacle {
            shape: core::Shape::Polygon {
                points: cutout.clone(),
            },
            layers: layers.all(),
            kind: core::ObstacleKind::Hole,
            net: None,
            clearance: config.edge_clearance_mm + outline::ARC_TOLERANCE,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: "board cutout".into(),
        });
    }
    // A region where a pour stays whole keeps other nets' tracks off its
    // layers and their vias out; the pour's own net may pass (and its
    // fill fills it).
    for (net, mask, points, label) in keep_whole {
        let Some(id) = net_ids_lookup(&nets, &net) else {
            continue;
        };
        obstacles.push(core::Obstacle {
            shape: core::Shape::Polygon { points },
            layers: mask,
            kind: core::ObstacleKind::Keepout,
            net: Some(id),
            clearance: 0.0,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label,
        });
    }
    let board_area = polygon_area(&loops.outline);
    let mut planes = Vec::new();
    for pour in &pours {
        // A pour without pads has nothing to connect.
        let Some(net) = net_ids_lookup(&nets, &pour.net) else {
            continue;
        };
        let net_class = classes[nets[net as usize].class];
        let brush = core::RuleClass {
            trace_width: pour.min_thickness.max(0.05),
            clearance: pour.clearance.max(net_class.clearance),
            ..net_class
        };
        let class = classes
            .iter()
            .position(|existing| *existing == brush)
            .unwrap_or_else(|| {
                classes.push(brush);
                classes.len() - 1
            });
        for layer in 0..layers.len() {
            if pour.layers & (1 << layer) == 0 {
                continue;
            }
            planes.push(core::Plane {
                net,
                class,
                layer,
                polygon: pour.polygon.clone(),
                // Another net's zone of a higher priority, of an equal one
                // the larger uuid (KiCad's ZONE::HigherPriority: USB-C
                // 3-way splitter's +5V and GND, both of priority 1).
                excluded: pours
                    .iter()
                    .filter(|other| {
                        other.net != pour.net
                            && other.layers & (1 << layer) != 0
                            && (other.priority, &other.uuid) > (pour.priority, &pour.uuid)
                    })
                    .map(|other| other.polygon.clone())
                    .chain(
                        pour_keepouts
                            .iter()
                            .filter(|(mask, _)| mask & (1 << layer) != 0)
                            .map(|(_, polygon)| polygon.clone()),
                    )
                    .collect(),
                // KiCad cuts a same-net zone of a strictly higher priority
                // out of this one's fill (an equal one overlaps it).
                above: pours
                    .iter()
                    .filter(|other| {
                        other.net == pour.net
                            && other.layers & (1 << layer) != 0
                            && other.priority > pour.priority
                    })
                    .map(|other| other.polygon.clone())
                    .collect(),
                connect: connect_pours,
                thermal_reach: pour.thermal_reach,
                thermal_gap: pour.thermal_gap,
                // An inner layer poured over nearly the whole board is a
                // plane: it carries no signals.
                exclusive: layers.names[layer].starts_with("In")
                    && polygon_area(&pour.polygon) >= 0.9 * board_area,
                solid: pour.solid,
                hatch: pour.hatch,
                island_min_area: pour.island_min_area,
            });
        }
    }
    if let Some(limit) = config.exclusive_layers {
        let mut by_net: BTreeMap<core::NetId, Vec<usize>> = BTreeMap::new();
        for (index, plane) in planes.iter().enumerate() {
            if plane.exclusive {
                by_net.entry(plane.net).or_default().push(index);
            }
        }
        let last = layers.len().saturating_sub(1);
        for (_, mut indices) in by_net {
            indices.sort_by_key(|index| {
                (
                    planes[*index].layer.min(last - planes[*index].layer),
                    planes[*index].layer,
                )
            });
            for index in indices.into_iter().skip(limit) {
                planes[index].exclusive = false;
            }
        }
    }
    if classes.is_empty() {
        return Err("the board has no routable connections".into());
    }
    let mut lowered = Lowered {
        board: core::Board {
            layer_count: layers.len(),
            neck_width: config
                .neck_width_mm
                .unwrap_or_else(|| {
                    classes
                        .iter()
                        .map(|class| class.trace_width)
                        .fold(f64::INFINITY, f64::min)
                })
                .max(config.width_floor()),
            outline: loops.outline.clone(),
            // Arcs are flattened to chords inside the true curve; the
            // chord error is added so copper stays clear of the real edge.
            edge_clearance: config.edge_clearance_mm + outline::ARC_TOLERANCE,
            hole_clearance: config.hole_clearance_mm,
            hole_to_hole: config.hole_to_hole_clearance_mm,
            classes,
            obstacles,
            nets,
            planes,
            solid_pads: {
                solid_pads.sort();
                solid_pads
            },
            isolated_pads: {
                isolated_pads.sort();
                isolated_pads
            },
            // In obstacle order already.
            solid_pad_obstacles,
            isolated_pad_obstacles,
            pad_spokes,
            merged_pads,
            no_via_pads: {
                no_via_pads.sort_unstable();
                no_via_pads
            },
            fill_passes,
            pin_row_gaps,
            terminal_copper: Vec::new(),
            attachment: Default::default(),
        },
        expressions: 0,
    };
    // A panel of different boards: a net per sub-board where no copper
    // joins them (`sub_boards::split_router_nets`).
    if config.sub_board_nets != Some(false)
        && let Some(pieces) = crate::sub_boards::Pieces::of_loops_cached(&loops.piece_loops, &loops.cutouts)
    {
        let split = crate::sub_boards::split_router_nets(&mut lowered.board, &pieces);
        if std::env::var_os("PCB_ROUTER_SUB_BOARD_DEBUG").is_some() {
            eprintln!(
                "sub-boards: {} pieces, {} nets split ({} more nets), kept whole: {:?}",
                pieces.count(),
                split.nets.len(),
                split.nets.iter().map(|(_, count)| count - 1).sum::<usize>(),
                split.joined
            );
        }
    }
    Ok(lowered)
}

/// Pads of a pin row lie at most this many times the closer one's nearest
/// neighbour apart (score-kicad-board's pin-row crossings, aesthetics.rs).
const PIN_ROW_ADJACENT: f64 = 1.25;

/// Adjacent pads farther apart than this (centre to centre, mm) are no pin
/// row to the router's pin-row cost: a track between them is ordinary.
const PIN_ROW_PITCH_MM: f64 = 1.5;

/// The gaps between adjacent SMD pads of one footprint (`pads`: centre,
/// copper layers, net), per copper layer with three pads or more: the
/// pairs whose centres lie at most `PIN_ROW_ADJACENT` times the closer
/// pad's nearest-neighbour distance and `PIN_ROW_PITCH_MM` apart, as
/// score-kicad-board's crossing measure pairs them.
pub(crate) fn footprint_pin_row_gaps(
    pads: &[([f64; 2], core::LayerMask, Option<core::NetId>)],
) -> Vec<core::PinRowGap> {
    let mut gaps = Vec::new();
    let layers = pads.iter().fold(0, |union, pad| union | pad.1);
    let distance = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
    for layer in 0..core::LayerMask::BITS as usize {
        if layers & (1 << layer) == 0 {
            continue;
        }
        let members: Vec<_> = pads
            .iter()
            .filter(|pad| pad.1 & (1 << layer) != 0)
            .collect();
        if members.len() < 3 {
            continue;
        }
        let nearest: Vec<f64> = members
            .iter()
            .map(|a| {
                members
                    .iter()
                    .map(|b| distance(a.0, b.0))
                    .filter(|d| *d > 1.0e-6)
                    .fold(f64::INFINITY, f64::min)
            })
            .collect();
        for (i, a) in members.iter().enumerate() {
            for (j, b) in members.iter().enumerate().skip(i + 1) {
                let gap = distance(a.0, b.0);
                if gap < 1.0e-6
                    || gap > PIN_ROW_PITCH_MM
                    || gap > PIN_ROW_ADJACENT * nearest[i].min(nearest[j])
                {
                    continue;
                }
                gaps.push(core::PinRowGap {
                    layer,
                    a: a.0,
                    b: b.0,
                    nets: [a.2, b.2],
                });
            }
        }
    }
    gaps
}

/// Whether a footprint is a touch electrode: its only pad, of a custom
/// shape, is copper art no part is soldered to. d20_pcb's J2 is a face of
/// the die, enclosed by the footprint's own frame on F.Cu and by a keepout
/// on B.Cu, and under the footprint's mask opening; the designer reaches
/// it with a via in its corner. Its pad holds its net's vias (not in
/// `no_via_pads`) and its own mask opening leaves them there.
fn touch_electrode(footprint: &Expr) -> bool {
    let mut pads = footprint
        .children()
        .iter()
        .filter(|child| child.head() == Some("pad"));
    matches!(
        (pads.next(), pads.next()),
        (Some(pad), None) if pad.children().get(3).and_then(Expr::atom) == Some("custom")
    )
}

/// A footprint's pad with copper, for `exposed_pad` (the lowering's and
/// score-kicad-board's).
pub(crate) struct PadBox {
    /// Its number (empty for none), without quotes.
    pub number: String,
    pub center: [f64; 2],
    pub smd: bool,
    /// Its copper's box (the pad's own shape): minimum and maximum.
    pub bounds: [[f64; 2]; 2],
}

/// SMD pads of one number whose boxes lie within this of each other are
/// one pad to `exposed_pad`: an exposed pad drawn on both sides (F.Cu and
/// B.Cu copies at one place, thermal vias between) or split into pieces.
const SAME_PAD_GAP: f64 = 0.5;

/// The footprint's exposed pad (a QFN's, a DFN's, an SOIC's thermal pad),
/// if it has one, as the indices of its pads: the largest SMD pad, at
/// least twice any other SMD pad's area, its centre strictly inside the
/// box of the centres of at least three other pads. SMD pads of one
/// number that touch or nearly do (`SAME_PAD_GAP`) count as one pad of
/// their boxes' box: a two-sided exposed pad (Sisu's U2.11, two 1.2 x 2
/// mm copies at one place) and one split into pieces are each one
/// candidate, its copies not "another pad" of its size. A via there is
/// the designer's way out of a pad the pins enclose (thermal vias, as
/// KiCad's own footprints have them, and a large pad keeps its solder);
/// a tab beside a row of pins (SOT-223, D-PAK) is not one: a track
/// reaches it from outside.
pub(crate) fn exposed_pad(pads: &[PadBox]) -> Vec<usize> {
    if pads.len() < 4 {
        return Vec::new();
    }
    // Union-find over the SMD pads of one number within the gap.
    let mut parent: Vec<usize> = (0..pads.len()).collect();
    fn root(parent: &mut [usize], mut index: usize) -> usize {
        while parent[index] != index {
            parent[index] = parent[parent[index]];
            index = parent[index];
        }
        index
    }
    let mut by_number: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, pad) in pads.iter().enumerate() {
        if pad.smd && !pad.number.is_empty() {
            by_number.entry(&pad.number).or_default().push(index);
        }
    }
    for members in by_number.values() {
        for (position, &a) in members.iter().enumerate() {
            for &b in &members[position + 1..] {
                let near = (0..2).all(|axis| {
                    pads[a].bounds[0][axis] <= pads[b].bounds[1][axis] + SAME_PAD_GAP
                        && pads[b].bounds[0][axis] <= pads[a].bounds[1][axis] + SAME_PAD_GAP
                });
                if near {
                    let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
                    parent[ra.max(rb)] = ra.min(rb);
                }
            }
        }
    }
    // Each SMD group's box, by its root.
    let mut groups: BTreeMap<usize, [[f64; 2]; 2]> = BTreeMap::new();
    for (index, pad) in pads.iter().enumerate().filter(|(_, pad)| pad.smd) {
        let group = root(&mut parent, index);
        let bounds = groups.entry(group).or_insert(pad.bounds);
        *bounds = [
            [
                bounds[0][0].min(pad.bounds[0][0]),
                bounds[0][1].min(pad.bounds[0][1]),
            ],
            [
                bounds[1][0].max(pad.bounds[1][0]),
                bounds[1][1].max(pad.bounds[1][1]),
            ],
        ];
    }
    let area =
        |bounds: &[[f64; 2]; 2]| (bounds[1][0] - bounds[0][0]) * (bounds[1][1] - bounds[0][1]);
    let Some((&largest, bounds)) = groups.iter().max_by(|a, b| area(a.1).total_cmp(&area(b.1)))
    else {
        return Vec::new();
    };
    let largest_area = area(bounds);
    if groups
        .iter()
        .any(|(group, other)| *group != largest && 2.0 * area(other) > largest_area)
    {
        return Vec::new();
    }
    let members: Vec<usize> = (0..pads.len())
        .filter(|index| pads[*index].smd && root(&mut parent, *index) == largest)
        .collect();
    let others: Vec<&PadBox> = pads
        .iter()
        .enumerate()
        .filter(|(index, _)| !members.contains(index))
        .map(|(_, other)| other)
        .collect();
    if others.len() < 3 {
        return Vec::new();
    }
    let center = [
        (bounds[0][0] + bounds[1][0]) / 2.0,
        (bounds[0][1] + bounds[1][1]) / 2.0,
    ];
    let inside = (0..2).all(|axis| {
        let (low, high) = others
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), other| {
                (low.min(other.center[axis]), high.max(other.center[axis]))
            });
        low < center[axis] && center[axis] < high
    });
    if inside { members } else { Vec::new() }
}

/// How far beyond a track's reach a net tie's copper is cut back from its
/// pads' centres: room for the lattice node that ends the track (half the
/// coarsest pitch and more).
const NET_TIE_MARGIN: f64 = 0.1;

/// The pad numbers a footprint's net tie joins, by group: KiCad 7's
/// `(net_tie_pad_groups "1, 2" ...)`, or for a board before version
/// 20220817 a footprint whose keywords start with "net tie", all its pads
/// in one group (KiCad 10.0.7 loads ghoul's KiCad 6 NetTie-2_SMD_Pad0.2mm
/// so; kicad-cli: a tie by `(tags "net tie")` at 20220815 and before, by
/// "net tie short" too, not by "Net Tie", "foo net tie" or a description,
/// and not at 20220817 and after).
fn net_tie_groups(footprint: &Expr, board_version: Option<u64>) -> Vec<Vec<String>> {
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
fn carve_net_tie(shape: core::Shape, carves: &[([f64; 2], f64)]) -> Option<core::Shape> {
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
        let bounds = self::shape(&custom_pad_local_geometry(pad)?).aabb();
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

fn net_ids_lookup(nets: &[core::Net], name: &str) -> Option<core::NetId> {
    nets.iter()
        .position(|net| net.name == name)
        .map(|index| index as core::NetId)
}

/// A board graphic on copper, placed once the nets of the pads are known.
struct CopperGraphic {
    shapes: Vec<core::Shape>,
    layers: core::LayerMask,
    /// Its routed net, by name.
    net: Option<String>,
    kind: String,
    /// The clearance of its class (its net's, else Default's), kept where
    /// it has no routed net.
    clearance: f64,
}

/// The layer names of an item with either a `layer` or a `layers` form.
/// What the lowering leaves out of the board without an error, one line
/// each, for the report's `warnings`: copper on a layer the board's stack
/// lacks (a typo, `In3.Cu` on a two-layer board: KiCad warns when it
/// loads such a board, the router routes through it) and text boxes of
/// fewer than three corners (no box to keep away from).
pub(crate) fn input_warnings(pcb: &Expr) -> Vec<String> {
    let Ok(layers) = LayerTable::from_pcb(pcb) else {
        return Vec::new();
    };
    // (kind, layer) -> (count, the first's position).
    type Seen = (usize, Option<[f64; 2]>);
    let mut unknown: BTreeMap<(&str, &str), Seen> = BTreeMap::new();
    let mut boxes = Vec::new();
    for item in pcb.children() {
        let kind = match item.head() {
            Some(kind @ ("segment" | "arc" | "via" | "zone")) => kind,
            Some("footprint") => {
                let reference = footprint_reference(item).unwrap_or_default();
                for text in item
                    .children()
                    .iter()
                    .filter(|child| child.head() == Some("fp_text_box"))
                {
                    if !geometry::text_hidden(text)
                        && matches!(geometry::text_box_shapes(text), Ok(shapes) if shapes.is_empty())
                    {
                        boxes.push(format!(
                            "{reference}'s text box has fewer than three corners: it is left out"
                        ));
                    }
                }
                continue;
            }
            Some("gr_text_box") => {
                if !geometry::text_hidden(item)
                    && matches!(geometry::text_box_shapes(item), Ok(shapes) if shapes.is_empty())
                {
                    boxes.push(
                        "a board text box has fewer than three corners: it is left out".into(),
                    );
                }
                continue;
            }
            _ => continue,
        };
        for name in item_layer_names(item) {
            if name.ends_with(".Cu")
                && !matches!(name, "*.Cu" | "F&B.Cu")
                && layers.index(name).is_none()
            {
                let at = form_xy(item, "start").or_else(|_| form_xy(item, "at")).ok();
                let entry = unknown.entry((kind, name)).or_insert((0, at));
                entry.0 += 1;
            }
        }
    }
    let stack = layers.names.join(", ");
    let mut warnings: Vec<String> = unknown
        .into_iter()
        .map(|((kind, layer), (count, at))| {
            format!(
                "{count} {kind}{} on {layer}{}, which the board's copper layers ({stack}) do not include: the router ignores {} there",
                if count == 1 { "" } else { "s" },
                at.map_or(String::new(), |at| format!(" (the first at {:.3}, {:.3})", at[0], at[1])),
                if count == 1 { "it" } else { "them" }
            )
        })
        .collect();
    warnings.extend(boxes);
    warnings
}

fn item_layer_names(item: &Expr) -> Vec<&str> {
    match form_atom(item, "layer", 1) {
        Some(name) => vec![name],
        None => item
            .child("layers")
            .map(Expr::children)
            .unwrap_or_default()
            .iter()
            .skip(1)
            .filter_map(Expr::atom)
            .collect(),
    }
}

/// The outer copper layers whose solder mask `names` open (F.Mask: the
/// front, B.Mask: the back).
fn mask_sides(names: &[&str], layer_count: usize) -> Vec<usize> {
    names
        .iter()
        .filter_map(|name| match *name {
            "F.Mask" => Some(0),
            "B.Mask" => Some(layer_count - 1),
            _ => None,
        })
        .collect()
}

/// A graphic on the Margin layer as KiCad 10.0.7's copper_edge_clearance
/// sees it: board edge, with its stroke (only Edge.Cuts strokes count as
/// lines of no width) and its fill, which copper on every layer keeps the
/// edge clearance from (the zone filler knocks it out too).
/// `copper_graphic_shapes` already holds an arc's chord error.
fn margin_obstacles(
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
        blocks_tracks: true,
        blocks_vias: true,
        label: label.clone(),
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
