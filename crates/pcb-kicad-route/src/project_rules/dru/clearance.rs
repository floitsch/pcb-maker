// Copyright (C) 2026 Toit contributors.

//! The clearances of the custom rules: the board's edge, hole and
//! hole-to-hole clearances (one value each), and the clearance between
//! the items of nets, raised per net where a rule is its nets' against
//! anything, else a pair rule of the router (`PairClearance`).

use std::collections::{BTreeMap, BTreeSet};

use super::condition::{Ex, Item, Kind, NetFacts, T3};
use super::lower::{
    BoardFacts, Lowered, Matcher, PairClearance, ROUTED_KINDS, mm, unknown_only_by_area,
};
use super::{Constraint, Rule};

/// Per routed kind A (`ROUTED_KINDS`) and kind B it meets (track, via,
/// SMD pad, plated pad: `other_kinds`), the layers a clearance rule holds
/// on between the items of two groups of nets, one bit per index of
/// `BoardFacts::copper_layers`.
type KindMasks = [[u32; 4]; 2];

/// Lowers an edge, hole or hole-to-hole clearance (at least `value`):
/// the router has one value for the board, raised where the rule
/// matches anything the router lays.
pub(super) fn lower_board_clearance(
    lowered: &mut Lowered,
    rule: &Rule,
    constraint: &Constraint,
    m: &Matcher,
    value: f64,
) {
    let kind = m.kind;
    let board = m.board;
    let narrower = rule.layer.is_some() || rule.condition.is_some();
    let matches = matches_routed(m);
    // The board setup's value for the kind: a minimum at or
    // below it only accepts less than the router keeps.
    let setup = match kind {
        "edge_clearance" => board.board_clearances[0],
        "hole_clearance" => board.board_clearances[1],
        _ => board.board_clearances[2],
    };
    match matches {
        T3::Unknown if value <= setup + 1.0e-9 => lowered.note(
            rule,
            constraint,
            "no_effect",
            format!(
                "{} {} at or below the board setup's {}: the router keeps more wherever the condition ({}) holds",
                kind.replace('_', " "),
                mm(value),
                mm(setup),
                m.opaque()
            ),
        ),
        T3::Unknown => lowered.note(
            rule,
            constraint,
            "unmodelled",
            format!("the condition depends on {}", m.opaque()),
        ),
        T3::False => lowered.note(
            rule,
            constraint,
            "no_effect",
            if m.shadowed_any.get() {
                "a later rule takes everything it matches (KiCad applies the last matching rule)"
                    .into()
            } else {
                "matches nothing the router lays on this board".into()
            },
        ),
        T3::True => {
            let (target, before) = match kind {
                "edge_clearance" => (&mut lowered.edge_clearance, board.board_clearances[0]),
                "hole_clearance" => (&mut lowered.hole_clearance, board.board_clearances[1]),
                _ => (&mut lowered.hole_to_hole, board.board_clearances[2]),
            };
            *target = target.max(value);
            if value <= before + 1.0e-9 {
                lowered.note(
                    rule,
                    constraint,
                    "no_effect",
                    format!(
                        "{} {}: the board setup keeps {}",
                        kind.replace('_', " "),
                        mm(value),
                        mm(before)
                    ),
                );
                return;
            }
            lowered.note(
                rule,
                constraint,
                if narrower { "approximated" } else { "applied" },
                format!(
                    "{} {} for the whole board{}",
                    kind.replace('_', " "),
                    mm(value),
                    if narrower {
                        " (the router has one value; the rule's layer or condition is narrower)"
                    } else {
                        ""
                    }
                ),
            );
        }
    }
}

/// Whether an edge, hole or hole-to-hole clearance rule matches anything
/// the router lays: an item of each group against the edge, or against
/// one of every group (for hole to hole, of its own net too).
fn matches_routed(m: &Matcher) -> T3 {
    let mut matches = T3::False;
    for group in &m.groups {
        let net = m.representative(group);
        let pairs: Vec<(Option<&NetFacts>, bool)> = match m.kind {
            // Holes of the same net count for hole to hole.
            "hole_to_hole" => m
                .groups
                .iter()
                .map(|other| (Some(m.representative(other)), false))
                .chain([(Some(net), true)])
                .collect(),
            "edge_clearance" => vec![(None, false)],
            _ => m
                .groups
                .iter()
                .map(|other| (Some(m.representative(other)), false))
                .collect(),
        };
        for (other, same) in pairs {
            matches = matches.or(m.truth_for(Some(net), other, same));
        }
    }
    matches
}

