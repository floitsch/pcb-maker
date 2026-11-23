// Copyright (C) 2026 Toit contributors.

//! The soft constraints between parts: `near`, `group`, `apart` and
//! `relative`, lowered into `pcb_placer`'s relations.

use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadNearConstraint {
    pub part: String,
    /// Another part: the gap between the two bodies is at most `max_mm`.
    #[serde(default)]
    pub part_of: Option<String>,
    /// A pad, `REF:PAD`: the gap between the body and the pad centre.
    #[serde(default)]
    pub pin_of: Option<String>,
    /// A pad on `part`: measure centre to centre with `pin_of`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad: Option<String>,
    /// An explicit limit. Omit it to request `minimize` instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_mm: Option<f64>,
    /// Prefer a shorter pad-to-pad distance without an acceptance limit.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub minimize: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadRelativeConstraint {
    pub part: String,
    #[serde(default)]
    pub below: Option<String>,
    #[serde(default)]
    pub above: Option<String>,
    #[serde(default)]
    pub left_of: Option<String>,
    #[serde(default)]
    pub right_of: Option<String>,
    /// Largest gap between the two bodies (default 5 mm).
    #[serde(default)]
    pub max_gap_mm: Option<f64>,
}

/// Parts kept together: each one's body within `max_mm` of the central
/// part's body (a regulator with its inductor and capacitors).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadGroupConstraint {
    /// References or glob patterns.
    pub parts: Vec<String>,
    /// The part the others gather around (default: the largest).
    #[serde(default)]
    pub around: Option<String>,
    pub max_mm: f64,
}

/// Parts kept away from others: every body of `parts` at least `min_mm`
/// from every body of `from` (a temperature sensor away from the regulator,
/// an audio input away from the switcher).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadApartConstraint {
    /// References or glob patterns.
    pub parts: Vec<String>,
    pub from: Vec<String>,
    pub min_mm: f64,
}

/// The footprint of `reference` a near or relative constraint about
/// `part` anchors to: the only one, or of several (a panel's copies) the
/// one sharing the most nets with `part` (its own copy's: KiKit names
/// each copy's nets apart), the first of those in board order. Until
/// 2026-11-04 the first in board order (by reference, then uuid), which on
/// a panel of copies could be another copy's than the part's.
fn anchor_in_copy(
    problem: &core::Problem,
    references: &[String],
    footprints: usize,
    reference: &str,
    part: usize,
) -> Option<usize> {
    let candidates: Vec<usize> = (0..footprints)
        .filter(|index| references[*index] == reference)
        .collect();
    if candidates.len() < 2 {
        return candidates.first().copied();
    }
    let nets: BTreeSet<usize> = problem.components[part]
        .pins
        .iter()
        .map(|pin| pin.net)
        .collect();
    let shared = |candidate: usize| {
        problem.components[candidate]
            .pins
            .iter()
            .map(|pin| pin.net)
            .collect::<BTreeSet<usize>>()
            .intersection(&nets)
            .count()
    };
    let most = candidates
        .iter()
        .map(|candidate| shared(*candidate))
        .max()?;
    candidates
        .into_iter()
        .find(|candidate| shared(*candidate) == most)
}

/// Applies `near`: each part's body within `max_mm` of another part's
/// body (`part_of`) or of a pad (`pin_of`).
pub(super) fn apply_near(
    problem: &mut core::Problem,
    parts: &Parts,
    entries: &[KiCadNearConstraint],
    constrained: &mut BTreeSet<usize>,
) -> Result<(), String> {
    for entry in entries {
        let max = match (entry.max_mm, entry.minimize, entry.pad.is_some()) {
            (Some(mm), false, _) => Some(mm.max(0.0)),
            (None, true, true) => None,
            _ => {
                return Err(format!(
                    "near constraint for {} needs max_mm or minimize: true with a pad and pin_of",
                    entry.part
                ));
            }
        };
        let part = parts.find(&entry.part)?;
        let anchor = near_anchor(problem, parts, entry, part)?;
        let relation = if let Some(pad) = &entry.pad {
            let Some((_, anchor_pad)) = entry.pin_of.as_deref().and_then(|pin| pin.split_once(':'))
            else {
                return Err(format!(
                    "near constraint for {} needs pin_of when pad is set",
                    entry.part
                ));
            };
            let source = pad_anchor(parts, part, pad)
                .ok_or_else(|| format!("near constraint: {} has no pad {pad:?}", entry.part))?;
            Relation::PadNear {
                source,
                anchor,
                max,
                pad_numbers: Some([pad.clone(), anchor_pad.to_owned()]),
            }
        } else {
            Relation::Near {
                part,
                anchor,
                max: max.expect("a body near constraint has a limit"),
            }
        };
        constrained.insert(relation.parts()[0].expect("a near relation has a part"));
        problem.constraints.relations.push(relation);
    }
    Ok(())
}

