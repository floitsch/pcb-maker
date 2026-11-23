// Copyright (C) 2026 Florian Loitsch.

//! Evidence for a through-hole mask bridge between two held footprints.

use super::*;
use pcb_kicad_file::copper_geometry::{ObstacleGeometry, lower_pad};
use pcb_kicad_file::kicad_sexpr::{Expr, form_at, form_f64};

#[derive(Clone, Copy)]
struct CirclePad<'a> {
    uuid: &'a str,
    description: &'a str,
    part: &'a str,
    kind: &'a str,
    center: [f64; 2],
    radius: f64,
    drill_radius: f64,
}

fn reported_pad<'a>(
    item: &'a DrcItem,
    description: &'a str,
    board: &'a Expr,
    mask: &str,
    copper: &str,
) -> Option<CirclePad<'a>> {
    if !is_pad_item(description) {
        return None;
    }
    let part = pad_footprint(description)?;
    let mut footprints = board.children().iter().filter(|node| {
        matches!(node.head(), Some("footprint" | "module"))
            && crate::footprint_reference(node).as_deref() == Some(part)
    });
    let footprint = footprints.next()?;
    if footprints.next().is_some() {
        return None;
    }
    let uuid = item.uuid.as_deref()?;
    let mut matching = footprint.children().iter().filter(|pad| {
        pad.head() == Some("pad") && crate::footprint_uuid(pad).as_deref() == Some(uuid)
    });
    let pad = matching.next()?;
    let kind = pad.children().get(2)?.atom()?;
    let layers = pad.child("layers")?.children();
    let has_layer = |names: &[&str]| {
        layers
            .iter()
            .any(|layer| layer.atom().is_some_and(|name| names.contains(&name)))
    };
    if matching.next().is_some()
        || !matches!(kind, "np_thru_hole" | "thru_hole")
        || pad.children().get(3)?.atom()? != "circle"
        || pad.child("padstack").is_some()
        || pad.child("drill")?.children().len() != 2
        || !has_layer(&[mask, "*.Mask"])
        || (kind == "thru_hole" && !has_layer(&[copper, "*.Cu", "F&B.Cu"]))
    {
        return None;
    }
    let drill_radius = form_f64(pad, "drill", 1).ok()? / 2.0;
    let lowered = lower_pad(pad, form_at(footprint).ok()?).ok()?;
    let ObstacleGeometry::Circle { center, radius } = lowered.geometry else {
        return None;
    };
    let anchor = item.located()?;
    if distance(center, anchor) > 0.0001 || drill_radius <= 0.0 || radius < drill_radius {
        return None;
    }
    Some(CirclePad {
        uuid,
        description,
        part,
        kind,
        center,
        radius,
        drill_radius,
    })
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn hold(brief: &crate::KiCadPlacementConstraints, part: &str) -> Option<String> {
    let places = brief
        .place
        .iter()
        .filter(|pose| pose.part == part)
        .map(|pose| ("place", pose.x, pose.y, pose.origin.as_deref()));
    let additions = brief
        .add
        .iter()
        .filter(|pose| pose.part == part)
        .map(|pose| {
            (
                if pose.from_board {
                    "add.from_board"
                } else {
                    "add"
                },
                pose.x,
                pose.y,
                pose.origin.as_deref(),
            )
        });
    let mut entries = places.chain(additions);
    let (kind, Some(x), Some(y), origin) = entries.next()? else {
        return None;
    };
    if entries.next().is_some() {
        return None;
    }
    let origin = match origin {
        Some("absolute") => "absolute",
        Some("board") | None => "relative to the board origin",
        _ => return None,
    };
    Some(format!(
        "`{kind}` holds {part} at ({x:.4}, {y:.4}) mm {origin}"
    ))
}

