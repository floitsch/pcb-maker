// Copyright (C) 2026 Toit contributors.

//! Lowering a KiCad board to the placer's problem.

use super::*;
use pcb_placer as core;

/// Whether a net's name marks it as one designers keep short: a token
/// (between non-alphanumerics) naming a switcher's switch node, feedback or
/// bootstrap, an RF or antenna feed, or a crystal pin.
pub(super) fn keep_short(name: &str) -> bool {
    const TOKENS: [&str; 14] = [
        "SW", "LX", "FB", "VFB", "BST", "BOOT", "RF", "ANT", "XTAL", "XIN", "XOUT", "XI", "XO",
        "OSC",
    ];
    name.split(|character: char| !character.is_ascii_alphanumeric())
        .any(|token| {
            TOKENS
                .iter()
                .any(|wanted| token.eq_ignore_ascii_case(wanted))
        })
}

/// Whether a net (by its raw name) connects parts; see `routable_net`.
pub(super) fn placer_net(raw: &str) -> bool {
    board_router::routable_net(raw)
}

pub(crate) struct LoweredPlacement {
    pub problem: core::Problem,
    pub references: Vec<String>,
    pub source_at: Vec<[f64; 3]>,
    pub constraint_warnings: Vec<String>,
    /// Copper layers signals can use (the board's layers less the planes
    /// a multilayer stack keeps): what the routing demand spreads over.
    pub routing_layers: f64,
    /// Per net of the problem: whether a pour carries it.
    pub poured: Vec<bool>,
}

/// `*` matches any run of characters, `?` one character, and a backslash
/// makes the next character literal (`REF\\*\\*` is KiCad's unannotated
/// `REF**` itself).
pub(crate) fn glob_matches(pattern: &str, text: &str) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum Token {
        Any,
        One,
        Literal(char),
    }
    let mut tokens = Vec::new();
    let mut characters = pattern.chars();
    while let Some(character) = characters.next() {
        tokens.push(match character {
            '*' => Token::Any,
            '?' => Token::One,
            '\\' => Token::Literal(characters.next().unwrap_or('\\')),
            other => Token::Literal(other),
        });
    }
    let text: Vec<char> = text.chars().collect();
    let mut table = vec![vec![false; text.len() + 1]; tokens.len() + 1];
    table[0][0] = true;
    for p in 1..=tokens.len() {
        if tokens[p - 1] == Token::Any {
            table[p][0] = table[p - 1][0];
        }
        for t in 1..=text.len() {
            table[p][t] = match tokens[p - 1] {
                Token::Any => table[p - 1][t] || table[p][t - 1],
                Token::One => table[p - 1][t - 1],
                Token::Literal(c) => table[p - 1][t - 1] && c == text[t - 1],
            };
        }
    }
    table[tokens.len()][text.len()]
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
        name: "keepout".into(),
        body_center: [0.0, 0.0],
        body_size: [
            bounds.maximum[0] - bounds.minimum[0],
            bounds.maximum[1] - bounds.minimum[1],
        ],
        round: false,
        halo: 0.0,
        pins: Vec::new(),
        side,
        fixed: true,
        angle_options: vec![0.0],
        far_side: Vec::new(),
        far_copper: Vec::new(),
        far_courtyard: Vec::new(),
        hollow: Vec::new(),
        tight: None,
        edge_inset: 0.0,
        courtyards: Vec::new(),
        holes_inside: false,
        pads: Vec::new(),
        copper_only: !bodies,
        cutout_outline: if bodies { Vec::new() } else { points },
        tight_hollow: Vec::new(),
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

/// On a board of several pieces (two boards side by side), no copper
/// joins one piece to another: a net lies on one piece. The parts a net
/// joins (directly or through other parts) stay on the piece of the fixed
/// parts among them, as a region constraint: zpn_devboard's movable parts
/// spread over its second piece, which holds one connector, and left 129
/// connections open after the first route (the designer's placement: 1).
pub(super) fn keep_on_pieces(problem: &mut core::Problem, footprints: usize) {
    if problem.pieces.len() < 2 {
        return;
    }
    // Parts joined by nets, by union-find.
    let mut parent: Vec<usize> = (0..footprints).collect();
    fn root(parent: &mut [usize], mut part: usize) -> usize {
        while parent[part] != part {
            parent[part] = parent[parent[part]];
            part = parent[part];
        }
        part
    }
    let mut first_on_net: BTreeMap<usize, usize> = BTreeMap::new();
    for part in 0..footprints {
        for pin in &problem.components[part].pins {
            match first_on_net.get(&pin.net) {
                Some(&other) => {
                    let (a, b) = (root(&mut parent, part), root(&mut parent, other));
                    parent[a] = b;
                }
                None => {
                    first_on_net.insert(pin.net, part);
                }
            }
        }
    }
    // The pieces the fixed parts of each group sit on.
    let piece_of = |problem: &core::Problem, part: usize| {
        let center = problem.components[part].center(problem.poses[part]);
        problem.pieces.iter().position(|b| {
            center[0] >= b[0] && center[0] <= b[2] && center[1] >= b[1] && center[1] <= b[3]
        })
    };
    let mut anchors: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for part in 0..footprints {
        if problem.components[part].fixed && !problem.components[part].pins.is_empty() {
            let group = root(&mut parent, part);
            if let Some(piece) = piece_of(problem, part) {
                anchors.entry(group).or_default().insert(piece);
            }
        }
    }
    if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
        eprintln!(
            "pieces: {} pieces, net groups anchored on them {:?}",
            problem.pieces.len(),
            anchors
                .values()
                .map(|pieces| pieces.len())
                .collect::<Vec<_>>()
        );
    }
    let constrained: BTreeSet<usize> = problem
        .constraints
        .regions
        .iter()
        .map(|(part, _)| *part)
        .collect();
    for part in 0..footprints {
        if problem.components[part].fixed
            || problem.components[part].pins.is_empty()
            || constrained.contains(&part)
        {
            continue;
        }
        let group = root(&mut parent, part);
        if let Some(pieces) = anchors.get(&group)
            && pieces.len() == 1
        {
            let piece = problem.pieces[*pieces.iter().next().expect("one piece")];
            problem.constraints.regions.push((part, piece));
        }
    }
}

