// Copyright (C) 2026 Toit contributors.

//! Lowering a KiCad board to the placer's problem.
//!
//! `lower_placement` runs the stages in order, each on a read-only
//! `LowerContext` (the board, the settings and the facts read from the
//! board once) and a `ProblemBuilder` (the problem and what each part's
//! index carries beside it): the footprints as parts (`footprint`), the
//! board's obstacles after them (`obstacles`), the nets' weights and the
//! board-wide constraints, the parts held fixed and the user's
//! constraints (`holds`), the footprints' rule areas, the automatic
//! constraints (decoupling, the router's rules in `router_rules`, the
//! pieces and copies in `pieces`). The order of parts, pins and nets is
//! the placer's input: its result and the placement digest depend on it.

use super::*;
use pcb_placer as core;

mod footprint;
mod holds;
mod obstacles;
mod pieces;
mod router_rules;
#[cfg(test)]
mod tests;

pub use self::holds::fixed_on_marks_warning;
pub(in crate::board_placer) use self::pieces::keep_on_pieces;
pub(crate) use self::router_rules::length_rule_report;

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

pub struct LoweredPlacement {
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
pub struct LoweredLengthRule {
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
pub fn glob_matches(pattern: &str, text: &str) -> bool {
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

/// What every stage reads and none changes: the board, the placer's
/// settings, and the facts read from the board once.
pub(super) struct LowerContext<'a> {
    pub pcb: &'a Expr,
    pub config: &'a KiCadBoardPlacerConfig,
    /// The board's net facts (`note_net_names`).
    pub net_names: crate::kicad_nets::NetNames,
    pub loops: outline::BoardLoops,
    /// The user's constraints, where the config holds them inline.
    pub user_constraints: Option<&'a crate::placement_constraints::KiCadPlacementConstraints>,
    /// The nets with copper pours.
    pub pour_nets: BTreeSet<String>,
    /// Copper layers signals can use (`LoweredPlacement::routing_layers`).
    pub routing_layers: f64,
    /// How far a mask opening reaches beyond the bodies' spacing
    /// (`mask_opening_growth`).
    pub mask_growth: f64,
    /// The board's mask expansion and minimum web (`far_mask_reach`).
    pub board_mask_expansion: f64,
    pub mask_web: f64,
    pub copper_clearance: f64,
}

impl<'a> LowerContext<'a> {
    /// Reads the board's net names (noting them on this thread), loops,
    /// pours, routing layers and mask settings.
    fn new(pcb: &'a Expr, config: &'a KiCadBoardPlacerConfig) -> Result<Self, String> {
        // The board's net facts; noted on this thread too, for the placer's
        // other readers of the board (`note_net_names`).
        let net_names = crate::note_net_names(pcb);
        let loops = outline::board_loops(pcb)?;
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
        let user_constraints = match &config.constraints {
            Some(KiCadConstraintsSource::Inline(constraints)) => Some(constraints),
            _ => None,
        };
        Ok(LowerContext {
            pcb,
            config,
            net_names,
            loops,
            user_constraints,
            pour_nets,
            routing_layers,
            mask_growth,
            board_mask_expansion,
            mask_web,
            copper_clearance: config.copper_clearance_mm.unwrap_or(0.2),
        })
    }

    /// The board's footprints, in its order: the problem's first parts.
    pub fn footprints(&self) -> impl Iterator<Item = &'a Expr> + 'a {
        let pcb: &'a Expr = self.pcb;
        pcb.children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
    }
}

/// Per footprint (the problem's first parts, in the board's order): what
/// the constraints, the pair rows and the router's rules look up by
/// footprint.
#[derive(Default)]
pub(super) struct Footprints {
    /// Each footprint's uuid: entries for a repeated reference name their
    /// footprint by it.
    pub uuids: Vec<Option<String>>,
    /// Whether it is a through-hole part of four pins or more, held where
    /// it is near the edge.
    pub connector: Vec<bool>,
    /// Every pad by name, for constraints that point at a pin.
    pub pad_offsets: Vec<BTreeMap<String, [f64; 2]>>,
    /// Every pad with a net (number, net, centre) and the footprint's
    /// library name, for the pair rows (`pair_rows`).
    pub pad_nets: Vec<Vec<PadNet>>,
    pub library_names: Vec<String>,
    pub pad_boxes: Vec<Vec<[f64; 4]>>,
    /// The footprint's own keepout zones (an antenna), for overhang
    /// constraints.
    pub keepout_boxes: Vec<Option<[f64; 4]>>,
    pub mouths: Vec<Option<[f64; 2]>>,
}

impl Footprints {
    /// The number of footprints.
    pub fn count(&self) -> usize {
        self.uuids.len()
    }
}

/// The problem as the stages build it, and per part (footprints first,
/// then obstacles) its reference, its input pose and its lock.
pub(super) struct ProblemBuilder {
    pub problem: core::Problem,
    /// The nets by name, numbered in the order their first pin is met.
    pub net_ids: BTreeMap<String, usize>,
    pub references: Vec<String>,
    pub source_at: Vec<[f64; 3]>,
    pub locked: Vec<bool>,
    pub footprints: Footprints,
}

impl ProblemBuilder {
    /// A problem of no parts and no nets on the board's outline and
    /// pieces, with the config's spacing, grid and margins.
    fn new(context: &LowerContext) -> ProblemBuilder {
        let config = context.config;
        ProblemBuilder {
            problem: core::Problem {
                outline: context.loops.outline.clone(),
                components: Vec::new(),
                net_weights: Vec::new(),
                poses: Vec::new(),
                spacing: config.spacing_mm,
                grid: config.grid_mm,
                edge_margin: config.edge_margin_mm,
                min_spacing: config.copper_clearance_mm.unwrap_or(0.2),
                far_side_pads_only: false,
                escape: None,
                crossings: None,
                pieces: context.loops.pieces.clone(),
                constraints: Default::default(),
            },
            net_ids: BTreeMap::new(),
            references: Vec::new(),
            source_at: Vec::new(),
            locked: Vec::new(),
            footprints: Footprints::default(),
        }
    }