/// Lowers a clearance between the items of nets (at least `value`):
/// which pairs of groups the rule matches (`group_pairs`), the placer's
/// pad clearance floor (`floor_pad_clearance`), then per net
/// (`raise_clearances`) where the rule is its nets' against anything,
/// else as a pair rule of the router (`lower_pair_rule`).
pub(super) fn lower_net_clearance(
    lowered: &mut Lowered,
    rule: &Rule,
    constraint: &Constraint,
    m: &Matcher,
    value: f64,
) {
    let board = m.board;
    let GroupPairs {
        truth,
        mut matched,
        maybe,
        mut unknown,
    } = group_pairs(m);
    // A minimum that is unknown only by where an area lies is
    // kept wherever it can hold (the pairs and, below, the kinds
    // and the area's layers it may match): never less than KiCad
    // asks, as for every other value (MokyaLora's
    // "TFT_ZONE_2W_Clearance", 0.25 mm between tracks that meet a
    // rule area on In3.Cu: kept between every two tracks there).
    if unknown && unknown_only_by_area(&m.condition) {
        matched.extend(maybe);
        unknown = false;
        lowered.by_area = true;
    }
    floor_pad_clearance(lowered, m, value);
    // A minimum at or below every net's clearance only lets KiCad
    // accept less than the router keeps, whatever its condition
    // matches: no finding comes of it (ducktop2 right_io's eleven
    // intra-footprint rules on `A.Reference`, 0.1 to 0.4 mm under
    // its classes' 0.15 to 0.5, were listed as rules the router does
    // not keep, trial 5).
    let least = board
        .current
        .iter()
        .map(|net| net.clearance_mm)
        .fold(f64::INFINITY, f64::min);
    if unknown && least.is_finite() && value <= least + 1.0e-9 {
        lowered.note(
            rule,
            constraint,
            "no_effect",
            format!(
                "a minimum of {} at or below every net's clearance (the least {}): the router keeps more wherever the condition ({}) holds",
                mm(value),
                mm(least),
                m.opaque()
            ),
        );
        return;
    }
    if unknown {
        lowered.note(
            rule,
            constraint,
            "unmodelled",
            format!("the condition depends on {}", m.opaque()),
        );
        return;
    }
    if matched.is_empty() {
        lowered.note(
            rule,
            constraint,
            "no_effect",
            if m.shadowed_any.get() {
                "a later rule takes every pair it matches (KiCad applies the last matching rule)"
                    .into()
            } else {
                "matches no pair of routed items of two nets on this board".into()
            },
        );
        return;
    }
    let other_kinds = other_kinds(board);
    let tables = kind_tables(m, &other_kinds, &matched);
    let Some(raise) = raised_groups(m, &other_kinds, &truth, &matched, &tables) else {
        lower_pair_rule(lowered, rule, constraint, m, value, &matched, &tables);
        return;
    };
    raise_clearances(lowered, rule, constraint, m, value, &raise);
}

/// Whether two groups (`i <= j`) hold a pair of nets. A group of one net
/// has no pair of its own, but whether it would match itself tells a rule
/// on one net (`A.NetClass == 'HV'`) from one on a pair.
fn eligible(groups: &[Vec<usize>], i: usize, j: usize) -> bool {
    i != j || groups[i].len() >= 2
}

/// Which pairs of groups (`i <= j`) a clearance rule matches.
struct GroupPairs {
    /// The rule's truth for every pair, eligible or not (`eligible`).
    truth: BTreeMap<(usize, usize), T3>,
    /// The eligible pairs it surely matches.
    matched: BTreeSet<(usize, usize)>,
    /// The eligible pairs it may match.
    maybe: BTreeSet<(usize, usize)>,
    /// Whether it may match some eligible pair.
    unknown: bool,
}

/// Clearances: per group pair, whether the rule matches it.
fn group_pairs(m: &Matcher) -> GroupPairs {
    let mut truth = BTreeMap::new();
    let mut matched = BTreeSet::new();
    let mut maybe = BTreeSet::new();
    let mut unknown = false;
    for (i, gi) in m.groups.iter().enumerate() {
        for (j, gj) in m.groups.iter().enumerate().skip(i) {
            let value = m.truth_for(
                Some(m.representative(gi)),
                Some(m.representative(gj)),
                false,
            );
            truth.insert((i, j), value);
            if !eligible(&m.groups, i, j) {
                continue;
            }
            match value {
                T3::True => {
                    matched.insert((i, j));
                }
                T3::Unknown => {
                    unknown = true;
                    maybe.insert((i, j));
                }
                T3::False => {}
            }
        }
    }
    GroupPairs {
        truth,
        matched,
        maybe,
        unknown,
    }
}

