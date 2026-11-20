// Copyright (C) 2026 Florian Loitsch.

//! Intrinsic pad and drill findings from the importer's existing KiCad pass.

use super::*;
use pcb_kicad_file::kicad_nets::node_net;
use serde_json::Value;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadImportFootprintFinding {
    pub part: String,
    pub footprint: String,
    /// Native KiCad finding type.
    pub kind: String,
    pub count: usize,
    pub evidence: Vec<KiCadImportFootprintEvidence>,
    pub action: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadImportFootprintEvidence {
    /// KiCad's wording, including the measured and required values for rules.
    pub description: String,
    pub items: Vec<KiCadImportFootprintItem>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadImportFootprintItem {
    pub uuid: String,
    pub description: String,
}

struct PadOwner {
    footprint: usize,
    drilled: bool,
    plated: bool,
    unnumbered_unassigned: bool,
}

#[derive(Default)]
struct Findings {
    evidence: Vec<KiCadImportFootprintEvidence>,
    zero_annulus_on_unnumbered_pad: bool,
}

/// Use a built-in pad/drill minimum as the authority. A custom rule's
/// area, side or other placement condition has only been evaluated at a
/// synthetic location.
fn required_items(kind: &str, description: &str) -> Option<(usize, usize, bool)> {
    if kind == "padstack" {
        return Some((1, 0, false));
    }
    let authority = description.split_once('(')?.1;
    if !authority.starts_with("board setup constraints ")
        && !authority.starts_with("board minimum ")
    {
        return None;
    }
    match kind {
        "hole_clearance" => Some((2, 1, false)),
        "hole_to_hole" => Some((2, 2, false)),
        "annular_width" => Some((1, 1, true)),
        "drill_out_of_range" => Some((1, 1, false)),
        _ => None,
    }
}

fn action(kind: &str, zero_annulus_on_unnumbered_pad: bool) -> &'static str {
    match kind {
        "hole_clearance" => {
            "Check the library footprint's copper and hole spacing against the part drawing and project fabrication rules."
        }
        "hole_to_hole" => {
            "Check the drill locations and sizes in the library footprint against the part drawing and project fabrication rules."
        }
        "annular_width" if zero_annulus_on_unnumbered_pad => {
            "KiCad measured zero annular width on an unnumbered plated hole with no net. Check the part drawing for the intended plating. Use NPTH only if the drawing specifies an unplated mechanical hole. If plating is specified, give the drill the required copper annulus."
        }
        "annular_width" => {
            "Check the plated pad's copper and drill dimensions against the project's minimum annular width. Preserve its pad number and electrical connection."
        }
        "drill_out_of_range" => {
            "Check the intended drill size and plating in the library footprint against the part drawing and project fabrication rules."
        }
        _ => {
            "Review the library padstack, including its copper layers, shape and drill, against the part drawing and project fabrication rules."
        }
    }
}

/// Match the report's items to IDs on the board written by import, using
/// the footprint instance as the owner. The staged coordinates are not
/// useful on the board the caller gets. Leave existing copper-clearance
/// findings to the importer's separate pad-gap policy.
pub(super) fn footprint_findings(board: &Expr, drc: &Value) -> Vec<KiCadImportFootprintFinding> {
    let candidates: Vec<_> = drc["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|violation| {
            if violation["severity"] != "error" || violation["excluded"] == true {
                return None;
            }
            let kind = violation["type"].as_str()?;
            let description = violation["description"]
                .as_str()
                .filter(|text| !text.is_empty())?;
            let (count, drills, plated) = required_items(kind, description)?;
            let items = violation["items"]
                .as_array()
                .filter(|items| items.len() == count)?;
            Some((kind, description, items, drills, plated))
        })
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    let footprints: Vec<_> = board
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .collect();
    let mut pads = BTreeMap::new();
    for (footprint, item) in footprints.iter().enumerate() {
        for pad in item
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
        {
            let Some(uuid) = form_atom(pad, "uuid", 1).or_else(|| form_atom(pad, "tstamp", 1))
            else {
                continue;
            };
            let owner = PadOwner {
                footprint,
                drilled: pad.child("drill").is_some(),
                plated: pad.children().get(2).and_then(Expr::atom) == Some("thru_hole"),
                unnumbered_unassigned: pad.children().get(1).and_then(Expr::atom) == Some("")
                    && pad.child("net").is_none_or(|net| {
                        matches!(net.children(), [_, Expr::Atom(code)] if code == "0")
                            || node_net(pad) == Some("")
                    }),
            };
            pads.entry(uuid.to_ascii_lowercase())
                .and_modify(|owner| *owner = None)
                .or_insert(Some(owner));
        }
    }
    let mut findings = BTreeMap::<(usize, &str), Findings>::new();
    for (kind, description, items, drills, plated) in candidates {
        let Some(resolved) = items
            .iter()
            .map(|item| {
                let uuid = item["uuid"].as_str()?;
                let description = item["description"]
                    .as_str()
                    .filter(|text| !text.is_empty())?;
                let owner = pads.get(&uuid.to_ascii_lowercase())?.as_ref()?;
                Some((
                    owner,
                    KiCadImportFootprintItem {
                        uuid: uuid.into(),
                        description: description.into(),
                    },
                ))
            })
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let footprint = resolved[0].0.footprint;
        if resolved
            .iter()
            .any(|(owner, _)| owner.footprint != footprint || (plated && !owner.plated))
            || resolved.iter().filter(|(owner, _)| owner.drilled).count() < drills
            || (items.len() == 2 && resolved[0].1.uuid.eq_ignore_ascii_case(&resolved[1].1.uuid))
        {
            continue;
        }
        let group = findings.entry((footprint, kind)).or_default();
        group.zero_annulus_on_unnumbered_pad |= kind == "annular_width"
            && resolved[0].0.unnumbered_unassigned
            && description
                .split_once("; actual ")
                .and_then(|(_, actual)| actual.strip_suffix(" mm)"))
                .and_then(|actual| actual.parse::<f64>().ok())
                == Some(0.0);
        group.evidence.push(KiCadImportFootprintEvidence {
            description: description.into(),
            items: resolved.into_iter().map(|(_, item)| item).collect(),
        });
    }
    findings
        .into_iter()
        .filter_map(|((owner, kind), group)| {
            let footprint = footprints[owner];
            Some(KiCadImportFootprintFinding {
                part: footprint_reference(footprint)?,
                footprint: footprint.children().get(1)?.atom()?.to_string(),
                kind: kind.into(),
                count: group.evidence.len(),
                evidence: group.evidence,
                action: action(kind, group.zero_annulus_on_unnumbered_pad),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BOARD: &str = r#"(kicad_pcb
        (footprint "lib:USB" (property "Reference" "J2")
            (pad "B12" smd rect (uuid "j2-pad"))
            (pad "" np_thru_hole circle (drill 0.6) (uuid "j2-hole"))
            (pad "S1" thru_hole circle (drill 0.3) (uuid "j2-shell")))
        (footprint "lib:USB" (property "Reference" "J2")
            (pad "" np_thru_hole circle (drill 0.6) (uuid "other-j2-hole"))))"#;
    const HOLE: &str = "Hole clearance violation (board setup constraints hole clearance 0.2540 mm; actual 0.1134 mm)";
    const ANNULAR: &str =
        "Annular width (board setup constraints min annular width 0.0750 mm; actual 0.0000 mm)";

    fn finding(kind: &str, description: &str, items: &[&str]) -> Value {
        json!({"type": kind, "severity": "error", "description": description,
            "items": items.iter().map(|uuid| json!({"uuid": uuid, "description": format!("KiCad item {uuid}") }))
                .collect::<Vec<_>>()})
    }

    #[test]
    fn input_hole_and_annular_findings_name_the_footprint_and_keep_native_evidence() {
        let hole = finding("hole_clearance", HOLE, &["j2-pad", "j2-hole"]);
        let ring = finding("annular_width", ANNULAR, &["j2-shell"]);
        let drc = json!({"violations": [hole.clone(), hole, ring]});
        let findings = footprint_findings(&parse(BOARD).unwrap(), &drc);
        let [ring, gap] = findings.as_slice() else {
            panic!("{findings:?}");
        };
        assert_eq!(
            (&*gap.part, &*gap.footprint, &*gap.kind, gap.count),
            ("J2", "lib:USB", "hole_clearance", 2)
        );
        assert_eq!(gap.evidence[0].description, HOLE);
        assert_eq!(gap.evidence[0].items[1].uuid, "j2-hole");
        assert!(gap.action.contains("copper and hole spacing"));
        assert_eq!(
            (&*ring.part, &*ring.kind, ring.count),
            ("J2", "annular_width", 1)
        );
        assert!(
            ring.action
                .contains("Preserve its pad number and electrical connection")
        );
    }

    #[test]
    fn the_unplated_option_requires_a_proven_unnumbered_unassigned_zero_annulus() {
        let facts = |pad_number, net, description| {
            let board = BOARD.replace(
                r#"(pad "S1" thru_hole circle (drill 0.3) (uuid "j2-shell"))"#,
                &format!(
                    r#"(pad "{pad_number}" thru_hole circle {net} (drill 0.3) (uuid "j2-shell"))"#
                ),
            );
            let drc = json!({"violations": [finding("annular_width", description, &["j2-shell"])]});
            footprint_findings(&parse(&board).unwrap(), &drc)
        };
        for (number, net, description, relevant) in [
            ("", "", ANNULAR, true),
            ("", "(net 0)", ANNULAR, true),
            ("", r#"(net "0")"#, ANNULAR, false),
            ("", r#"(net "GND")"#, ANNULAR, false),
            ("S1", "", ANNULAR, false),
            (
                "",
                "",
                "Annular width (board setup constraints min annular width 0.0750 mm; actual 0.0100 mm)",
                false,
            ),
        ] {
            let findings = facts(number, net, description);
            assert_eq!(findings.len(), 1);
            assert_eq!(findings[0].evidence[0].description, description);
            assert_eq!(
                findings[0]
                    .action
                    .contains("Use NPTH only if the drawing specifies"),
                relevant
            );
        }
    }

    #[test]
    fn contextual_synthetic_and_unowned_findings_do_not_become_input_advice() {
        let gap = finding("hole_clearance", HOLE, &["j2-pad", "j2-hole"]);
        let mut warning = gap.clone();
        warning["severity"] = "warning".into();
        let mut excluded = gap.clone();
        excluded["excluded"] = true.into();
        let drc = json!({"violations": [
            warning, excluded,
            // Even the same reference can mean two different instances.
            finding("hole_clearance", HOLE, &["j2-pad", "other-j2-hole"]),
            finding("hole_clearance", HOLE, &["j2-pad", "unknown-hole"]),
            finding("hole_clearance", HOLE, &["j2-hole", "j2-hole"]),
            finding("hole_clearance", "Hole clearance violation (rule 'In mounting area' hole clearance 0.5 mm; actual 0.1 mm)", &["j2-pad", "j2-hole"]),
            finding("annular_width", ANNULAR, &["j2-hole"]),
            finding("invalid_outline", "Board has malformed outline", &[]),
            finding("copper_edge_clearance", HOLE, &["j2-pad", "j2-hole"]),
            finding("clearance", HOLE, &["j2-pad", "j2-hole"]),
        ], "unconnected_items":[gap]});
        assert!(footprint_findings(&parse(BOARD).unwrap(), &drc).is_empty());
    }
}