    /// Adds a lowered footprint as the next part.
    fn push_footprint(&mut self, lowered: footprint::LoweredFootprint) {
        self.problem.components.push(lowered.component);
        self.problem.poses.push(lowered.pose);
        self.references.push(lowered.reference);
        self.source_at.push(lowered.at);
        self.locked.push(lowered.locked);
        let footprints = &mut self.footprints;
        footprints.uuids.push(lowered.uuid);
        footprints.connector.push(lowered.connector);
        footprints.pad_offsets.push(lowered.pad_offsets);
        footprints.pad_nets.push(lowered.pad_nets);
        footprints.library_names.push(lowered.library_name);
        footprints.pad_boxes.push(lowered.pad_boxes);
        footprints.keepout_boxes.push(lowered.keepout_box);
        footprints.mouths.push(lowered.mouth);
    }

    /// Adds a fixed, locked obstacle of the board as the next part, under
    /// `reference` and without an input pose.
    fn push_obstacle(&mut self, component: core::Component, pose: core::Pose, reference: &str) {
        self.problem.components.push(component);
        self.problem.poses.push(pose);
        self.references.push(reference.into());
        self.source_at.push([0.0; 3]);
        self.locked.push(true);
    }
}

/// Lowers a board and the placer's settings (with the constraints resolved)
/// to the placer's problem, through the stages the module's doc lists.
pub fn lower_placement(
    pcb: &Expr,
    config: &KiCadBoardPlacerConfig,
) -> Result<LoweredPlacement, String> {
    let context = LowerContext::new(pcb, config)?;
    let mut builder = ProblemBuilder::new(&context);
    for footprint in context.footprints() {
        let lowered = footprint::lower_footprint(&context, footprint, &mut builder.net_ids)?;
        builder.push_footprint(lowered);
    }
    let keepouts = obstacles::add_board_rule_areas(&context, &mut builder)?;
    obstacles::add_copper_graphics(&context, &mut builder)?;
    let marks = obstacles::add_board_marks(&context, &mut builder)?;
    let cutouts = obstacles::add_cutouts(&context, &mut builder);
    builder.problem.net_weights = net_weights(&builder.net_ids, &builder.problem.components);
    board_constraints(&context, &mut builder.problem, &marks, cutouts.solid);
    holds::fix_default_parts(&context, &mut builder, &keepouts);
    let (pair_rows, mut constraint_warnings) =
        holds::apply_user_constraints(&context, &mut builder)?;
    holds::carry_slots(&mut builder, &cutouts.owned);
    // A polygon outline: parts held at an edge keep to the nearest side of
    // the polygon that faces it.
    if context
        .user_constraints
        .and_then(|constraints| constraints.outline.as_ref())
        .is_some_and(|outline| outline.polygonal())
    {
        builder.problem.constraints.sides =
            core::constraints::outline_sides(&context.loops.piece_loops);
    }
    let overlap_warnings = holds::fixed_overlap_warnings(&builder);
    let (attached, attached_report) = holds::attach_marks_to_parts(&context, &mut builder, &marks);
    let on_marks_warnings = holds::fixed_on_marks_warnings(&builder, &marks);
    obstacles::add_footprint_rule_areas(&context, &mut builder)?;
    add_decoupling(&context, &mut builder.problem)?;
    let (array_rooms, length_rules) = router_rules::apply_router_rules(&context, &mut builder);
    let footprint_count = builder.footprints.count();
    if config.keep_on_pieces {
        keep_on_pieces(&mut builder.problem, footprint_count);
    }
    if config.keep_copies {
        pieces::keep_copies_apart(&mut builder.problem, footprint_count);
    }
    builder.problem.constraints.link_pairs();
    constraint_warnings.extend(overlap_warnings);
    constraint_warnings.extend(on_marks_warnings);
    let poured = poured_nets(
        &context.pour_nets,
        &builder.net_ids,
        builder.problem.net_weights.len(),
    );
    let net_names = names_by_id(&builder.net_ids);
    let pair_row_parts =
        super::lowered_pair_rows(&pair_rows, &builder.references, &builder.net_ids);
    // Each pair row turns to face its pair's two ends after the
    // refinement (`pcb_placer::facing`).
    builder.problem.constraints.facing = pair_row_parts
        .iter()
        .filter_map(|row| row.facing.clone())
        .collect();
    Ok(LoweredPlacement {
        problem: builder.problem,
        net_names,
        references: builder.references,
        source_at: builder.source_at,
        constraint_warnings,
        routing_layers: context.routing_layers,
        poured,
        attached_marks: attached,
        attached_report,
        array_rooms,
        pair_rows: pair_row_parts,
        length_rules,
    })
}

/// Each net's weight in the placer's wire cost, by id: `net_weight` of its
/// pins on `components`, three times that for a net designers keep short.
fn net_weights(net_ids: &BTreeMap<String, usize>, components: &[core::Component]) -> Vec<f64> {
    let mut pin_counts = vec![0usize; net_ids.len()];
    for component in components {
        for pin in &component.pins {
            pin_counts[pin.net] += 1;
        }
    }
    // Large nets (power) would otherwise dominate and collapse the layout
    // (`net_weight`).
    // Nets designers keep short count three times: a switcher's switch
    // node, feedback and bootstrap, RF and antenna feeds, crystal pins (by
    // the pin function KiCad puts in the net's name, "Net-(U9-FB)"; Sisu's
    // feedback divider ended 46 mm from its regulator, its RF feed 52 mm
    // long).
    let mut names = vec![""; net_ids.len()];
    for (name, id) in net_ids {
        names[*id] = name.as_str();
    }
    pin_counts
        .iter()
        .zip(&names)
        .map(|(pins, name)| {
            let weight = net_weight(*pins);
            if keep_short(name) {
                3.0 * weight
            } else {
                weight
            }
        })
        .collect()
}

/// The board-wide constraints: the copper's edge clearance, the mask gap,
/// the solid cutouts (`solid_cutouts`), and how the board's marks
/// (`marks`) yield, let silkscreen pass and keep no halo.
fn board_constraints(
    context: &LowerContext,
    problem: &mut core::Problem,
    marks: &[(usize, BoardMark)],
    solid_cutouts: Vec<usize>,
) {
    let config = context.config;
    // KiCad reports copper touching the edge even where the clearance is
    // zero (hackrf's SMA connector P4, its pads flush with the outline: 5
    // copper_edge_clearance findings; 0.01 mm off it, none).
    // 0.01 mm off it, none: the floor; above it the project's clearance
    // (the molecules' 0.01 mm, their edge connectors' copper 0.012 mm
    // from the edge: KiCad finds nothing).
    problem.constraints.copper_edge = config.copper_edge_clearance_mm.unwrap_or(0.0).max(0.01);
    problem.constraints.pad_mask_gap = pad_mask_gap(context.pcb);
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
}

/// Adds the decoupling relations (`auto_decoupling`): each capacitor near
/// the supply pins of its rail; near relations follow their anchors with
/// `decoupling_near`.
fn add_decoupling(context: &LowerContext, problem: &mut core::Problem) -> Result<(), String> {
    let config = context.config;
    // Decoupling at the pins (`decoupling_near`): near relations follow
    // their anchors in the legaliser, and each capacitor is near the
    // nearest of every supply pin of its rail, at the tighter tolerance.
    problem.constraints.follow_anchors = config.decoupling_near;
    if config.auto_decoupling {
        let automatic = decoupling_relations(
            context.pcb,
            problem,
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
    Ok(())
}

/// Per net id below `count`: whether a pour carries it.
fn poured_nets(
    pour_nets: &BTreeSet<String>,
    net_ids: &BTreeMap<String, usize>,
    count: usize,
) -> Vec<bool> {
    let mut poured = vec![false; count];
    for (name, id) in net_ids {
        if *id < poured.len() && pour_nets.contains(name.as_str()) {
            poured[*id] = true;
        }
    }
    poured
}

/// The nets' names by id.
fn names_by_id(net_ids: &BTreeMap<String, usize>) -> Vec<String> {
    let mut net_names_by_id = vec![String::new(); net_ids.len()];
    for (name, id) in net_ids {
        net_names_by_id[*id] = name.clone();
    }
    net_names_by_id
}