/// Pads against pads, which the router never lays but the
/// placer puts side by side: a rule that holds between the pads
/// of every two nets on some layer floors the placer's copper
/// clearance (`pad_clearance_all`; OpenESC's "2 oz outer
/// copper": 0.16 mm on F.Cu and B.Cu over the board's 0.09,
/// whose parts the placer packed 0.09 mm apart). Whether a
/// later rule takes the pads leaves the report alone.
fn floor_pad_clearance(lowered: &mut Lowered, m: &Matcher, value: f64) {
    let groups = &m.groups;
    let pad_kinds_here: Vec<Kind> = [Kind::SmdPad, Kind::PlatedPad]
        .into_iter()
        .filter(|kind| m.board.pad_kinds.contains(kind))
        .collect();
    let shadowed_before = m.shadowed_any.get();
    let pads_match = |a_net: &NetFacts, b_net: &NetFacts| {
        m.layers.iter().any(|&layer| {
            pad_kinds_here.iter().any(|&a_kind| {
                pad_kinds_here.iter().any(|&b_kind| {
                    let a = Item {
                        kind: a_kind,
                        net: Some(a_net),
                        layer,
                    };
                    let b = Item {
                        kind: b_kind,
                        net: Some(b_net),
                        layer,
                    };
                    m.truth(a, Some(b), false) == T3::True
                })
            })
        })
    };
    let pad_pairs: Vec<(usize, usize)> = (0..groups.len())
        .flat_map(|i| (i..groups.len()).map(move |j| (i, j)))
        .filter(|&(i, j)| eligible(groups, i, j))
        .collect();
    if !pad_pairs.is_empty()
        && pad_pairs
            .iter()
            .all(|&(i, j)| pads_match(m.representative(&groups[i]), m.representative(&groups[j])))
    {
        lowered.pad_clearance_all = lowered.pad_clearance_all.max(value);
    }
    m.shadowed_any.set(shadowed_before);
}

/// Every copper layer of the board, one bit per index.
fn all_layers(board: &BoardFacts) -> u32 {
    if board.copper_layers.len() >= 32 {
        u32::MAX
    } else {
        (1u32 << board.copper_layers.len()) - 1
    }
}

/// The router's kinds of item B (pcb_router::rules::TRACK, VIA,
/// PAD, PLATED), each with the kinds of the board it holds: SMD and
/// plated pads apart, as KiCad's `isPlated()` tells them (until
/// 2026-11-07 one kind, the rule from both: the laptop motherboard's
/// "Pad to Track clearance" walled its SMD balls).
fn other_kinds(board: &BoardFacts) -> [Vec<Kind>; 4] {
    let present = |kind: Kind| -> Vec<Kind> {
        if board.pad_kinds.contains(&kind) {
            vec![kind]
        } else {
            Vec::new()
        }
    };
    [
        vec![Kind::Track],
        vec![Kind::Via],
        present(Kind::SmdPad),
        present(Kind::PlatedPad),
    ]
}

/// For groups `a` (A's) and `b` (B's): per kind of A (a track, a via)
/// and of B (a track, a via, a pad), the layers the rule holds on.
fn kind_masks(m: &Matcher, other_kinds: &[Vec<Kind>; 4], a: usize, b: usize) -> KindMasks {
    let board = m.board;
    let mut result = [[0u32; 4]; 2];
    for (ai, a_kind) in ROUTED_KINDS.iter().enumerate() {
        for (bi, b_kinds) in other_kinds.iter().enumerate() {
            let mut per_kind = Vec::new();
            for b_kind in b_kinds {
                let mut mask = 0u32;
                for (index, layer) in board.copper_layers.iter().enumerate() {
                    if !m.layers.contains(&layer.as_str()) {
                        continue;
                    }
                    let a_item = Item {
                        kind: *a_kind,
                        net: Some(m.representative(&m.groups[a])),
                        layer,
                    };
                    let b_item = Item {
                        kind: *b_kind,
                        net: Some(m.representative(&m.groups[b])),
                        layer,
                    };
                    let truth = m.truth(a_item, Some(b_item), false);
                    // (Unknown where the whole was known: kept.)
                    if truth != T3::False {
                        mask |= 1u32.checked_shl(index as u32).unwrap_or(0);
                    }
                }
                per_kind.push(mask);
            }
            result[ai][bi] = per_kind.into_iter().fold(0, |all, mask| all | mask);
        }
    }
    result
}

