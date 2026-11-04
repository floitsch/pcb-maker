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

/// Whether a net (by its raw name) connects parts: a named net, not an
/// `unconnected-(...)` one. Such a net's pads are one pin's (a mounting
/// hole's pads of one number, a reversible footprint's copies), all on one
/// part, even where the router joins them (`NetNames::routable`), so they
/// pull no part towards another. (The placer read the router's jumper
/// nets from whatever board its thread had lowered before, usually none.)
pub(super) fn placer_net(raw: &str) -> bool {
    !raw.is_empty()
        && !raw
            .rsplit('/')
            .next()
            .unwrap_or(raw)
            .starts_with("unconnected-(")
}

pub(crate) struct LoweredPlacement {
    pub problem: core::Problem,
    /// The problem's nets' names, by id (as the board names them).
    pub net_names: Vec<String>,
    pub references: Vec<String>,
    pub source_at: Vec<[f64; 3]>,
    pub constraint_warnings: Vec<String>,
    /// Copper layers signals can use (the board's layers less the planes
    /// a multilayer stack keeps): what the routing demand spreads over.
    pub routing_layers: f64,
    /// Per net of the problem: whether a pour carries it.
    pub poured: Vec<bool>,
    /// The board's marks that belong to a movable part (`attach_marks`):
    /// (the mark's place in `board_marks`, the part); no obstacles.
    pub attached_marks: Vec<(usize, usize)>,
    /// The same, said: the mark, its layer and anchor, and the part.
    pub attached_report: Vec<String>,
    /// The pair rows (`pair_rows`): each one's first part and the other,
    /// its facing and its pitch's detail.
    pub pair_rows: Vec<super::LoweredPairRow>,
    /// The room kept around grid arrays (`bga_room`), said: per array its
    /// bands and the parts they let in or leave alone.
    pub array_rooms: Vec<String>,
    /// The nets of `length` rules the placer holds (`length_maxima`,
    /// `length_minima`): per net its window and its relations' range in
    /// `problem.constraints.relations` (board-placer.json's
    /// `length_rules`).
    pub length_rules: Vec<LoweredLengthRule>,
}