/// A pad number can name several copper shapes. A centre-to-centre
/// constraint needs one point; reject distinct centres before the ordinary
/// pin lookup takes the first shape for each pad number.
pub(crate) fn validate_near_pads(
    pcb: &Expr,
    constraints: &KiCadPlacementConstraints,
) -> Result<(), String> {
    let mut named = BTreeSet::new();
    for entry in &constraints.near {
        if let Some(pad) = entry.pad.as_deref() {
            named.insert((entry.part.as_str(), pad));
            named.extend(entry.pin_of.as_deref().and_then(|pin| pin.split_once(':')));
        }
    }
    if named.is_empty() {
        return Ok(());
    }
    for footprint in pcb
        .children()
        .iter()
        .filter(|child| child.head() == Some("footprint"))
    {
        let Some(reference) = footprint_reference(footprint) else {
            continue;
        };
        let mut centers = BTreeMap::new();
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            let Some(number) = pad.children().get(1).and_then(Expr::atom) else {
                continue;
            };
            if !named.contains(&(reference.as_str(), number)) {
                continue;
            }
            let [x, y, _] = form_at(pad)?;
            if centers
                .insert(number, [x, y])
                .is_some_and(|previous| previous != [x, y])
            {
                return Err(format!(
                    "near constraint names {reference}:{number}, which has multiple pad centres"
                ));
            }
        }
    }
    Ok(())
}

/// What a near entry about `part` holds it to: a part's body or a pad, on
/// the row's first part where a row carries it.
fn near_anchor(
    problem: &core::Problem,
    parts: &Parts,
    entry: &KiCadNearConstraint,
    part: usize,
) -> Result<Anchor, String> {
    // The anchor may be a part a row carries (a keyboard's diode at its
    // switch): its place in the row, on the row's first part. Of a
    // reference several footprints share (a panel's copies), the one
    // of the part's copy (`anchor_in_copy`).
    let any = |reference: &str| -> Result<usize, String> {
        anchor_in_copy(problem, parts.references, parts.footprints, reference, part)
            .ok_or_else(|| format!("constraint names unknown part {reference:?}"))
    };
    let anchor = match (&entry.part_of, &entry.pin_of) {
        (Some(other), None) => {
            let index = any(other)?;
            match (parts.leaders.get(&index), parts.places.get(&index)) {
                (Some(leader), Some(place)) => {
                    Anchor::Area(*leader, index, place.center, place.half)
                }
                _ => Anchor::Body(index),
            }
        }
        (None, Some(pin)) => {
            let (reference, pad) = pin
                .split_once(':')
                .ok_or_else(|| format!("pin_of {pin:?} is not REF:PAD"))?;
            let index = any(reference)?;
            pad_anchor(parts, index, pad)
                .ok_or_else(|| format!("pin_of {pin:?}: {reference} has no pad {pad:?}"))?
        }
        _ => {
            return Err(format!(
                "near constraint for {} needs exactly one of part_of and pin_of",
                entry.part
            ));
        }
    };
    Ok(anchor)
}

fn pad_anchor(parts: &Parts, index: usize, pad: &str) -> Option<Anchor> {
    let offset = *parts.pads[index].get(pad)?;
    Some(
        match (parts.leaders.get(&index), parts.places.get(&index)) {
            (Some(leader), Some(place)) => Anchor::Area(
                *leader,
                index,
                [offset[0] + place.offset[0], offset[1] + place.offset[1]],
                [0.0, 0.0],
            ),
            _ => Anchor::Point(index, offset),
        },
    )
}