/// The kinds and layers of every matched pair of groups, both ways.
fn kind_tables(
    m: &Matcher,
    other_kinds: &[Vec<Kind>; 4],
    matched: &BTreeSet<(usize, usize)>,
) -> BTreeMap<(usize, usize), KindMasks> {
    let mut tables = BTreeMap::new();
    for &(i, j) in matched {
        tables.insert((i, j), kind_masks(m, other_kinds, i, j));
        if i != j {
            tables.insert((j, i), kind_masks(m, other_kinds, j, i));
        }
    }
    tables
}

/// The groups whose nets' clearance a rule raises, where one clearance a
/// net keeps the rule: one on every layer and every kind of item for
/// every matched pair, and groups that cover every matched pair. `None`
/// where the rule is a pair rule.
fn raised_groups(
    m: &Matcher,
    other_kinds: &[Vec<Kind>; 4],
    truth: &BTreeMap<(usize, usize), T3>,
    matched: &BTreeSet<(usize, usize)>,
    tables: &BTreeMap<(usize, usize), KindMasks>,
) -> Option<BTreeSet<usize>> {
    let groups = &m.groups;
    let all_layers = all_layers(m.board);
    let uniform = tables.values().all(|table| {
        table.iter().all(|row| {
            row.iter()
                .zip(other_kinds)
                .all(|(mask, kinds)| kinds.is_empty() || *mask == all_layers)
        })
    });
    // Groups that match every group, themselves included: the
    // rule is theirs against anything, and raising them keeps
    // every matched pair, as KiCad's larger-of-two does. Failing
    // that, groups that match every group they have a pair with
    // on this board (a single net needs no clearance to itself).
    let covers = |set: &BTreeSet<usize>| {
        matched
            .iter()
            .all(|(i, j)| set.contains(i) || set.contains(j))
    };
    let star: BTreeSet<usize> = (0..groups.len())
        .filter(|&i| (0..groups.len()).all(|j| truth[&(i.min(j), i.max(j))] == T3::True))
        .collect();
    let present: BTreeSet<usize> = (0..groups.len())
        .filter(|&i| {
            (0..groups.len())
                .filter(|&j| eligible(groups, i, j))
                .all(|j| matched.contains(&(i.min(j), i.max(j))))
        })
        .collect();
    if !uniform {
        None
    } else if covers(&star) {
        Some(star)
    } else if covers(&present) {
        Some(present)
    } else {
        None
    }
}