/// A net of a `length` rule as the placer holds it (`length_windows`).
#[derive(Clone, Debug, Default)]
pub(crate) struct LoweredLengthRule {
    pub net: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// The share of the minimum its pins keep apart (0: none), and of the
    /// maximum they keep within (infinite: none).
    pub floor: f64,
    pub soft: f64,
    /// The parts its pins lie on, by index.
    pub parts: Vec<usize>,
    /// Its relations in `problem.constraints.relations`.
    pub relations: std::ops::Range<usize>,
    /// Why the placer holds less than the rule (a floor on a net of more
    /// than two parts, both parts fixed, too many pins).
    pub note: String,
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
/// The warning for a fixed part on marks of the input (`marks`: each
/// "text \"+\" (F.SilkS at 62.13, 66.20)"), the first three named.
pub(crate) fn fixed_on_marks_warning(reference: &str, marks: &[String]) -> String {
    let shown = 3;
    match marks {
        [mark] => format!(
            "fixed part {reference} lies on the board's {mark}: both stay where the input put them"
        ),
        _ => format!(
            "fixed part {reference} lies on {} of the board's texts and silkscreen graphics ({}{}): all stay where the input put them",
            marks.len(),
            marks[..marks.len().min(shown)].join("; "),
            if marks.len() > shown {
                format!("; {} more", marks.len() - shown)
            } else {
                String::new()
            }
        ),
    }
}

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
        far_open: Default::default(),
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
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
        courtless: Vec::new(),
        slots: Vec::new(),
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
    // The board's net facts; noted on this thread too, for the placer's
    // other readers of the board (`note_net_names`).
    let net_names = crate::note_net_names(pcb);
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
    // Every pad with a net (number, net, centre) and the footprint's
    // library name, for the pair rows (`pair_rows`).
    let mut pad_nets: Vec<Vec<super::PadNet>> = Vec::new();
    let mut library_names: Vec<String> = Vec::new();
    let mut pad_boxes: Vec<Vec<[f64; 4]>> = Vec::new();
    let mut keepout_boxes: Vec<Option<[f64; 4]>> = Vec::new();
    let mut mouths: Vec<Option<[f64; 2]>> = Vec::new();
    // The nets with copper pours, and the layers signals can route on: a
    // four-layer stack keeps one plane, six or more two (what the
    // harvested designers do).
    let layer_table = crate::board_router::LayerTable::from_pcb(pcb)?;
    let pour_nets: BTreeSet<String> = crate::board_router::pours(pcb, &layer_table)?
        .into_iter()
        .map(|pour| net_names.normalize(&pour.net).to_string())
        .collect();
    let copper_layers = layer_table.len();
    let routing_layers = match copper_layers {
        0..=2 => 2.0,
        3..=5 => (copper_layers - 1) as f64,
        _ => (copper_layers - 2) as f64,
    };
    let mask_growth = mask_opening_growth(pcb, config);
    // The board's mask expansion and minimum web: a pad's mask opening on
    // the side its part is not on keeps other parts' openings the web
    // apart there (`far_mask_reach`).
    let mask_setup = |key: &str| {
        pcb.child("setup")
            .and_then(|setup| form_f64(setup, key, 1).ok())
            .unwrap_or(0.0)
    };
    let board_mask_expansion = mask_setup("pad_to_mask_clearance");
    let mask_web = mask_setup("solder_mask_min_width").max(0.0);
    let copper_clearance = config.copper_clearance_mm.unwrap_or(0.2);
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
        // The non-plated holes with the clearance ring they keep from
        // other copper (`keep_away`).
        let mut hole_rings: Vec<[f64; 4]> = Vec::new();
        let mut pin_centers: Vec<[f64; 2]> = Vec::new();
        let mut named = BTreeMap::new();
        let mut own_pad_nets = Vec::new();
        // The copper layer of the side the part is not on: an edge-mount
        // connector's SMD pads there occupy that side too.
        let other_copper = if back { "F.Cu" } else { "B.Cu" };
        let other_mask = if back { "F.Mask" } else { "B.Mask" };
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
            if keep_away > 0.0 {
                hole_rings.push(pad_box);
            }
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
                // Its mask opening there, where it reaches past the copper
                // clearance: other parts' pads on that side keep it the
                // web plus their expansion away (vaio_re's J1, a Pi Zero
                // socket on the front: its 2.3 mm mounting holes open the
                // mask 1.625 mm around on both sides, and a TSOT-23 and an
                // 0805 on the back under it bridged five times).
                let reach =
                    far_mask_reach(pad, footprint, other_mask, board_mask_expansion, mask_web)
                        - copper_clearance;
                if reach > keep_away + 1.0e-9 {
                    let grow = reach - keep_away;
                    far_copper.push([
                        pad_box[0] - grow,
                        pad_box[1] - grow,
                        pad_box[2] + grow,
                        pad_box[3] + grow,
                    ]);
                }
            }
            let Some(net) = node_net(pad)
                .filter(|raw| placer_net(raw))
                .map(|raw| net_names.normalize(raw))
            else {
                continue;
            };
            let next = net_ids.len();
            let pad_at = form_at(pad)?;
            if let Some(name) = pad.children().get(1).and_then(Expr::atom) {
                own_pad_nets.push((
                    name.to_string(),
                    net.to_string(),
                    [pad_at[0], pad_at[1]],
                    pad_box,
                ));
            }
            let net = *net_ids.entry(net.to_string()).or_insert(next);
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
        // The footprint's copper graphics on any copper layer (KiCad 9's
        // `(layers "B.Cu" "B.Mask")` too): copper the edge and cutouts
        // keep their clearance from (`Component::copper_graphics`).
        let mut copper_graphics = Vec::new();
        for child in footprint.children() {
            let on_copper = form_atom(child, "layer", 1)
                .is_some_and(|layer| layer.ends_with(".Cu"))
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
        // Without a courtyard, the body a courtyard would give the part:
        // its pads, copper and mask graphics and its fabrication outline
        // (else its silkscreen outline) on its side; KiCad's courtyard
        // clearance is 0 (no board of the benchmark sets a
        // `courtyard_clearance` rule). Two such parts may not overlap them
        // (`Component::courtless`): Goga64 ULK's PG1316 switches have no
        // courtyard, and a thumb row lay on the grid's last row with its
        // pads between theirs.
        let courtless_body = if courtless {
            let outline_boxes = |suffix: &str| -> Result<Vec<[f64; 4]>, String> {
                let mut boxes = Vec::new();
                for child in footprint.children() {
                    if form_atom(child, "layer", 1).is_some_and(|layer| {
                        layer.starts_with(own_prefix) && layer.ends_with(suffix)
                    }) && let Some(graphic) = graphic_box(child)?
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
            own_pads
                .iter()
                .chain(&own_graphic_boxes)
                .chain(&outline)
                .copied()
                .reduce(|a, b| {
                    [
                        a[0].min(b[0]),
                        a[1].min(b[1]),
                        a[2].max(b[2]),
                        a[3].max(b[3]),
                    ]
                })
        } else {
            None
        };
        // The footprint's own closed Edge.Cuts loops (a switch's peg
        // holes): holes that move with it (`Component::slots`).
        let mut slots = Vec::new();
        for child in footprint.children() {
            if matches!(child.head(), Some("fp_circle" | "fp_rect" | "fp_poly"))
                && form_atom(child, "layer", 1) == Some("Edge.Cuts")
                && let Some(graphic) = graphic_box(child)?
            {
                slots.push(graphic);
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
            far_open: Default::default(),
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
            hole_rings: hole_rings.clone(),
            copper_graphics,
            courtless: courtless_body.into_iter().collect(),
            slots,
        });
        poses.push(core::Pose {
            position: [at[0], at[1]],
            angle: at[2],
        });
        references.push(reference);
        uuids.push(footprint_uuid(footprint));
        source_at.push(at);
        pad_offsets.push(named);
        pad_nets.push(own_pad_nets);
        library_names.push(
            footprint
                .children()
                .get(1)
                .and_then(Expr::atom)
                .unwrap_or_default()
                .to_string(),
        );
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

    // The board's own texts and silkscreen graphics the input placed
    // (`marks`): obstacles on their side, as a keepout there.
    let mut marks: Vec<(usize, BoardMark)> = Vec::new();
    if config.keep_board_marks {
        for mark in board_marks(pcb, outline_box(pcb))? {
            let (component, pose) = mark_obstacle(&mark);
            marks.push((components.len(), mark));
            components.push(component);
            poses.push(pose);
            references.push(if marks.last().is_some_and(|(_, mark)| mark.copper) {
                "copper text".into()
            } else {
                "silkscreen mark".into()
            });
            source_at.push([0.0; 3]);
            locked.push(true);
        }
    }

    // Cutouts are board edge too: parts keep the edge margin from them.
    // Those an `outline` constraint cut (the board's own graphics, which
    // it replaced) keep bodies off as well.
    let outline_given = matches!(&config.constraints, Some(KiCadConstraintsSource::Inline(constraints)) if constraints.outline.is_some());
    let mut solid_cutouts = Vec::new();
    // The cutouts that are a footprint's own closed graphic, with that
    // footprint: they go with the part where it moves.
    let mut owned_cutouts: Vec<(usize, usize)> = Vec::new();
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
        if let Some(Some(owner)) = loops.cutout_owners.get(number) {
            owned_cutouts.push((components.len(), *owner));
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
            far_open: Default::default(),
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
            hole_rings: Vec::new(),
            copper_graphics: Vec::new(),
            courtless: Vec::new(),
            slots: Vec::new(),
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
    // Large nets (power) would otherwise dominate and collapse the layout
    // (`net_weight`: how steeply; `steep` by default).
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
            let weight = config
                .net_weight
                .weight(*pins, KiCadNetWeight::named_rail(name));
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
        escape: None,
        crossings: None,
        pieces: loops.pieces.clone(),
        constraints: Default::default(),
    };
    // KiCad reports copper touching the edge even where the clearance is
    // zero (hackrf's SMA connector P4, its pads flush with the outline: 5
    // copper_edge_clearance findings; 0.01 mm off it, none).
    // 0.01 mm off it, none: the floor; above it the project's clearance
    // (the molecules' 0.01 mm, their edge connectors' copper 0.012 mm
    // from the edge: KiCad finds nothing).
    problem.constraints.copper_edge = config.copper_edge_clearance_mm.unwrap_or(0.0).max(0.01);
    problem.constraints.pad_mask_gap = pad_mask_gap(pcb);
    problem.constraints.solid_cutouts = solid_cutouts;
    problem.constraints.yielding = marks.iter().map(|(index, _)| *index).collect();
    problem.constraints.leads_keep_copper = config.holes_in_courtyards_error != Some(true);
    if config.silk_over_parts_error != Some(true) {
        problem.constraints.passable_marks = marks
            .iter()
            .filter(|(_, mark)| !mark.copper)
            .map(|(index, _)| *index)
            .collect();
    }
    problem.constraints.halo_free = marks
        .iter()
        .filter(|(_, mark)| !mark.copper)
        .map(|(index, _)| *index)
        .collect();
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
    // A differential pair's two series parts as a row of two
    // (`pair_rows`): added to the user's rows, where any pair has them.
    let pair_rows = if config.pair_rows_on() {
        implicit_pair_rows(
            &problem,
            &references[..footprint_count],
            &library_names,
            &pad_nets,
            user_constraints,
            &config.pair_geometry,
        )
    } else {
        Vec::new()
    };
    let with_pairs = (!pair_rows.is_empty()).then(|| {
        let mut constraints = user_constraints.cloned().unwrap_or_else(|| {
            crate::placement_constraints::KiCadPlacementConstraints {
                version: 1,
                ..Default::default()
            }
        });
        constraints
            .row
            .extend(pair_rows.iter().map(|pair_row| pair_row.row.clone()));
        constraints
    });
    let constraint_warnings = match (&config.constraints, &with_pairs) {
        (_, Some(constraints)) => apply_constraints(
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
        (Some(KiCadConstraintsSource::Inline(constraints)), None) => apply_constraints(
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
        (None, None) => Vec::new(),
        (Some(KiCadConstraintsSource::File(path)), None) => {
            return Err(format!(
                "constraints file {} was not resolved",
                path.display()
            ));
        }
    };
    // A footprint's own slots (`Component::slots`) move with it: a
    // movable part's slot at its input pose is no hole of the board, and
    // the part carries it. A fixed part's at its input pose stays the
    // cutout it was, and the part carries none (as before: bodies may
    // reach over a held part's slot, copper keeps the edge margin from
    // it).
    let leader_of = |problem: &core::Problem, part: usize| {
        problem
            .constraints
            .followers
            .iter()
            .find(|follower| follower.part == part)
            .map_or(part, |follower| follower.leader)
    };
    // A fixed part a `place` entry put elsewhere carries its slots too.
    let carried = |problem: &core::Problem, part: usize| {
        let leader = leader_of(problem, part);
        let pose = problem.poses[leader];
        let at = source_at[leader];
        !problem.components[leader].fixed
            || (pose.position[0] - at[0]).abs() > 1.0e-6
            || (pose.position[1] - at[1]).abs() > 1.0e-6
            || (pose.angle - at[2]).abs() > 1.0e-6
    };
    for &(cutout, owner) in &owned_cutouts {
        if owner < footprint_count && carried(&problem, owner) {
            problem.components[cutout].side = core::Side::Neither;
        }
    }
    for index in 0..footprint_count {
        if !carried(&problem, index) {
            problem.components[index].slots.clear();
        }
    }
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
                // A hollow part's body admits other parts (`hollow`:
                // framework_mobo_lefthalf's J1 around M1's pads); the
                // input check judges their pads (`held_parts_overlapping`).
                if !ca.side.collides(cb.side) || !ca.hollow.is_empty() || !cb.hollow.is_empty() {
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
    // A mark on a movable part at its input position belongs to that part
    // (a connector's pin names inside its courtyard): no obstacle. Taken
    // from the input once (`attached_marks` in the config carries it to
    // the lowerings of the placed board).
    let attached = match &config.attached_marks {
        Some(attached) => attached.clone(),
        None => attach_marks(&problem, footprint_count, &marks),
    };
    let mut attached_report = Vec::new();
    for &(place, part) in &attached {
        let Some((index, mark)) = marks.get(place) else {
            continue;
        };
        problem.components[*index].side = core::Side::Neither;
        problem.constraints.yielding.retain(|other| other != index);
        problem.constraints.halo_free.retain(|other| other != index);
        attached_report.push(format!("{}: {}", mark.key(), references[part]));
    }
    // A fixed part on a mark of the input (a header's pin names inside its
    // courtyard, a test pad under its name): both stay as the input put
    // them; said, not refused.
    // One warning per fixed part, however many marks it lies on (the
    // OpenESC panel's two frames lay on 20 texts and graphics: twenty
    // lines ahead of the one that mattered).
    let mut on_marks: Vec<(usize, Vec<String>)> = Vec::new();
    for &(index, ref mark) in &marks {
        let mark_side = problem.components[index].side;
        for (part, _) in references.iter().enumerate().take(footprint_count) {
            let component = &problem.components[part];
            if !component.fixed || !component.side.collides(mark_side) {
                continue;
            }
            let pose = problem.poses[part];
            let (center, half) = (component.center(pose), component.half_extent(pose.angle));
            let body = [
                center[0] - half[0],
                center[1] - half[1],
                center[0] + half[0],
                center[1] + half[1],
            ];
            if mark
                .boxes
                .iter()
                .any(|b| b[0] < body[2] && body[0] < b[2] && b[1] < body[3] && body[1] < b[3])
            {
                let what = format!(
                    "{} ({} at {:.2}, {:.2})",
                    mark.what, mark.layer, mark.at[0], mark.at[1]
                );
                match on_marks.iter_mut().find(|(other, _)| *other == part) {
                    Some((_, marks)) => marks.push(what),
                    None => on_marks.push((part, vec![what])),
                }
            }
        }
    }
    for (part, marks) in on_marks {
        stacked_warnings.push(fixed_on_marks_warning(&references[part], &marks));
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
    // Decoupling at the pins (`decoupling_near`): near relations follow
    // their anchors in the legaliser, and each capacitor is near the
    // nearest of every supply pin of its rail, at the tighter tolerance.
    problem.constraints.follow_anchors = config.decoupling_near;
    if config.auto_decoupling {
        let automatic = decoupling_relations(
            pcb,
            &problem,
            config.decoupling_tolerance(),
            config.decoupling_near,
        )?;
        if !automatic.is_empty() {
            if problem.constraints.relation_weight == 0.0 {
                problem.constraints.relation_weight = config.constraint_weight;
            }
            problem.constraints.automatic = automatic.len();
            problem.constraints.relations.extend(automatic);
        }
    }
    let mut array_rooms = Vec::new();
    let mut length_rules = Vec::new();
    if let Some(rules) = &config.router_rules {
        if config.via_in_pad_far_side {
            far_side_of_vias_in_pads(pcb, rules, config, &pad_offsets, &mut problem);
            far_side_of_fanouts(pcb, rules, config, &pad_offsets, &mut problem);
        }
        if config.bga_room {
            array_rooms = room_around_arrays(pcb, rules, &pad_offsets, &references, &mut problem);
        }
        if config.length_maxima || config.length_minima {
            length_rules = length_windows(rules, config, &net_ids, footprint_count, &mut problem);
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
    let mut net_names_by_id = vec![String::new(); net_ids.len()];
    for (name, id) in &net_ids {
        net_names_by_id[*id] = name.clone();
    }
    let pair_row_parts = super::lowered_pair_rows(&pair_rows, &references, &net_ids);
    // Each pair row turns to face its pair's two ends after the
    // refinement (`pcb_placer::facing`).
    problem.constraints.facing = pair_row_parts
        .iter()
        .filter_map(|row| row.facing.clone())
        .collect();
    Ok(LoweredPlacement {
        problem,
        net_names: net_names_by_id,
        references,
        source_at,
        constraint_warnings,
        routing_layers,
        poured,
        attached_marks: attached,
        attached_report,
        array_rooms,
        pair_rows: pair_row_parts,
        length_rules,
    })
}

/// The far side under the vias the router's via-in-pad rule puts in
/// admitted footprints' enclosed pads (`far_side_vias`): each via goes
/// through the board, so it joins its footprint's `far_side`, where parts
/// on the other side keep off it as off a through-hole pin (MIDAS-MK1:
/// C112, an 0603 on the back, under U106's B2, whose via the router then
/// refused). The box keeps the via's copper and its drill's hole clearance
/// from the other side's pads where the legality test adds the copper
/// clearance alone (the tight levels and KiCad's legality, which test
/// far-side boxes against pads).
fn far_side_of_vias_in_pads(
    pcb: &Expr,
    rules: &KiCadBoardRouterConfig,
    config: &KiCadBoardPlacerConfig,
    pad_offsets: &[BTreeMap<String, [f64; 2]>],
    problem: &mut core::Problem,
) {
    let copper = config.copper_clearance_mm.unwrap_or(0.2);
    let hole = config.hole_clearance_mm.unwrap_or(rules.hole_clearance_mm);
    for via in crate::board_router::far_side_vias(pcb, rules) {
        let Some(center) = pad_offsets
            .get(via.footprint)
            .and_then(|pads| pads.get(&via.pad))
        else {
            continue;
        };
        let half = (via.diameter / 2.0).max(via.drill / 2.0 + hole - copper);
        problem.components[via.footprint].far_side.push([
            center[0] - half,
            center[1] - half,
            center[0] + half,
            center[1] + half,
        ]);
    }
}

/// The far side under the dog-bones of the router's prepared fan-outs
/// (`bga_fanout`, `far_side_balls`): each inner ball's via sits half a
/// pitch off along both axes of the footprint, toward the nearer way out
/// (`rings_of`; both ways along an axis where both sides are as near),
/// and goes through the board as a via in a pad does. Nothing with the
/// fan-out off.
fn far_side_of_fanouts(
    pcb: &Expr,
    rules: &KiCadBoardRouterConfig,
    config: &KiCadBoardPlacerConfig,
    pad_offsets: &[BTreeMap<String, [f64; 2]>],
    problem: &mut core::Problem,
) {
    let copper = config.copper_clearance_mm.unwrap_or(0.2);
    let hole = config.hole_clearance_mm.unwrap_or(rules.hole_clearance_mm);
    for array in crate::board_router::far_side_balls(pcb, rules) {
        let Some(offsets) = pad_offsets.get(array.footprint) else {
            continue;
        };
        let balls: Vec<(&str, [f64; 2])> = array
            .balls
            .iter()
            .filter_map(|ball| offsets.get(ball).map(|at| (ball.as_str(), *at)))
            .collect();
        let local: Vec<[f64; 2]> = balls.iter().map(|(_, at)| *at).collect();
        let (_, toward) = crate::board_router::rings_of(&local, array.pitch);
        let half = (array.diameter / 2.0).max(array.drill / 2.0 + hole - copper);
        let signs = |side: f64| {
            if side == 0.0 {
                vec![-1.0, 1.0]
            } else {
                vec![side]
            }
        };
        for (position, (ball, at)) in balls.iter().enumerate() {
            if !array.inner.iter().any(|inner| inner == ball) {
                continue;
            }
            for sx in signs(toward[position][0]) {
                for sy in signs(toward[position][1]) {
                    let center = [
                        at[0] + sx * array.pitch / 2.0,
                        at[1] + sy * array.pitch / 2.0,
                    ];
                    problem.components[array.footprint].far_side.push([
                        center[0] - half,
                        center[1] - half,
                        center[0] + half,
                        center[1] + half,
                    ]);
                }
            }
        }
    }
}

/// The room an array's outer rings' surface escapes take beyond its ball
/// field on its own side (`bga_room`): the surface rings' tracks, a track
/// and a clearance each, turning along the edge, then a row of the class's
/// vias with a clearance on both sides (Glasgow's U30: 2 x 0.3 + 0.6 + 0.3
/// = 1.5 mm; its designer keeps the nearest movable part's pads 1.28 mm
/// from the field and nine in ten balls' channels 1.66 mm free).
pub(super) fn surface_band(room: &crate::board_router::ArrayRoom) -> f64 {
    room.surface_rings as f64 * (room.track + room.clearance) + room.via + 2.0 * room.clearance
}

/// The room beyond the ball field on the far side the via ring's tracks
/// take (`bga_room`): a track and a clearance per ring deeper than the
/// surface rings that holds a ball of a net (Glasgow's U30: rings 2 to 5,
/// 4 x 0.3 = 1.2 mm); none where every ball leaves on the surface, and
/// then the array keeps no room at all.
pub(super) fn far_band(room: &crate::board_router::ArrayRoom) -> f64 {
    (room.deepest + 1).saturating_sub(room.surface_rings) as f64 * (room.track + room.clearance)
}

/// The box `[min x, min y, max x, max y]` of `inner` and `outer`.
fn union_box(inner: [f64; 4], outer: [f64; 4]) -> [f64; 4] {
    [
        inner[0].min(outer[0]),
        inner[1].min(outer[1]),
        inner[2].max(outer[2]),
        inner[3].max(outer[3]),
    ]
}

/// Room around grid arrays (`bga_room`; docs/placer.md, "Room around grid
/// arrays (2026-11-04)"). Per array the router's detection finds
/// (`array_rooms`, the fan-out's `board_arrays`), in its own frame from
/// its ball field (the box of the ball centres grown by half a pitch):
///
/// - its own side: its body (and its tight body) grows to the field and
///   the surface band (`surface_band`), so no part's body comes nearer at
///   any level; the edge inset grows by as much, so the array's copper
///   keeps the edge margin it kept;
/// - the far side: the field and the far band (`far_band`) join its
///   `far_side` boxes, letting in only the array's own decoupling
///   capacitors (`Component::far_open`), which keep off the via sites
///   (the fan-out's and the vias in pads, the boxes before).
///
/// Only an array whose inner balls the fan-out serves with dog-bones in
/// the diagonal gaps (`far_side_balls`, Glasgow's U30) keeps room: one
/// whose inner balls leave by vias in their pads (MIDAS's U106, no gap
/// fits a via; the room cost MIDAS-MK1 its strict pass at one seed of
/// three) or whose every ball leaves on the surface (vd-rd's U200: 3 to 6
/// open) keeps none, nor does any array with the fan-out off.
/// Fixed parts are not moved: a free array keeps its room from them as
/// from any part, and a fixed array's report names the fixed parts in its
/// room. Returns the report lines.
fn room_around_arrays(
    pcb: &Expr,
    rules: &KiCadBoardRouterConfig,
    pad_offsets: &[BTreeMap<String, [f64; 2]>],
    references: &[String],
    problem: &mut core::Problem,
) -> Vec<String> {
    let mut report = Vec::new();
    let dog_boned: BTreeSet<usize> = crate::board_router::far_side_balls(pcb, rules)
        .iter()
        .map(|array| array.footprint)
        .collect();
    for room in crate::board_router::array_rooms(pcb, rules) {
        if !dog_boned.contains(&room.footprint) {
            continue;
        }
        let Some(offsets) = pad_offsets.get(room.footprint) else {
            continue;
        };
        let balls: Vec<[f64; 2]> = room
            .balls
            .iter()
            .filter_map(|ball| offsets.get(ball).copied())
            .collect();
        let Some(centres) = balls
            .iter()
            .map(|at| [at[0], at[1], at[0], at[1]])
            .reduce(union_box)
        else {
            continue;
        };
        let half = room.pitch / 2.0;
        let field = [
            centres[0] - half,
            centres[1] - half,
            centres[2] + half,
            centres[3] + half,
        ];
        let grow = |b: [f64; 4], by: f64| [b[0] - by, b[1] - by, b[2] + by, b[3] + by];
        let (top, far) = (surface_band(&room), far_band(&room));
        // An array whose every ball of a net leaves on the surface (vd-rd's
        // DDR3 BGA-96: each ball faces the outside or the empty middle)
        // has no via ring crowding its outer rings' escapes: no room (the
        // band cost vd-rd 3 connections and 2 findings, measured).
        if far <= 0.0 {
            continue;
        }
        let component = &mut problem.components[room.footprint];
        let body = [
            component.body_center[0] - component.body_size[0] / 2.0,
            component.body_center[1] - component.body_size[1] / 2.0,
            component.body_center[0] + component.body_size[0] / 2.0,
            component.body_center[1] + component.body_size[1] / 2.0,
        ];
        let grown = union_box(body, grow(field, top));
        let least = [
            body[0] - grown[0],
            body[1] - grown[1],
            grown[2] - body[2],
            grown[3] - body[3],
        ]
        .into_iter()
        .fold(f64::INFINITY, f64::min);
        component.body_center = [(grown[0] + grown[2]) / 2.0, (grown[1] + grown[3]) / 2.0];
        component.body_size = [grown[2] - grown[0], grown[3] - grown[1]];
        component.edge_inset += least.max(0.0);
        if let Some(tight) = component.tight {
            component.tight = Some(union_box(tight, grow(field, top)));
        }
        let from = component.far_side.len();
        component.far_side.push(grow(field, far));
        component.far_open = core::FarOpen {
            from,
            parts: room.decoupling.clone(),
        };
        let letting: Vec<String> = room
            .decoupling
            .iter()
            .map(|part| references[*part].clone())
            .collect();
        let mut line = format!(
            "{}: pitch {:.3} mm, surface rings {}, deepest ring {}; own side {:.3} mm, far side {:.3} mm beyond the field",
            references[room.footprint], room.pitch, room.surface_rings, room.deepest, top, far,
        );
        if !letting.is_empty() {
            line += &format!("; under it only its decoupling {}", letting.join(" "));
        }
        let fixed = fixed_in_room(problem, room.footprint, references);
        if !fixed.is_empty() {
            line += &format!("; fixed parts left in its room: {}", fixed.join(" "));
        }
        report.push(line);
    }
    report
}

/// For a fixed array (`room_around_arrays`), the fixed parts whose bodies
/// meet its grown body on its side or its far-side room, at their poses:
/// the room cannot move them.
fn fixed_in_room(problem: &core::Problem, array: usize, references: &[String]) -> Vec<String> {
    if !problem.components[array].fixed {
        return Vec::new();
    }
    // The legality test leaves two fixed parts alone: ask it of the array
    // as if it moved, where it lies on the board.
    let mut probe = problem.clone();
    probe.components[array].fixed = false;
    let at = probe.poses[array];
    if !core::legal::is_legal(&probe, &probe.poses, array, at, std::iter::empty()) {
        return Vec::new();
    }
    (0..problem.components.len())
        .filter(|other| *other != array && problem.components[*other].fixed)
        .filter(|other| {
            !core::legal::is_legal(&probe, &probe.poses, array, at, [*other].into_iter())
        })
        .map(|other| references[other].clone())
        .collect()
}

/// The share of a net's length maximum its pins are kept within before
/// the anneal pays for the distance (`Relation::Within`'s soft bound).
pub(super) const LENGTH_SOFT_SHARE: f64 = 0.8;

/// At most this many pins of a length-bounded net are related pairwise (a
/// bus's nets join two or three pins).
const LENGTH_MOST_PINS: usize = 16;

/// The share of a net's length minimum its pins are kept apart at least
/// (`length_minima`, `Relation::Within`'s floor): the route between two
/// pins is about their octilinear distance, and the meanders on its
/// straight tracks add at most about as much again where a bus runs at
/// its clearance (the firestarter's DVP bus on the designer's placement:
/// docs/placer.md, "Length minima (2026-11-04)").
pub(super) const LENGTH_FLOOR_SHARE: f64 = 0.5;

/// Length windows (`length_maxima`, `length_minima`): for each net of a
/// `length` rule with a maximum (`length_bounded_nets`), every two of its
/// pins on different parts, not both fixed, keep within
/// `LENGTH_SOFT_SHARE` of the maximum octilinearly, and pay steeply beyond
/// it; for each net with a minimum (`length_floor_nets`) whose pins lie on
/// two parts, they keep `LENGTH_FLOOR_SHARE` of the minimum apart at
/// least (`Relation::Within`, automatic relations: not reported as the
/// user's constraints). The part that moves in the relation repair is the
/// movable one, the smaller of two movable. Returns per net what it holds.
fn length_windows(
    rules: &KiCadBoardRouterConfig,
    config: &KiCadBoardPlacerConfig,
    net_ids: &BTreeMap<String, usize>,
    footprint_count: usize,
    problem: &mut core::Problem,
) -> Vec<LoweredLengthRule> {
    let maxima = config
        .length_maxima
        .then(|| crate::board_router::length_bounded_nets(rules))
        .flatten();
    let minima = config
        .length_minima
        .then(|| crate::board_router::length_floor_nets(rules))
        .flatten();
    let mut nets: BTreeSet<&String> = BTreeSet::new();
    nets.extend(maxima.iter().flat_map(|bounded| bounded.keys()));
    nets.extend(minima.iter().flat_map(|floors| floors.keys()));
    let mut report = Vec::new();
    let mut relations = Vec::new();
    let first = problem.constraints.relations.len();
    for net in nets {
        let max = maxima
            .as_ref()
            .and_then(|bounded| bounded.get(net).copied());
        let min = minima.as_ref().and_then(|floors| floors.get(net).copied());
        let Some(&id) = net_ids.get(net) else {
            continue;
        };
        let pins: Vec<(usize, [f64; 2])> = (0..footprint_count)
            .flat_map(|index| {
                problem.components[index]
                    .pins
                    .iter()
                    .filter(move |pin| pin.net == id)
                    .map(move |pin| (index, pin.offset))
            })
            .collect();
        let mut parts: Vec<usize> = pins.iter().map(|(index, _)| *index).collect();
        parts.dedup();
        let mut entry = LoweredLengthRule {
            net: net.clone(),
            min,
            max,
            soft: f64::INFINITY,
            parts: parts.clone(),
            ..LoweredLengthRule::default()
        };
        if pins.len() > LENGTH_MOST_PINS {
            entry.note = format!("{} pins: not held (at most {LENGTH_MOST_PINS})", pins.len());
            entry.relations = first + relations.len()..first + relations.len();
            report.push(entry);
            continue;
        }
        let (soft, max_bound) = max.map_or((f64::INFINITY, f64::INFINITY), |max| {
            (LENGTH_SOFT_SHARE * max, max)
        });
        let floor = match min {
            Some(min) if parts.len() == 2 => (LENGTH_FLOOR_SHARE * min).min(soft),
            Some(_) => {
                entry.note = format!(
                    "its pins lie on {} parts: the minimum is not held apart (two parts only)",
                    parts.len()
                );
                0.0
            }
            None => 0.0,
        };
        entry.soft = soft;
        entry.floor = floor;
        let start = first + relations.len();
        if max.is_some() || floor > 0.0 {
            for (position, &(a, pin_a)) in pins.iter().enumerate() {
                for &(b, pin_b) in &pins[position + 1..] {
                    let (fixed_a, fixed_b) =
                        (problem.components[a].fixed, problem.components[b].fixed);
                    if a == b || (fixed_a && fixed_b) {
                        continue;
                    }
                    let area = |index: usize| {
                        let size = problem.components[index].body_size;
                        size[0] * size[1]
                    };
                    let first = fixed_b || (!fixed_a && area(a) <= area(b));
                    let ((part, pin), (other, other_pin)) = if first {
                        ((a, pin_a), (b, pin_b))
                    } else {
                        ((b, pin_b), (a, pin_a))
                    };
                    relations.push(core::constraints::Relation::Within {
                        part,
                        pin,
                        other,
                        other_pin,
                        soft,
                        max: max_bound,
                        floor,
                    });
                }
            }
        }
        entry.relations = start..first + relations.len();
        if entry.relations.is_empty() && entry.note.is_empty() && parts.len() >= 2 {
            entry.note = "its parts are fixed: nothing to hold".into();
        }
        report.push(entry);
    }
    if relations.is_empty() {
        return report;
    }
    if problem.constraints.relation_weight == 0.0 {
        problem.constraints.relation_weight = config.constraint_weight;
    }
    problem.constraints.automatic += relations.len();
    problem.constraints.relations.extend(relations);
    report
}

/// board-placer.json's `length_rules`: per net the placer held
/// (`LoweredPlacement::length_rules`), its window and its pins' nearest
/// and farthest octilinear distance across parts at `poses`.
pub(crate) fn length_rule_report(
    lowered: &LoweredPlacement,
    poses: &[core::Pose],
) -> Vec<super::KiCadLengthRuleReport> {
    let problem = &lowered.problem;
    let round = |value: f64| (value * 1000.0).round() / 1000.0;
    lowered
        .length_rules
        .iter()
        .map(|rule| {
            let id = lowered.net_names.iter().position(|name| *name == rule.net);
            let pins: Vec<(usize, [f64; 2])> = rule
                .parts
                .iter()
                .flat_map(|&index| {
                    let component = &problem.components[index];
                    component
                        .pins
                        .iter()
                        .filter(move |pin| Some(pin.net) == id)
                        .map(move |pin| (index, component.pin_position(pin, poses[index])))
                })
                .collect();
            let (mut closest, mut farthest) = (f64::INFINITY, 0.0f64);
            for (position, &(a, at_a)) in pins.iter().enumerate() {
                for &(b, at_b) in &pins[position + 1..] {
                    if a != b {
                        let distance = core::constraints::octilinear(at_a, at_b);
                        closest = closest.min(distance);
                        farthest = farthest.max(distance);
                    }
                }
            }
            if !closest.is_finite() {
                closest = 0.0;
            }
            let held_apart = rule.floor > 0.0 && !rule.relations.is_empty();
            let held_within = rule.max.is_some() && !rule.relations.is_empty();
            let held = match (held_apart, held_within) {
                (true, true) => "both",
                (true, false) => "apart",
                (false, true) => "together",
                (false, false) => "none",
            };
            let kept = (!held_apart || closest + 1.0e-6 >= rule.floor)
                && (!held_within || farthest <= rule.soft + 1.0e-6);
            let mut note = rule.note.clone();
            if held_apart && !kept && closest + 1.0e-6 < rule.floor {
                note = format!(
                    "its pins lie {:.2} mm apart, closer than half the minimum ({:.2} mm): the route and its meanders may not make up the minimum (give the parts room, or an apart entry, docs/constraints.md)",
                    closest, rule.floor
                );
            } else if held_within && !kept && rule.max.is_some_and(|max| farthest > max) {
                note = format!(
                    "its pins lie {farthest:.2} mm apart, beyond the maximum: no route of the net meets it"
                );
            }
            super::KiCadLengthRuleReport {
                net: rule.net.clone(),
                min_mm: rule.min,
                max_mm: rule.max,
                apart_mm: held_apart.then(|| round(rule.floor)),
                within_mm: held_within.then(|| round(rule.soft)),
                parts: rule
                    .parts
                    .iter()
                    .map(|&index| lowered.references[index].clone())
                    .collect(),
                closest_mm: round(closest),
                farthest_mm: round(farthest),
                held: held.into(),
                kept,
                note,
            }
        })
        .collect()
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