/// A panel of copies joined by tabs is one outline piece, so
/// `keep_on_pieces` cannot tell the copies apart; their nets can: each
/// copy's parts form a group no net leaves (KiKit names a copy's nets
/// `Board_N-...`), and the copies' groups hold the same references. Where
/// two or more such groups each have fixed parts and the boxes of those
/// do not overlap, every movable part of a copy stays in its copy's
/// region: the board's box, cut between the copy's fixed parts and every
/// other copy's, on the axis they are farther apart (halfway, moved by
/// half the difference of the board's margins beyond all copies' fixed
/// parts), and starts there (global placement begins at its centre). On the
/// 0xCB pcb-panel (four copies, one piece) 15-22 parts of one copy landed
/// on another's area. Boards whose nets form one group (most boards, and
/// panels whose copies share net names) are left alone. Returns the
/// number of copies confined.
fn keep_copies_apart(problem: &mut core::Problem, footprints: usize) -> usize {
    if problem.pieces.len() >= 2 || footprints < 4 {
        return 0;
    }
    let mut parent: Vec<usize> = (0..footprints).collect();
    fn root(parent: &mut [usize], mut part: usize) -> usize {
        while parent[part] != part {
            parent[part] = parent[parent[part]];
            part = parent[part];
        }
        part
    }
    let mut first_on_net: BTreeMap<usize, usize> = BTreeMap::new();
    for part in 0..footprints {
        for pin in &problem.components[part].pins {
            match first_on_net.get(&pin.net) {
                Some(&other) => {
                    let (a, b) = (root(&mut parent, part), root(&mut parent, other));
                    parent[a] = b;
                }
                None => {
                    first_on_net.insert(pin.net, part);
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for part in 0..footprints {
        if !problem.components[part].pins.is_empty() {
            groups
                .entry(root(&mut parent, part))
                .or_default()
                .push(part);
        }
    }
    if groups.len() < 2 {
        return 0;
    }
    // A copy: a group whose references (with pin counts) another group
    // repeats.
    let signature = |members: &[usize]| -> Vec<(String, usize)> {
        let mut parts: Vec<(String, usize)> = members
            .iter()
            .map(|part| {
                (
                    problem.components[*part].name.clone(),
                    problem.components[*part].pins.len(),
                )
            })
            .collect();
        parts.sort();
        parts
    };
    let mut kinds: BTreeMap<Vec<(String, usize)>, usize> = BTreeMap::new();
    for members in groups.values() {
        *kinds.entry(signature(members)).or_default() += 1;
    }
    let body_box = |problem: &core::Problem, part: usize| {
        let component = &problem.components[part];
        let (center, half) = (
            component.center(problem.poses[part]),
            component.half_extent(problem.poses[part].angle),
        );
        [
            center[0] - half[0],
            center[1] - half[1],
            center[0] + half[0],
            center[1] + half[1],
        ]
    };
    // Each copy's fixed parts' box, and its movable parts.
    let mut copies: Vec<([f64; 4], Vec<usize>)> = Vec::new();
    for members in groups.values() {
        if kinds[&signature(members)] < 2 {
            continue;
        }
        let mut anchor = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        let mut movable = Vec::new();
        for &part in members {
            if problem.components[part].fixed {
                let b = body_box(problem, part);
                anchor = [
                    anchor[0].min(b[0]),
                    anchor[1].min(b[1]),
                    anchor[2].max(b[2]),
                    anchor[3].max(b[3]),
                ];
            } else {
                movable.push(part);
            }
        }
        if anchor[0].is_finite() && !movable.is_empty() {
            copies.push((anchor, movable));
        }
    }
    if copies.len() < 2 {
        return 0;
    }
    // The copies' fixed parts must keep apart: interleaved copies have no
    // region of their own.
    let gap = |a: &[f64; 4], b: &[f64; 4]| -> [f64; 2] {
        [
            (b[0] - a[2]).max(a[0] - b[2]),
            (b[1] - a[3]).max(a[1] - b[3]),
        ]
    };
    for (index, (a, _)) in copies.iter().enumerate() {
        for (b, _) in &copies[index + 1..] {
            let [x, y] = gap(a, b);
            if x <= 0.0 && y <= 0.0 {
                return 0;
            }
        }
    }
    let bounds = problem.bounds();
    let constrained: BTreeSet<usize> = problem
        .constraints
        .regions
        .iter()
        .map(|(part, _)| *part)
        .collect();
    // The copies repeat one layout, so each copy's region lies around its
    // fixed parts as the board lies around all copies' fixed parts: a cut
    // halfway between two copies' fixed parts moves by half the difference
    // of the board's margins beyond them (kept between the two). Halfway
    // alone cut SmartSpin2k's copies (fixed parts at the top of each copy,
    // 2.3 mm from the board's top edge, 40.5 mm from its bottom) through
    // their own parts: 162 of the designer's 630 movable parts lay outside
    // their copy's region, none with the shift.
    let all = copies.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, (a, _)| {
            [
                b[0].min(a[0]),
                b[1].min(a[1]),
                b[2].max(a[2]),
                b[3].max(a[3]),
            ]
        },
    );
    let shift = [
        ((bounds[2] - all[2]) - (all[0] - bounds[0])) / 2.0,
        ((bounds[3] - all[3]) - (all[1] - bounds[1])) / 2.0,
    ];
    let cut = |before: f64, after: f64, axis: usize| {
        ((before + after) / 2.0 + shift[axis]).clamp(before, after)
    };
    let mut regions = Vec::new();
    for (index, (anchor, _)) in copies.iter().enumerate() {
        let mut region = bounds;
        for (other, (theirs, _)) in copies.iter().enumerate() {
            if other == index {
                continue;
            }
            let [x, y] = gap(anchor, theirs);
            if x >= y {
                if theirs[0] >= anchor[2] {
                    region[2] = region[2].min(cut(anchor[2], theirs[0], 0));
                } else {
                    region[0] = region[0].max(cut(theirs[2], anchor[0], 0));
                }
            } else if theirs[1] >= anchor[3] {
                region[3] = region[3].min(cut(anchor[3], theirs[1], 1));
            } else {
                region[1] = region[1].max(cut(theirs[3], anchor[1], 1));
            }
        }
        regions.push(region);
    }
    // Each copy's parts start in their own region (its centre): from the
    // common start (a netlist's stack, the fixed pins' centroid) global
    // placement clamped the other copies' parts against their regions'
    // near borders, where they piled up (SmartSpin2k: 57 parts without a
    // legal place).
    for ((_, movable), region) in copies.iter().zip(&regions) {
        let centre = [(region[0] + region[2]) / 2.0, (region[1] + region[3]) / 2.0];
        for &part in movable {
            if !constrained.contains(&part) {
                problem.constraints.regions.push((part, *region));
                problem.constraints.starts.push((part, centre));
            }
        }
    }
    if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
        eprintln!(
            "copies: {} copies held apart ({} movable parts), regions {:?}",
            copies.len(),
            copies
                .iter()
                .map(|(_, movable)| movable.len())
                .sum::<usize>(),
            regions
                .iter()
                .map(|r| r.map(|v| (v * 10.0).round() / 10.0))
                .collect::<Vec<_>>()
        );
    }
    copies.len()
}

pub(crate) fn lower_placement(
    pcb: &Expr,
    config: &KiCadBoardPlacerConfig,
) -> Result<LoweredPlacement, String> {
    crate::note_net_names(pcb);
    let loops = outline::board_loops(pcb)?;
    let mut net_ids = BTreeMap::<String, usize>::new();
    let mut components = Vec::new();
    let mut poses = Vec::new();
    let mut references = Vec::new();
    let mut source_at = Vec::new();
    let mut locked = Vec::new();
    // Each footprint's uuid: entries for a repeated reference name their
    // footprint by it.
    let mut uuids = Vec::new();
    let mut connector = Vec::new();
    // Every pad by name, for constraints that point at a pin, and the
    // footprint's own keepout zones (an antenna), for overhang constraints.
    let mut pad_offsets: Vec<BTreeMap<String, [f64; 2]>> = Vec::new();
    let mut pad_boxes: Vec<Vec<[f64; 4]>> = Vec::new();
    let mut keepout_boxes: Vec<Option<[f64; 4]>> = Vec::new();
    let mut mouths: Vec<Option<[f64; 2]>> = Vec::new();
    // The nets with copper pours, and the layers signals can route on: a
    // four-layer stack keeps one plane, six or more two (what the
    // harvested designers do).
    let layer_table = crate::board_router::LayerTable::from_pcb(pcb)?;
    let pour_nets: BTreeSet<String> = crate::board_router::pours(pcb, &layer_table)?
        .into_iter()
        .map(|pour| normalize_net(&pour.net).to_string())
        .collect();
    let copper_layers = layer_table.len();
    let routing_layers = match copper_layers {
        0..=2 => 2.0,
        3..=5 => (copper_layers - 1) as f64,
        _ => (copper_layers - 2) as f64,
    };
    let mask_growth = mask_opening_growth(pcb, config);
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let at = form_at(footprint)?;
        let reference = footprint_reference(footprint).unwrap_or_default();
        let (mut body_center, mut body_size, round) = local_body(footprint)?;
        // A footprint with a courtyard on each side (a hot-swap switch: the
        // socket's on the back, the switch's on the front; a reversible
        // part) takes its own side's courtyard as its body and the other's
        // as what it occupies there: KiCad checks courtyards side by side
        // (urchin's back switches claimed the front switch's square on the
        // back too, and found no room among the mounting holes and logos
        // the designer put between them).
        let courtyard_on = |prefix: &str| {
            footprint.children().iter().any(|child| {
                matches!(
                    child.head(),
                    Some("fp_line" | "fp_rect" | "fp_arc" | "fp_circle" | "fp_poly")
                ) && form_atom(child, "layer", 1)
                    .is_some_and(|layer| layer.starts_with(prefix) && layer.ends_with(".CrtYd"))
            })
        };
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
            back != (!smd && !courtyard_on(own) && courtyard_on(other))
        };
        let (own_prefix, other_prefix) = if back { ("B.", "F.") } else { ("F.", "B.") };
        let mut far_courtyard = None;
        if courtyard_on(own_prefix) && courtyard_on(other_prefix) {
            let (center, size, own_round, _) =
                body_on_side(footprint, ".CrtYd", Some(other_prefix))?;
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
        let mut pins = Vec::new();
        let mut through = false;
        let mut far_side = Vec::new();
        let mut far_copper = Vec::new();
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
        let mut own_pads = Vec::new();
        let mut pin_centers: Vec<[f64; 2]> = Vec::new();
        let mut named = BTreeMap::new();
        // The copper layer of the side the part is not on: an edge-mount
        // connector's SMD pads there occupy that side too.
        let other_copper = if back { "F.Cu" } else { "B.Cu" };
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            if let (Some(name), Ok(at)) = (pad.children().get(1).and_then(Expr::atom), form_at(pad))
            {
                named.entry(name.to_string()).or_insert([at[0], at[1]]);
            }
            let pad_type = pad.children().get(2).and_then(Expr::atom).unwrap_or("");
            // The pad (or hole) with the clearance a hole keeps, in the
            // footprint's frame.
            let pad_at = form_at(pad)?;
            let size = pad_extent(pad);
            let (sin, cos) = quarter_sin_cos(-(pad_at[2] - at[2]));
            // A non-plated hole keeps its own clearance, and the board's
            // hole clearance, from other copper: the clearance tests add
            // the copper clearance to the box, so the box takes the rest.
            let keep_away = if pad_type == "np_thru_hole" {
                local_clearance::pad_clearance(pad, footprint)?
                    .max(
                        config.hole_clearance_mm.unwrap_or(0.0)
                            - config.copper_clearance_mm.unwrap_or(0.2),
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
            own_pads.push(pad_box);
            pin_centers.push([pad_at[0], pad_at[1]]);
            let on_other_side = pad.child("layers").is_some_and(|layers| {
                layers
                    .children()
                    .iter()
                    .skip(1)
                    .filter_map(Expr::atom)
                    .any(|layer| layer == other_copper || layer == "F&B.Cu")
            });
            if matches!(pad_type, "thru_hole" | "np_thru_hole") || on_other_side {
                through = true;
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
                let lead =
                    pad_type == "thru_hole" && (leaded_part || (unattributed && drill >= 0.6));
                if lead {
                    far_side.push(pad_box);
                } else {
                    far_copper.push(pad_box);
                    if pad_type != "smd" && holes_error {
                        far_side.push(hole_box);
                    }
                }
            }
            let Some(net) = node_net(pad)
                .filter(|raw| placer_net(raw))
                .map(normalize_net)
            else {
                continue;
            };
            let next = net_ids.len();
            let net = *net_ids.entry(net.to_string()).or_insert(next);
            let pad_at = form_at(pad)?;
            pins.push(core::Pin {
                offset: [pad_at[0], pad_at[1]],
                net,
            });
        }
        // Copper and mask graphics on the outer layers of the side the part
        // is not on occupy that side as well: Castor's LOGO footprint sits
        // on the front with its copper polygon on B.Cu, and a part placed
        // on the back over it was shorted and bridged.
        let (own_side, other_side) = if back {
            (core::Side::Back, core::Side::Front)
        } else {
            (core::Side::Front, core::Side::Back)
        };
        let other_mask = if other_copper == "F.Cu" {
            "F.Mask"
        } else {
            "B.Mask"
        };
        let mut own_graphics = false;
        let mut own_graphic_boxes = Vec::new();
        // The mask openings on the part's side, grown to KiCad's mask
        // margin beyond the bodies' spacing.
        let mut own_mask_boxes: Vec<[f64; 4]> = Vec::new();
        let mut far_graphics = Vec::new();
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
                far_graphics.push(graphic);
            } else if matches!(layer, "F.Cu" | "B.Cu" | "F.Mask" | "B.Mask") {
                own_graphics = true;
                own_graphic_boxes.push(graphic);
                if layer.ends_with(".Mask") {
                    own_mask_boxes.push([
                        graphic[0] - mask_growth,
                        graphic[1] - mask_growth,
                        graphic[2] + mask_growth,
                        graphic[3] + mask_growth,
                    ]);
                }
            }
        }
        // Artwork (a logo) has neither pads nor a courtyard: it occupies the
        // sides its copper and mask graphics are on (its body is their box),
        // and nothing without any, rather than being a 1 mm wall at its
        // origin.
        let artwork = !footprint.children().iter().any(|child| {
            child.head() == Some("pad")
                || (matches!(
                    child.head(),
                    Some("fp_line" | "fp_rect" | "fp_arc" | "fp_circle" | "fp_poly")
                ) && form_atom(child, "layer", 1)
                    .is_some_and(|layer| layer.ends_with(".CrtYd")))
        });
        // A through-hole part sits on its footprint's side; on the other
        // side only its holes and pads (`far_side`) are in the way, and its
        // copper and mask graphics there.
        let side = if artwork {
            match (own_graphics, !far_graphics.is_empty()) {
                (false, false) => core::Side::Neither,
                (true, false) => own_side,
                (false, true) => other_side,
                (true, true) => core::Side::Both,
            }
        } else {
            far_copper.extend(far_graphics.iter().copied());
            far_side.extend(far_courtyard);
            own_side
        };
        let far_graphics = !artwork && (!far_graphics.is_empty() || far_courtyard.is_some());
        let angle_options = if config.keep_rotation {
            vec![at[2]]
        } else {
            let mut angles: Vec<f64> = (0..4)
                .map(|quarter| normalize_angle(at[2] + 90.0 * quarter as f64))
                .collect();
            // A part at an odd angle (a thumb cluster's keys at -30) may
            // also turn square to the board: the placer's bodies are boxes
            // around the turned part, half as large again at 30 degrees
            // (ErgoSNM: two keys found no room).
            if normalize_angle(at[2]) % 90.0 > 1.0e-6 {
                angles.extend([0.0, 90.0, 180.0, 270.0]);
            }
            // In a fixed order, not from the board's angle: the searches
            // walk them in this order (`core::orient`).
            if config.orientation_search {
                core::orient::sort_angles(&mut angles);
            }
            angles
        };
        // How far the copper stays inside the body: the body may come that
        // much closer to the board edge.
        let edge_inset = if own_pads.is_empty() {
            0.0
        } else {
            let mut pads = own_pads.iter().fold(
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
            );
            // Copper graphics of the footprint count as copper too.
            for child in footprint.children() {
                if !form_atom(child, "layer", 1).is_some_and(|layer| layer.ends_with(".Cu")) {
                    continue;
                }
                if let Some(graphic) = graphic_box(child)? {
                    pads = [
                        pads[0].min(graphic[0]),
                        pads[1].min(graphic[1]),
                        pads[2].max(graphic[2]),
                        pads[3].max(graphic[3]),
                    ];
                }
            }
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
        };
        let both_courtyards = courtyard_on(own_prefix) && courtyard_on(other_prefix);
        let mut courtyard =
            courtyard_shapes_on_side(footprint, both_courtyards.then_some(other_prefix))?;
        // A courtyard far from rectangular (a hot-swap socket's outline)
        // blocks with its own shape, as strips, and the pads: ghoul's
        // designer put resistor networks, regulators and mounting holes in
        // the corners of the switches' boxes, outside their courtyards.
        let mut shaped = Vec::new();
        // A footprint without any courtyard blocks with its copper (pads,
        // copper and mask graphics) alone, and holes may lie inside it:
        // KiCad checks its copper, not a box around it (hackclub
        // keyboar_'s Kailh sockets have no courtyard; the designer put
        // every switch's diode between its pads).
        let courtless =
            !courtyard_on("F.") && !courtyard_on("B.") && !own_pads.is_empty() && !artwork;
        // Unless its pads stand in rows of pins: then a body fills their
        // box (`solid_pin_rows`), and the part blocks with that box.
        let solid = courtless && config.solid_pin_rows && pads_in_pin_rows(&pin_centers);
        if solid {
            // The body is that box already, unless copper or mask graphics
            // reach past it.
            if !own_graphic_boxes.is_empty() {
                let pads = own_pads.iter().fold(
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
                );
                shaped = vec![pads];
                shaped.extend(own_graphic_boxes.iter().copied());
            }
        } else if courtless {
            shaped = own_pads.clone();
            shaped.extend(own_graphic_boxes.iter().copied());
        } else if !round && !artwork {
            let slabs = courtyard_slabs(footprint, both_courtyards.then_some(other_prefix))?;
            let area: f64 = slabs.iter().map(|b| (b[2] - b[0]) * (b[3] - b[1])).sum();
            if !slabs.is_empty() && area < 0.75 * body_size[0] * body_size[1] {
                if slabs.len() > 1 && courtyard.0.is_empty() {
                    courtyard.0 = slabs.clone();
                }
                shaped = slabs;
                shaped.extend(own_pads.iter().copied());
                // Its mask openings beside the courtyard block too: they
                // are in the body's box, which this shape stands in for,
                // and another part's pads there bridge the mask
                // (`mask_openings`). Held to the body's box (grown as they
                // are): a stroke's half width past it would widen every
                // neighbour search (`far_reach`).
                if config.mask_openings {
                    let body = [
                        body_center[0] - body_size[0] / 2.0 - mask_growth,
                        body_center[1] - body_size[1] / 2.0 - mask_growth,
                        body_center[0] + body_size[0] / 2.0 + mask_growth,
                        body_center[1] + body_size[1] / 2.0 + mask_growth,
                    ];
                    shaped.extend(own_mask_boxes.iter().map(|opening| {
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
        connector.push(through && pins.len() >= 4);
        locked.push(
            footprint.child("locked").is_some()
                || footprint
                    .children()
                    .iter()
                    .any(|child| child.atom() == Some("locked")),
        );
        components.push(core::Component {
            name: reference.clone(),
            body_center,
            body_size,
            round,
            halo: if let Some(halo) = config.halo_overrides_mm.get(&reference) {
                *halo
            } else if config.pins_per_halo_track > 0.0 {
                ((pins.len() as f64 / config.pins_per_halo_track).ceil() * config.track_pitch_mm)
                    .min(config.maximum_halo_mm)
                    * config.halo_scale
            } else {
                0.0
            },
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
            far_courtyard: if artwork {
                Vec::new()
            } else {
                far_courtyard.into_iter().collect()
            },
            hollow: shaped,
            tight: if config.tight_bodies == Some(true) {
                tight_body(
                    footprint,
                    (courtyard_on(own_prefix) && courtyard_on(other_prefix))
                        .then_some(other_prefix),
                )?
            } else {
                None
            },
            edge_inset,
            courtyards: courtyard.0,
            holes_inside: courtyard.1 || (courtless && !solid),
            pads: own_pads.clone(),
            copper_only: false,
            cutout_outline: Vec::new(),
            tight_hollow: Vec::new(),
        });
        poses.push(core::Pose {
            position: [at[0], at[1]],
            angle: at[2],
        });
        references.push(reference);
        uuids.push(footprint_uuid(footprint));
        source_at.push(at);
        pad_offsets.push(named);
        pad_boxes.push(own_pads);
        keepout_boxes.push(footprint_keepout_box(footprint, at)?);
        mouths.push(connector_mouth(footprint)?);
    }

    // Rule areas that forbid footprints are obstacles too, and a part the
    // designer placed reaching into one (an antenna module at its keepout)
    // is there on purpose and stays. One that allows footprints but forbids
    // pads keeps copper out only (d20's back: KiCad flagged 27 of our pads
    // in it).
    let footprint_count = components.len();
    let mut keepouts: Vec<pcb_router::geometry::Aabb> = Vec::new();
    for item in pcb
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
        components.push(component);
        poses.push(pose);
        references.push("keepout".into());
        source_at.push([0.0; 3]);
        locked.push(true);
    }

    // Copper-layer graphics are part of the board: parts must not be
    // placed on top of them. They follow the footprints, so footprint
    // indices stay aligned with the board file. (Copper and silkscreen
    // texts move off the parts and the copper after placement instead:
    // see `labels`.)
    // KiCad 9 draws a graphic on several layers at once (`layers`):
    // Sisu's GND rectangles on F.Cu and F.Mask.
    let outer_copper = |item: &Expr| -> Vec<usize> {
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
    };
    for (item, layer) in pcb.children().iter().flat_map(|item| {
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
        components.push(core::Component {
            name: "copper graphic".into(),
            body_center: [0.0, 0.0],
            body_size: [
                bounds.maximum[0] - bounds.minimum[0],
                bounds.maximum[1] - bounds.minimum[1],
            ],
            round: false,
            halo: 0.0,
            pins: Vec::new(),
            side: if layer == 0 {
                core::Side::Front
            } else {
                core::Side::Back
            },
            fixed: true,
            angle_options: vec![0.0],
            far_side: Vec::new(),
            far_copper: Vec::new(),
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
        });
        poses.push(core::Pose {
            position: [
                (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
                (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
            ],
            angle: 0.0,
        });
        references.push("copper graphic".into());
        source_at.push([0.0; 3]);
        locked.push(true);
    }

    // Cutouts are board edge too: parts keep the edge margin from them.
    // Those an `outline` constraint cut (the board's own graphics, which
    // it replaced) keep bodies off as well.
    let outline_given = matches!(&config.constraints, Some(KiCadConstraintsSource::Inline(constraints)) if constraints.outline.is_some());
    let mut solid_cutouts = Vec::new();
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
        if outline_given && loops.board_cutouts.get(number).copied().unwrap_or(false) {
            solid_cutouts.push(components.len());
        }
        // The hole itself: the copper keeps the edge margin from it in the
        // legality test, which looser levels bring down to the rules'
        // clearance. Grown here too, the margin counted twice and never
        // relaxed (katia: parts found no room beside its cutouts).
        components.push(core::Component {
            name: "cutout".into(),
            body_center: [0.0, 0.0],
            body_size: [maximum[0] - minimum[0], maximum[1] - minimum[1]],
            round: false,
            halo: 0.0,
            pins: Vec::new(),
            side: core::Side::Both,
            fixed: true,
            angle_options: vec![0.0],
            far_side: Vec::new(),
            far_copper: Vec::new(),
            far_courtyard: Vec::new(),
            hollow: Vec::new(),
            tight: None,
            edge_inset: 0.0,
            courtyards: Vec::new(),
            holes_inside: false,
            pads: Vec::new(),
            copper_only: true,
            cutout_outline: cutout.clone(),
            tight_hollow: Vec::new(),
        });
        poses.push(core::Pose {
            position: [
                (minimum[0] + maximum[0]) / 2.0,
                (minimum[1] + maximum[1]) / 2.0,
            ],
            angle: 0.0,
        });
        references.push("cutout".into());
        source_at.push([0.0; 3]);
        locked.push(true);
    }

    let mut pin_counts = vec![0usize; net_ids.len()];
    for component in &components {
        for pin in &component.pins {
            pin_counts[pin.net] += 1;
        }
    }
    // Large nets (power) would otherwise dominate and collapse the layout.
    // Nets designers keep short count three times: a switcher's switch
    // node, feedback and bootstrap, RF and antenna feeds, crystal pins (by
    // the pin function KiCad puts in the net's name, "Net-(U9-FB)"; Sisu's
    // feedback divider ended 46 mm from its regulator, its RF feed 52 mm
    // long).
    let mut names = vec![""; net_ids.len()];
    for (name, id) in &net_ids {
        names[*id] = name.as_str();
    }
    let net_weights = pin_counts
        .iter()
        .zip(&names)
        .map(|(pins, name)| {
            let weight = (3.0 / (*pins as f64 - 1.0).max(1.0)).min(1.0);
            if keep_short(name) {
                3.0 * weight
            } else {
                weight
            }
        })
        .collect();
    let mut problem = core::Problem {
        outline: loops.outline.clone(),
        components,
        net_weights,
        poses,
        spacing: config.spacing_mm,
        grid: config.grid_mm,
        edge_margin: config.edge_margin_mm,
        min_spacing: config.copper_clearance_mm.unwrap_or(0.2),
        far_side_pads_only: false,
        shared_halos: config.shared_halos,
        pieces: loops.pieces.clone(),
        constraints: Default::default(),
    };
    // KiCad reports copper touching the edge even where the clearance is
    // zero (hackrf's SMA connector P4, its pads flush with the outline: 5
    // copper_edge_clearance findings; 0.01 mm off it, none).
    problem.constraints.copper_edge = config.copper_edge_clearance_mm.unwrap_or(0.0).max(0.02);
    problem.constraints.solid_cutouts = solid_cutouts;
    for index in 0..footprint_count {
        let reference = &references[index];
        let component = &problem.components[index];
        let pose = problem.poses[index];
        let (center, half) = (component.center(pose), component.half_extent(pose.angle));
        let reach = config.edge_keep_mm;
        let in_keepout = keepouts.iter().any(|keepout| {
            center[0] + half[0] + reach > keepout.minimum[0]
                && center[0] - half[0] - reach < keepout.maximum[0]
                && center[1] + half[1] + reach > keepout.minimum[1]
                && center[1] - half[1] - reach < keepout.maximum[1]
        });
        let on_edge = !core::legal::body_inside_outline(&problem, index, problem.poses[index])
            || in_keepout
            || (connector[index]
                && core::legal::body_near_outline(
                    &problem,
                    index,
                    problem.poses[index],
                    config.edge_keep_mm,
                ));
        let default_fixed = locked[index] || problem.components[index].pins.is_empty() || on_edge;
        problem.components[index].fixed = config.fixed.contains(reference)
            || config
                .fixed_patterns
                .iter()
                .any(|pattern| glob_matches(pattern, reference))
            || (default_fixed && !config.free.contains(reference));
    }
    let user_constraints = match &config.constraints {
        Some(KiCadConstraintsSource::Inline(constraints)) => Some(constraints),
        _ => None,
    };
    let constraint_warnings = match &config.constraints {
        Some(KiCadConstraintsSource::Inline(constraints)) => apply_constraints(
            &mut problem,
            &references,
            &pad_offsets,
            &pad_boxes,
            &keepout_boxes,
            &mouths,
            &locked,
            &uuids,
            constraints,
            config.constraint_weight,
        )?,
        None => Vec::new(),
        Some(KiCadConstraintsSource::File(path)) => {
            return Err(format!(
                "constraints file {} was not resolved",
                path.display()
            ));
        }
    };
    // A polygon outline: parts held at an edge keep to the nearest side of
    // the polygon that faces it.
    if user_constraints
        .and_then(|constraints| constraints.outline.as_ref())
        .is_some_and(|outline| outline.polygonal())
    {
        problem.constraints.sides = core::constraints::outline_sides(&loops.piece_loops);
    }
    // Fixed parts on top of each other place and route without a word,
    // and the board comes back with opens: say so (fence issue E). Checked
    // after the constraints, which fix and free parts (move_all frees a
    // stack fresh from a netlist that the default rules had fixed).
    let mut stacked_warnings = Vec::new();
    {
        let fixed: Vec<usize> = (0..footprint_count)
            .filter(|index| {
                problem.components[*index].fixed && !problem.components[*index].pins.is_empty()
            })
            .collect();
        for (a, first) in fixed.iter().enumerate() {
            for second in &fixed[a + 1..] {
                let (ca, cb) = (&problem.components[*first], &problem.components[*second]);
                if !ca.side.collides(cb.side) {
                    continue;
                }
                let (pa, pb) = (problem.poses[*first], problem.poses[*second]);
                let (center_a, half_a) = (ca.center(pa), ca.half_extent(pa.angle));
                let (center_b, half_b) = (cb.center(pb), cb.half_extent(pb.angle));
                let overlap = (0..2).all(|axis| {
                    (center_a[axis] - center_b[axis]).abs() < half_a[axis] + half_b[axis] - 0.05
                });
                if overlap {
                    stacked_warnings.push(format!(
                        "fixed parts {} and {} overlap",
                        references[*first], references[*second]
                    ));
                }
            }
        }
    }
    // A fixed footprint's own rule areas (an antenna module's keepout, in
    // board coordinates) are obstacles like the board's: PixelWave's
    // ESP32 on the back keeps parts out of its antenna area on both sides,
    // and KiCad flagged 50 pads and parts we put there. A movable part's
    // keepout moves with it: it blocks the other side as far-side box, or,
    // where it forbids pads but not footprints, as far-side copper
    // (MokyaLora's U.FL on the back keeps pads out on the front).
    for (index, footprint) in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .enumerate()
    {
        for zone in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("zone"))
        {
            let Some((component, pose, _, bodies)) = rule_area_obstacle(zone)? else {
                continue;
            };
            if problem.components[index].fixed {
                problem.components.push(component);
                problem.poses.push(pose);
                references.push("keepout".into());
                source_at.push([0.0; 3]);
            } else if component
                .side
                .collides(match problem.components[index].side {
                    core::Side::Front => core::Side::Back,
                    core::Side::Back => core::Side::Front,
                    _ => core::Side::Neither,
                })
            {
                let at = source_at[index];
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
                let local = local.iter().fold(
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
                );
                if bodies {
                    problem.components[index].far_side.push(local);
                } else {
                    problem.components[index].far_copper.push(local);
                }
            }
        }
    }
    if config.auto_decoupling {
        let automatic = decoupling_relations(pcb, &problem, config.decoupling_mm)?;
        if !automatic.is_empty() {
            if problem.constraints.relation_weight == 0.0 {
                problem.constraints.relation_weight = config.constraint_weight;
            }
            problem.constraints.automatic = automatic.len();
            problem.constraints.relations.extend(automatic);
        }
    }
    if config.keep_on_pieces {
        keep_on_pieces(&mut problem, footprint_count);
    }
    if config.keep_copies {
        keep_copies_apart(&mut problem, footprint_count);
    }
    problem.constraints.link_pairs();
    let mut constraint_warnings = constraint_warnings;
    constraint_warnings.extend(stacked_warnings);
    let mut poured = vec![false; problem.net_weights.len()];
    for (name, id) in &net_ids {
        if *id < poured.len() && pour_nets.contains(name.as_str()) {
            poured[*id] = true;
        }
    }
    Ok(LoweredPlacement {
        problem,
        references,
        source_at,
        constraint_warnings,
        routing_layers,
        poured,
    })
}