pub(super) fn action(
    findings: &[&DrcFinding],
    board: Option<&Expr>,
    facts: &KiCadRunFacts,
) -> Option<String> {
    let board = board?;
    let brief = facts.authored_layout_constraints.as_ref()?;
    let mut pairs = BTreeMap::<_, (String, BTreeSet<&str>)>::new();
    for finding in findings {
        let (mask, copper) = match finding.description.split_whitespace().next()? {
            "Front" => ("F.Mask", "F.Cu"),
            "Rear" => ("B.Mask", "B.Cu"),
            _ => return None,
        };
        let ([first, second], [first_label, second_label]) = (
            finding.items.as_slice(),
            finding.item_descriptions.as_slice(),
        ) else {
            return None;
        };
        let first = reported_pad(first, first_label, board, mask, copper)?;
        let second = reported_pad(second, second_label, board, mask, copper)?;
        let (hole, pad) = match (first.kind, second.kind) {
            ("np_thru_hole", "thru_hole") => (first, second),
            ("thru_hole", "np_thru_hole") => (second, first),
            _ => return None,
        };
        if hole.part == pad.part
            || !facts.fixed_parts.contains(hole.part)
            || !facts.fixed_parts.contains(pad.part)
        {
            return None;
        }
        let hole_hold = hold(brief, hole.part)?;
        let pad_hold = hold(brief, pad.part)?;
        let centers = distance(hole.center, pad.center);
        let overlap = hole.drill_radius + pad.radius - centers;
        // The mechanical drill reaches the plated pad's copper annulus.
        // This proof does not depend on solder-mask expansion or routing.
        if overlap <= 0.000001 || centers + hole.drill_radius <= pad.drill_radius + 0.000001 {
            return None;
        }
        let (example, faces) = pairs
            .entry((hole.part, hole.uuid, pad.part, pad.uuid))
            .or_default();
        faces.insert(mask);
        if example.is_empty() {
            let [hx, hy] = hole.center;
            let [px, py] = pad.center;
            *example = format!(
                "{} at ({hx:.4}, {hy:.4}) mm reaches {overlap:.3} mm inside the copper outline of {} at ({px:.4}, {py:.4}) mm. {hole_hold}. {pad_hold}.",
                hole.description, pad.description
            );
        }
    }
    if pairs.is_empty() {
        return None;
    }
    let mut examples: Vec<_> = pairs
        .values()
        .take(4)
        .map(|(example, faces)| {
            format!(
                "On {}, {example}",
                faces.iter().copied().collect::<Vec<_>>().join(" and ")
            )
        })
        .collect();
    if pairs.len() > examples.len() {
        examples.push(format!(
            "{} further held pairs have the same kind of conflict.",
            pairs.len() - examples.len()
        ));
    }
    Some(format!(
        "{} Review the footprint pad and drill geometry against the mechanical brief before routing again. Separating the pair requires a footprint edit or a change to one of the supplied poses.",
        examples.join(" ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BOARD: &str = r#"(kicad_pcb
      (footprint "test:hole" (at 1 2) (property "Reference" "H1")
        (pad "" np_thru_hole circle (at 0 0) (size 3 3) (drill 3) (layers "*.Cu" "*.Mask") (uuid "hole")))
      (footprint "test:ring" (at 3.1 2) (property "Reference" "J1")
        (pad "1" thru_hole circle (at 0 0) (size 2 2) (drill 1) (layers "*.Cu" "*.Mask") (net "COL4") (uuid "ring"))))"#;

    fn fixture() -> (KiCadRunFacts, Vec<DrcFinding>) {
        let mut facts = KiCadRunFacts::new("layout-kicad-board");
        facts.fixed_parts = ["H1".into(), "J1".into()].into();
        facts.authored_layout_constraints = serde_json::from_value(json!({
            "version": 1,
            "place": [{"part":"J1", "x":3.1, "y":2.0, "origin":"absolute"}],
            "add": [{"part":"H1", "from_board":true, "x":1.0, "y":2.0, "origin":"absolute"}],
        }))
        .unwrap();
        let findings = ["Front", "Rear"]
            .iter()
            .map(|face| {
                drc_finding(&json!({
                    "type": "solder_mask_bridge",
                    "description": format!("{face} solder mask aperture bridges items with different nets"),
                    "items": [
                        {"description": "NPTH pad of H1", "uuid":"hole", "pos":{"x":1.0,"y":2.0}},
                        {"description": "PTH pad 1 [COL4] of J1", "uuid":"ring", "pos":{"x":3.1,"y":2.0}},
                    ]
                }))
            })
            .collect();
        (facts, findings)
    }

    #[test]
    fn matching_native_pair_names_both_faces_and_explicit_constraints() {
        let (facts, findings) = fixture();
        let findings: Vec<_> = findings.iter().collect();
        let board = crate::parse(BOARD).unwrap();
        let advice = action(&findings, Some(&board), &facts).unwrap();
        assert!(advice.contains("B.Mask and F.Mask"));
        assert!(advice.contains("NPTH pad of H1 at (1.0000, 2.0000) mm"));
        assert!(advice.contains(
            "0.400 mm inside the copper outline of PTH pad 1 [COL4] of J1 at (3.1000, 2.0000) mm"
        ));
        assert!(advice.contains("`add.from_board` holds H1"));
        assert!(advice.contains("`place` holds J1"));
    }

    #[test]
    fn a_bridge_without_held_overlapping_drill_and_copper_uses_general_guidance() {
        let (mut facts, mut findings) = fixture();
        let board = crate::parse(BOARD).unwrap();
        let smaller = crate::parse(&BOARD.replace("(drill 3)", "(drill 0.5)")).unwrap();
        let case = [&findings[0], &findings[1]];
        assert!(action(&case, Some(&smaller), &facts).is_none());
        facts.fixed_parts.remove("J1");
        assert!(action(&case, Some(&board), &facts).is_none());
        facts.fixed_parts.insert("J1".into());
        findings[1].item_descriptions[1] = "Track [COL4] on B.Cu, length 1 mm".into();
        assert!(action(&[&findings[0], &findings[1]], Some(&board), &facts).is_none());
        findings[1].item_descriptions[1] = "PTH pad 1 [COL4] of J1".into();
        findings[1].items[1].uuid = Some("different".into());
        assert!(action(&[&findings[0], &findings[1]], Some(&board), &facts).is_none());
    }
}
