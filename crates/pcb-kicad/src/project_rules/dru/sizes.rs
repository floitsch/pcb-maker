// Copyright (C) 2026 Toit contributors.

//! The sizes the custom rules give one item: track widths (per layer
//! where a rule holds on some layers only), connection widths, via
//! diameters, drills and annular rings.

use super::condition::{Item, Kind, T3};
use super::lower::{BoardFacts, Lowered, Matcher, mm};
use super::{Constraint, Rule};

/// Lowers a track width, connection width, via diameter, hole size or
/// annular width: the nets the rule holds for, the bounds it sets them
/// (`set_bounds`, per layer where it holds on some layers only:
/// `write_layer_widths`), and whether that changes what the net classes
/// and the board setup keep.
pub(super) fn lower_sizes(
    lowered: &mut Lowered,
    rule: &Rule,
    constraint: &Constraint,
    m: &Matcher,
) {
    let kind = m.kind;
    let board = m.board;
    // Unary: tracks or vias of a net. A track width rule may hold
    // on some layers only (`(layer outer)`, `A.Layer == 'In1.Cu'`):
    // per layer of the board, the nets it holds for. A connection
    // width's minimum is a floor on the tracks' width, per layer
    // alike (its maximum and opt mean nothing to KiCad).
    let widths = matches!(kind, "track_width" | "connection_width");
    let per_layer: Vec<Vec<usize>> = if widths {
        nets_per_layer(m)
    } else {
        Vec::new()
    };
    let by_layer = per_layer.windows(2).any(|pair| pair[0] != pair[1]);
    record_rule_layer_widths(lowered, constraint, kind, by_layer, &per_layer);
    let mut unknown = false;
    let mut applies = Vec::new();
    for group in &m.groups {
        match m.truth_for(Some(m.representative(group)), None, false) {
            T3::True => applies.extend(group.iter().copied()),
            T3::Unknown => unknown = true,
            T3::False => {}
        }
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
    if applies.is_empty() {
        lowered.note(
            rule,
            constraint,
            "no_effect",
            format!(
                "matches no routed {} on this board",
                if widths { "track" } else { "via" }
            ),
        );
        return;
    }
    let every = applies.len() == board.nets.len();
    let parts = set_bounds(lowered, kind, constraint, &applies, every, by_layer);
    if parts.is_empty() {
        lowered.note(rule, constraint, "no_effect", "no limit".into());
        return;
    }
    let changes = changes_current(board, kind, constraint, &applies);
    let mut detail = format!(
        "{} on {}",
        parts.join(", "),
        if every {
            "every net".to_string()
        } else {
            format!("{} of {} nets", applies.len(), board.nets.len())
        }
    );
    // (A track width rule on some layers is kept per layer.)
    let approximated =
        !by_layer && rule.layer.is_some() && m.layers.len() < board.copper_layers.len();
    if approximated {
        detail.push_str(" (on every layer: the router has one width a net)");
    }
    if by_layer {
        let names: Vec<&str> = board
            .copper_layers
            .iter()
            .zip(&per_layer)
            .filter(|(_, nets)| !nets.is_empty())
            .map(|(name, _)| name.as_str())
            .collect();
        detail.push_str(&format!(" on {} (per layer)", names.join(", ")));
    }
    let necks = neck_note(board, constraint, widths, every, by_layer);
    if let Some(note) = &necks {
        detail.push_str(note);
    }
    if !changes && necks.is_none() {
        detail.push_str(": the net classes and the board setup keep it");
        lowered.note(rule, constraint, "no_effect", detail);
        return;
    }
    if by_layer {
        write_layer_widths(lowered, board, kind, constraint, &per_layer);
    }
    lowered.note(
        rule,
        constraint,
        if approximated {
            "approximated"
        } else {
            "applied"
        },
        detail,
    );
}

/// Per copper layer of the board (all of them, in its order), the nets
/// whose tracks the rule surely holds for there: none on a layer the
/// rule's layer clause leaves out.
fn nets_per_layer(m: &Matcher) -> Vec<Vec<usize>> {
    m.board
        .copper_layers
        .iter()
        .map(|name| {
            let mut nets = Vec::new();
            if m.layers.contains(&name.as_str()) {
                for group in &m.groups {
                    let a = Item {
                        kind: Kind::Track,
                        net: Some(m.representative(group)),
                        layer: name.as_str(),
                    };
                    if m.truth(a, None, false) == T3::True {
                        nets.extend(group.iter().copied());
                    }
                }
            }
            nets
        })
        .collect()
}

/// A width the rule aims at (an opt, or a maximum: an impedance's
/// band) is the designer's width on those layers; a minimum alone
/// is a floor for the process there (the 4in1-mini's "2 oz outer
/// copper, track width", 0.16 mm on F.Cu and B.Cu, which the
/// inner layers' 1 oz copper does not need), not a width the
/// class is for (trial 8: the verdict told it to keep every
/// net off the inner layers). Recorded per net and layer whether or not
/// the rule changes what the router does (`Lowered::rule_layer_widths`).
fn record_rule_layer_widths(
    lowered: &mut Lowered,
    constraint: &Constraint,
    kind: &str,
    by_layer: bool,
    per_layer: &[Vec<usize>],
) {
    let aimed = constraint.opt.is_some() || constraint.max.is_some();
    if kind == "track_width"
        && by_layer
        && aimed
        && let Some(width) = constraint.opt.or(constraint.min).or(constraint.max)
    {
        for (layer, nets) in per_layer.iter().enumerate() {
            for &net in nets {
                lowered
                    .rule_layer_widths
                    .entry(net)
                    .or_default()
                    .insert(layer, width);
            }
        }
    }
}

/// Raises (a minimum) or lowers (a maximum: `max`) the values of the nets
/// a rule holds for (`applies`), and the board's value (`all`) where it
/// holds for every net.
fn bound(
    values: &mut [f64],
    all: Option<&mut f64>,
    value: f64,
    max: bool,
    applies: &[usize],
    every: bool,
) {
    if every && let Some(all) = all {
        *all = all.max(value);
    }
    for &net in applies {
        values[net] = if max {
            values[net].min(value)
        } else {
            values[net].max(value)
        };
    }
}

/// Sets the bounds a rule gives the nets it holds for (`applies`) on
/// every layer, and describes them, one part per bound; a rule held per
/// layer (`by_layer`) is only described here (`write_layer_widths`).
fn set_bounds(
    lowered: &mut Lowered,
    kind: &str,
    constraint: &Constraint,
    applies: &[usize],
    every: bool,
    by_layer: bool,
) -> Vec<String> {
    let mut parts = Vec::new();
    match kind {
        "connection_width" => {
            // Per layer below where `by_layer`, else per net.
            if let Some(min) = constraint.min {
                if !by_layer {
                    bound(
                        &mut lowered.width_min,
                        Some(&mut lowered.width_min_all),
                        min,
                        false,
                        applies,
                        every,
                    );
                }
                parts.push(format!(
                    "connection width at least {}: a floor on the track width",
                    mm(min)
                ));
            }
        }
        "track_width" if by_layer => {
            // Per layer (`Lowered::layer_width_min`, ...; written
            // below unless the rule has no effect): the router
            // routes each layer at its own width.
            if let Some(min) = constraint.min {
                parts.push(format!("width at least {}", mm(min)));
            }
            if let Some(max) = constraint.max {
                parts.push(format!("at most {}", mm(max)));
            }
            if let Some(opt) = constraint.opt {
                parts.push(format!("routed at {}", mm(opt)));
            }
        }
        "track_width" => {
            if let Some(min) = constraint.min {
                bound(
                    &mut lowered.width_min,
                    Some(&mut lowered.width_min_all),
                    min,
                    false,
                    applies,
                    every,
                );
                parts.push(format!("width at least {}", mm(min)));
            }
            if let Some(max) = constraint.max {
                bound(&mut lowered.width_max, None, max, true, applies, every);
                parts.push(format!("at most {}", mm(max)));
            }
            if let Some(opt) = constraint.opt {
                for &net in applies {
                    lowered.width_opt[net] = Some(opt);
                }
                parts.push(format!("routed at {}", mm(opt)));
            }
        }
        "via_diameter" => {
            if let Some(min) = constraint.min {
                bound(
                    &mut lowered.via_diameter_min,
                    Some(&mut lowered.via_diameter_min_all),
                    min,
                    false,
                    applies,
                    every,
                );
                parts.push(format!("via diameter at least {}", mm(min)));
            }
            if let Some(max) = constraint.max {
                bound(
                    &mut lowered.via_diameter_max,
                    None,
                    max,
                    true,
                    applies,
                    every,
                );
                parts.push(format!("at most {}", mm(max)));
            }
        }
        "hole_size" => {
            if let Some(min) = constraint.min {
                bound(
                    &mut lowered.via_drill_min,
                    Some(&mut lowered.via_drill_min_all),
                    min,
                    false,
                    applies,
                    every,
                );
                parts.push(format!("via drill at least {}", mm(min)));
            }
            if let Some(max) = constraint.max {
                bound(&mut lowered.via_drill_max, None, max, true, applies, every);
                parts.push(format!("at most {}", mm(max)));
            }
        }
        _ => {
            if let Some(min) = constraint.min {
                bound(
                    &mut lowered.annulus_min,
                    Some(&mut lowered.annulus_min_all),
                    min,
                    false,
                    applies,
                    every,
                );
                parts.push(format!("via annular ring at least {}", mm(min)));
            }
        }
    }
    parts
}

/// Whether a rule asks some net it holds for (`applies`) for more than
/// its rules before the custom rules keep (`BoardFacts::current`): a
/// width or via below its minimum, above its maximum, or a track width
/// off its opt.
fn changes_current(
    board: &BoardFacts,
    kind: &str,
    constraint: &Constraint,
    applies: &[usize],
) -> bool {
    applies.iter().any(|&net| {
        let now = &board.current[net];
        let (value, drill) = match kind {
            "track_width" | "connection_width" => (now.trace_width_mm, 0.0),
            "via_diameter" => (now.via_size_mm, 0.0),
            "hole_size" => (now.via_drill_mm, 0.0),
            _ => (now.via_size_mm, now.via_drill_mm),
        };
        let min = constraint.min.map_or(0.0, |min| {
            if kind == "annular_width" {
                drill + 2.0 * min
            } else {
                min
            }
        });
        value < min - 1.0e-9
            || (kind != "connection_width"
                && constraint.max.is_some_and(|max| value > max + 1.0e-9))
            || (kind == "track_width"
                && constraint
                    .opt
                    .is_some_and(|opt| (value - opt).abs() > 1.0e-9))
    })
}

/// The note on the neck-down width a width minimum above it raises, for
/// a rule's detail; `None` where it raises none.
fn neck_note(
    board: &BoardFacts,
    constraint: &Constraint,
    widths: bool,
    every: bool,
    by_layer: bool,
) -> Option<String> {
    let above = widths
        && constraint
            .min
            .is_some_and(|min| min > board.neck_width + 1.0e-9);
    if above && (!every || by_layer) {
        // A minimum on some nets floors their neck-downs and their
        // narrow-signal width as well (router `width_floor`).
        Some(format!(
            "; their neck-downs rise from {} to it",
            mm(board.neck_width)
        ))
    } else if above && every && !by_layer {
        // A board-wide minimum also floors the neck-downs.
        Some(format!(
            "; the neck-down width {} rises to it",
            mm(board.neck_width)
        ))
    } else {
        None
    }
}

/// Writes a rule held per layer into the per-layer bounds
/// (`Lowered::layer_width_min`, ...): per layer, the nets it holds for
/// there (`per_layer`); a connection width gives a minimum only.
fn write_layer_widths(
    lowered: &mut Lowered,
    board: &BoardFacts,
    kind: &str,
    constraint: &Constraint,
    per_layer: &[Vec<usize>],
) {
    let layer_count = board.copper_layers.len();
    if lowered.layer_width_min.is_empty() {
        lowered.layer_width_min = vec![vec![0.0; board.nets.len()]; layer_count];
        lowered.layer_width_max = vec![vec![f64::INFINITY; board.nets.len()]; layer_count];
        lowered.layer_width_opt = vec![vec![None; board.nets.len()]; layer_count];
    }
    for (layer, nets) in per_layer.iter().enumerate() {
        for &net in nets {
            if let Some(min) = constraint.min {
                let slot = &mut lowered.layer_width_min[layer][net];
                *slot = slot.max(min);
            }
            if kind == "connection_width" {
                continue;
            }
            if let Some(max) = constraint.max {
                let slot = &mut lowered.layer_width_max[layer][net];
                *slot = slot.min(max);
            }
            if let Some(opt) = constraint.opt {
                lowered.layer_width_opt[layer][net] = Some(opt);
            }
        }
    }
}