/// Applies `group`: every member's body within `max_mm` of the central
/// part's (`around`, or the largest member).
pub(super) fn apply_groups(
    problem: &mut core::Problem,
    parts: &Parts,
    entries: &[KiCadGroupConstraint],
    constrained: &mut BTreeSet<usize>,
) -> Result<(), String> {
    for entry in entries {
        let mut members = Vec::new();
        for pattern in &entry.parts {
            let matched = parts.matching(pattern);
            if matched.is_empty() {
                return Err(format!("group constraint {pattern:?} matches no part"));
            }
            for part in matched {
                if !members.contains(&part) {
                    members.push(part);
                }
            }
        }
        let center = match &entry.around {
            Some(reference) => parts.find(reference)?,
            None => *members
                .iter()
                .max_by(|a, b| {
                    let area = |index: usize| {
                        problem.components[index].body_size[0]
                            * problem.components[index].body_size[1]
                    };
                    area(**a).total_cmp(&area(**b))
                })
                .expect("a group has members"),
        };
        for part in members.into_iter().filter(|part| *part != center) {
            problem.constraints.relations.push(Relation::Near {
                part,
                anchor: Anchor::Body(center),
                max: entry.max_mm.max(0.0),
            });
            constrained.insert(part);
        }
        constrained.insert(center);
    }
    Ok(())
}

/// Applies `apart`: every body of `parts` at least `min_mm` from every
/// body of `from`.
#[allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "!(x > 0.0) rejects NaN as well as zero and negative values"
)]
pub(super) fn apply_apart(
    problem: &mut core::Problem,
    parts: &Parts,
    entries: &[KiCadApartConstraint],
    constrained: &mut BTreeSet<usize>,
) -> Result<(), String> {
    for entry in entries {
        let (named, others) = (
            apart_members(parts, &entry.parts)?,
            apart_members(parts, &entry.from)?,
        );
        if !(entry.min_mm > 0.0) {
            return Err(format!(
                "apart constraint {:?}: min_mm must be positive",
                entry.parts
            ));
        }
        for part in &named {
            for other in others.iter().filter(|other| *other != part) {
                problem.constraints.relations.push(Relation::Apart {
                    part: *part,
                    anchor: *other,
                    min: entry.min_mm,
                });
            }
            constrained.insert(*part);
        }
    }
    Ok(())
}

/// The parts an apart entry's globs match, each once, in glob order.
fn apart_members(parts: &Parts, patterns: &[String]) -> Result<Vec<usize>, String> {
    let mut members = Vec::new();
    for pattern in patterns {
        let matched = parts.matching(pattern);
        if matched.is_empty() {
            return Err(format!("apart constraint {pattern:?} matches no part"));
        }
        for part in matched {
            if !members.contains(&part) {
                members.push(part);
            }
        }
    }
    Ok(members)
}

/// Applies `relative`: each part beside another (below, above, left_of or
/// right_of it), at most `max_gap_mm` from it.
pub(super) fn apply_relative(
    problem: &mut core::Problem,
    parts: &Parts,
    entries: &[KiCadRelativeConstraint],
    constrained: &mut BTreeSet<usize>,
) -> Result<(), String> {
    for entry in entries {
        let part = parts.find(&entry.part)?;
        let sides = [
            (&entry.below, Edge::Bottom),
            (&entry.above, Edge::Top),
            (&entry.left_of, Edge::Left),
            (&entry.right_of, Edge::Right),
        ];
        let named: Vec<_> = sides.iter().filter(|(other, _)| other.is_some()).collect();
        if named.len() != 1 {
            return Err(format!(
                "relative constraint for {} needs exactly one of below, above, left_of, right_of",
                entry.part
            ));
        }
        let (other, side) = named[0];
        let other = other.as_deref().expect("named");
        parts.find(other)?;
        let anchor = anchor_in_copy(problem, parts.references, parts.footprints, other, part)
            .ok_or_else(|| format!("constraint names unknown part {other:?}"))?;
        problem.constraints.relations.push(Relation::Beside {
            part,
            anchor,
            side: *side,
            max_gap: entry.max_gap_mm.unwrap_or(5.0).max(0.0),
        });
        constrained.insert(part);
    }
    Ok(())
}