/// A rule between two sets of nets (ChengminLi's flyback:
/// 2.5 mm primary to secondary), from some item types only
/// (JLCPCB's "Pad to Track": 0.2 mm from pads to tracks over
/// classes of 0.127 on AMLing2's drone carrier), or on some
/// layers only: one clearance a net would keep it from
/// every net, every item, on every layer, so it is a pair
/// rule of the router (`pcb_router::ClearanceRule`).
fn lower_pair_rule(
    lowered: &mut Lowered,
    rule: &Rule,
    constraint: &Constraint,
    m: &Matcher,
    value: f64,
    matched: &BTreeSet<(usize, usize)>,
    tables: &BTreeMap<(usize, usize), KindMasks>,
) {
    let board = m.board;
    let groups = &m.groups;
    let binds = |pairs: &BTreeSet<(usize, usize)>| {
        pairs.iter().any(|&(i, j)| {
            groups[i]
                .iter()
                .chain(groups[j].iter())
                .any(|&net| board.current[net].clearance_mm < value - 1.0e-9)
        })
    };
    let sides: BTreeSet<usize> = matched.iter().flat_map(|(i, j)| [*i, *j]).collect();
    let nets: usize = sides.iter().map(|g| groups[*g].len()).sum();
    if !binds(matched) {
        lowered.note(
            rule,
            constraint,
            "no_effect",
            format!(
                "clearance {} {}: the net classes keep at least as much",
                mm(value),
                if sides.len() > 1 {
                    "between two sets of nets"
                } else {
                    "on some item kinds or layers"
                }
            ),
        );
        return;
    }
    let side = |group: usize| (groups.len() > 1).then(|| groups[group].clone());
    let mut kinds_seen = 0u16;
    let mut layers_seen = 0u32;
    for (&(a, b), table) in tables {
        let mut by_layers: BTreeMap<u32, u16> = BTreeMap::new();
        for (ai, row) in table.iter().enumerate() {
            for (bi, mask) in row.iter().enumerate() {
                if *mask != 0 {
                    *by_layers.entry(*mask).or_default() |= 1 << (ai * 4 + bi);
                }
            }
        }
        for (mask, kinds) in by_layers {
            kinds_seen |= kinds;
            layers_seen |= mask;
            let pair = PairClearance {
                first: side(a),
                second: side(b),
                kinds,
                layers: mask,
                clearance: value,
            };
            if !lowered.pair_clearances.contains(&pair) {
                lowered.pair_clearances.push(pair);
            }
        }
    }
    let kind_names = ["track", "via", "pad", "plated pad"];
    let mut kinds_text: Vec<String> = Vec::new();
    for ai in 0..2 {
        for bi in 0..4 {
            if kinds_seen & (1 << (ai * 4 + bi)) != 0 {
                kinds_text.push(format!("{} to {}", kind_names[ai], kind_names[bi]));
            }
        }
    }
    let layers_text = if layers_seen == all_layers(board) {
        "every layer".to_string()
    } else {
        board
            .copper_layers
            .iter()
            .enumerate()
            .filter(|(index, _)| layers_seen & 1u32.checked_shl(*index as u32).unwrap_or(0) != 0)
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let detail = format!(
        "clearance {} as a pair rule: {} ({}; {})",
        mm(value),
        if groups.len() == 1 {
            "every net".to_string()
        } else if sides.len() > 1 {
            format!("between sets of nets, {nets} nets")
        } else {
            format!("within a set of {nets} nets")
        },
        kinds_text.join(", "),
        layers_text
    );
    lowered.note(rule, constraint, "applied", detail);
}

/// Raises the clearance of the nets of `raise` (every net's where it
/// holds every group) to `value`: one clearance a net keeps the rule,
/// approximated where the rule is narrower than a net
/// (`narrower_than_nets`).
fn raise_clearances(
    lowered: &mut Lowered,
    rule: &Rule,
    constraint: &Constraint,
    m: &Matcher,
    value: f64,
    raise: &BTreeSet<usize>,
) {
    let board = m.board;
    let groups = &m.groups;
    let every = raise.len() == groups.len();
    let raised_nets: usize = raise.iter().map(|g| groups[*g].len()).sum();
    let raises = raise
        .iter()
        .flat_map(|g| groups[*g].iter())
        .any(|&net| board.current[net].clearance_mm < value - 1.0e-9);
    if every {
        lowered.clearance_all = lowered.clearance_all.max(value);
    } else {
        for g in raise {
            for &net in &groups[*g] {
                lowered.clearance[net] = lowered.clearance[net].max(value);
            }
        }
    }
    let status = if !narrower_than_nets(&m.condition, rule) {
        "applied"
    } else {
        "approximated"
    };
    let mut detail = format!(
        "clearance {} on {}",
        mm(value),
        if every {
            "every net".to_string()
        } else {
            format!("{raised_nets} of {} nets", board.nets.len())
        }
    );
    if !raises {
        detail.push_str(": the net classes keep at least as much");
        lowered.note(rule, constraint, "no_effect", detail);
        return;
    }
    if status == "approximated" {
        detail.push_str(
            " (on every layer and against every item type: the router has one clearance a net)",
        );
    }
    lowered.note(rule, constraint, status, detail);
}

/// Whether a clearance rule asks less than every layer and every routed
/// item type, which the router's per-net clearance cannot tell apart.
pub(super) fn narrower_than_nets(condition: &Ex, rule: &Rule) -> bool {
    fn item_atoms(ex: &Ex) -> bool {
        match ex {
            Ex::Or(a, b) | Ex::And(a, b) | Ex::Compare(_, a, b) => item_atoms(a) || item_atoms(b),
            Ex::Not(a) => item_atoms(a),
            Ex::Property(_, name) => {
                matches!(name.as_str(), "type" | "layer" | "padtype" | "viatype")
            }
            Ex::Call(_, name, _) => !matches!(
                name.as_str(),
                "hasnetclass" | "innetclass" | "hasexactnetclass"
            ),
            _ => false,
        }
    }
    rule.layer.is_some() || item_atoms(condition)
}
