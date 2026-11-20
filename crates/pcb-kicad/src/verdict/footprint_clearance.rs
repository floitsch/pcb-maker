// Copyright (C) 2026 Florian Loitsch.

//! Evidence for different-net pad clearance inside an input footprint.

use super::{DrcFinding, DrcItem, is_pad_item, pad_footprint};
use crate::kicad_nets::{footprint_reference, footprint_uuid, item_net, net_codes};
use crate::{Expr, form_atom, parse};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

struct Pad<'a> {
    owner: &'a str,
    number: &'a str,
    net: &'a str,
}

fn named_pad(description: &str) -> Option<Pad<'_>> {
    if !is_pad_item(description) {
        return None;
    }
    let owner = pad_footprint(description)?;
    let (label, _) = description.rsplit_once("] of ")?;
    let (number, net) = label.split_once(" [")?;
    let number = number
        .strip_prefix("Pad ")
        .or_else(|| number.strip_prefix("PTH pad "))?;
    if number.is_empty()
        || number.contains(char::is_whitespace)
        || net.trim().is_empty()
        || net == "<no net>"
    {
        return None;
    }
    Some(Pad { owner, number, net })
}

fn millimetres(text: &str) -> Option<f64> {
    let mut fields = text.split_whitespace();
    let length: f64 = fields.next()?.parse().ok()?;
    let scale = match fields.next()? {
        "mm" => 1.0,
        "µm" | "μm" | "um" => 0.001,
        "mil" | "mils" => 0.0254,
        "in" | "inch" | "inches" => 25.4,
        _ => return None,
    };
    let length = length * scale;
    (fields.next().is_none() && length.is_finite() && length >= 0.0).then_some(length)
}

pub(super) fn clearance(description: &str) -> Option<(f64, f64)> {
    let (required, actual) = description.strip_suffix(')')?.rsplit_once("; actual ")?;
    let required = millimetres(required.rsplit_once("clearance ")?.1)?;
    let actual = millimetres(actual)?;
    (actual < required).then_some((actual, required))
}

fn board_footprint<'a>(
    board: &'a Expr,
    pads: [&Pad<'_>; 2],
    items: [&DrcItem; 2],
) -> Option<&'a str> {
    let codes = net_codes(board);
    let mut owners = board.children().iter().filter(|footprint| {
        matches!(footprint.head(), Some("footprint" | "module"))
            && footprint_reference(footprint).as_deref() == Some(pads[0].owner)
            && pads.iter().zip(items).all(|(expected, item)| {
                footprint
                    .children()
                    .iter()
                    .filter(|pad| {
                        pad.head() == Some("pad")
                            && pad.children().get(1).and_then(Expr::atom) == Some(expected.number)
                            && item_net(pad, &codes) == Some(expected.net)
                            && item.uuid.as_deref().is_none_or(|uuid| {
                                footprint_uuid(pad)
                                    .is_some_and(|actual| actual.eq_ignore_ascii_case(uuid))
                            })
                    })
                    .count()
                    == 1
            })
    });
    let footprint = owners.next()?;
    if owners.next().is_some() {
        return None;
    }
    footprint.children().get(1)?.atom()
}

fn source_library(project: &Path, id: &str) -> Option<PathBuf> {
    let (nickname, name) = id.split_once(':')?;
    if name.is_empty()
        || Path::new(name).file_name()?.to_str()? != name
        || name.contains(['\\', ':'])
    {
        return None;
    }
    let project = project.canonicalize().ok()?;
    let table = parse(&fs::read_to_string(project.join("fp-lib-table")).ok()?).ok()?;
    let mut libraries = table.children().iter().filter(|library| {
        library.head() == Some("lib") && form_atom(library, "name", 1) == Some(nickname)
    });
    let library = libraries.next()?;
    if libraries.next().is_some() || form_atom(library, "type", 1) != Some("KiCad") {
        return None;
    }
    let uri = form_atom(library, "uri", 1)?;
    let relative = uri
        .strip_prefix("${KIPRJMOD}/")
        .or_else(|| uri.strip_prefix("$(KIPRJMOD)/"))?;
    let path = Path::new(relative);
    if relative.contains(['$', '\\'])
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return None;
    }
    let path = project
        .join(path)
        .join(format!("{name}.kicad_mod"))
        .canonicalize()
        .ok()?;
    (path.starts_with(project) && path.is_file()).then_some(path)
}

/// A different-net pad pair below KiCad's copper clearance within one footprint.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KiCadFootprintClearance {
    pub reference: String,
    pub pads: [KiCadFootprintClearancePad; 2],
    pub measured_mm: f64,
    pub required_mm: f64,
    pub shortfall_mm: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_library_copy: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KiCadFootprintClearancePad {
    pub number: String,
    pub net: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
}

fn pad_pair(finding: &DrcFinding, board: Option<&Expr>) -> Option<KiCadFootprintClearance> {
    let [a, b] = finding.item_descriptions.as_slice() else {
        return None;
    };
    let [item_a, item_b] = finding.items.as_slice() else {
        return None;
    };
    let (a, b) = (named_pad(a)?, named_pad(b)?);
    if a.owner != b.owner || a.number == b.number || a.net == b.net {
        return None;
    }
    let library_id = match board {
        Some(board) => Some(board_footprint(board, [&a, &b], [item_a, item_b])?.to_owned()),
        None => None,
    };
    let (measured_mm, required_mm) = clearance(&finding.description)?;
    let reference = a.owner.to_owned();
    let mut pads = [(a, item_a), (b, item_b)].map(|(pad, item)| KiCadFootprintClearancePad {
        number: pad.number.to_owned(),
        net: pad.net.to_owned(),
        uuid: item.uuid.clone(),
    });
    pads.sort_by(|a, b| crate::canonical_order::natural_cmp(&a.number, &b.number));
    Some(KiCadFootprintClearance {
        reference,
        pads,
        measured_mm,
        required_mm,
        shortfall_mm: required_mm - measured_mm,
        library_id,
        resolved_library_copy: None,
    })
}

#[derive(Default)]
pub(super) struct Review {
    pub pairs: Vec<KiCadFootprintClearance>,
    unresolved: usize,
    omitted: usize,
}

impl Review {
    pub fn assess(
        findings: &[&DrcFinding],
        board_path: Option<&Path>,
        source_project: Option<&Path>,
    ) -> Self {
        const LIMIT: usize = 4;
        let copper: Vec<_> = findings
            .iter()
            .copied()
            .filter(|finding| finding.kind == "clearance")
            .collect();
        if copper.is_empty() {
            return Self::default();
        }
        let board = board_path
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|text| parse(&text).ok());
        let mut pairs = Vec::new();
        let mut unresolved = 0;
        for finding in copper {
            if let Some(pair) = pad_pair(finding, board.as_ref()) {
                if !pairs.contains(&pair) {
                    pairs.push(pair);
                }
            } else {
                unresolved += 1;
            }
        }
        pairs.sort_by(|a, b| {
            b.shortfall_mm
                .total_cmp(&a.shortfall_mm)
                .then_with(|| crate::canonical_order::natural_cmp(&a.reference, &b.reference))
                .then_with(|| {
                    crate::canonical_order::natural_cmp(&a.pads[0].number, &b.pads[0].number)
                })
                .then_with(|| {
                    crate::canonical_order::natural_cmp(&a.pads[1].number, &b.pads[1].number)
                })
        });
        let omitted = pairs.len().saturating_sub(LIMIT);
        pairs.truncate(LIMIT);
        let mut copies = BTreeMap::new();
        for pair in &mut pairs {
            if let (Some(project), Some(id)) = (source_project, &pair.library_id) {
                pair.resolved_library_copy = copies
                    .entry(id.clone())
                    .or_insert_with(|| source_library(project, id))
                    .clone();
            }
        }
        Self {
            pairs,
            unresolved,
            omitted,
        }
    }

    pub fn note(&self) -> Option<String> {
        if self.pairs.is_empty() && self.unresolved == 0 {
            return None;
        }
        let mut note = String::new();
        let mut libraries = BTreeMap::new();
        let measure = |mm: f64| {
            if mm > 0.0 && mm < 0.000_001 {
                format!("{mm:.3e}")
            } else if mm < 0.0001 {
                format!("{mm:.6}")
            } else {
                format!("{mm:.4}")
            }
        };
        let shown: Vec<_> = self
            .pairs
            .iter()
            .map(|pair| {
                if let Some(id) = pair.library_id.as_deref() {
                    libraries.insert(id, pair.resolved_library_copy.as_deref());
                }
                let [a, b] = &pair.pads;
                format!(
                    "{}:{} ({}) to {}:{} ({}), {} mm measured, {} mm required, {} mm short",
                    pair.reference,
                    a.number,
                    a.net,
                    pair.reference,
                    b.number,
                    b.net,
                    measure(pair.measured_mm),
                    measure(pair.required_mm),
                    measure(pair.shortfall_mm),
                )
            })
            .collect();
        if !shown.is_empty() {
            note.push_str(&format!(
                " Different-net pad clearance is below the KiCad requirement for {}.",
                shown.join(" | ")
            ));
        }
        if self.omitted > 0 {
            note.push_str(&format!(
                " KiCad reported {} more pad pairs in drc.json.",
                self.omitted
            ));
        }
        if self.unresolved > 0 {
            note.push_str(&format!(
                " For {} of these findings, review the items and applicable clearance in drc.json to confirm the pads, nets and measured gap.", self.unresolved
            ));
        }
        for (id, copy) in libraries {
            if let Some(path) = copy {
                note.push_str(&format!(
                    " The resolved project-library copy of {id} is {}.",
                    path.display()
                ));
            } else {
                note.push_str(&format!(" Locate {id} in the source footprint library."));
            }
        }
        note.push_str(" Verify the assigned pad nets against the schematic. Check the copper pad shapes, including any custom primitives, against the part's land pattern and the approved fabrication clearance. Make any lasting correction in the source footprint library or board donor used for import. Preserve pad numbers and net assignments, update the footprint in the PCB, and rerun KiCad DRC.");
        Some(note)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drc_finding;
    use serde_json::json;

    fn finding(actual: &str, requirement: &str, examples: [&str; 2]) -> DrcFinding {
        drc_finding(&json!({
            "type": "clearance",
            "description": format!("Clearance violation (rule 'fine pitch' clearance {requirement}; actual {actual})"),
            "items": examples.map(|description| json!({"description": description}))
        }))
    }

    #[test]
    fn named_copper_pads_report_the_normalized_gap_and_shortfall() {
        let finding = finding(
            "3 mils",
            "90 µm",
            [
                "Pad 2 [GND] of U10 on F.Cu",
                "Pad 1 [/OSD/VIDEO [1]] of U10 on F.Cu",
            ],
        );
        let review = Review::assess(&[&finding], None, None);
        let pair = &review.pairs[0];
        assert_eq!(pair.reference, "U10");
        assert_eq!(pair.pads[0].number, "1");
        assert_eq!(pair.pads[0].net, "/OSD/VIDEO [1]");
        assert!((pair.measured_mm - 0.0762).abs() < 1e-12);
        assert!((pair.required_mm - 0.09).abs() < 1e-12);
        assert!((pair.shortfall_mm - 0.0138).abs() < 1e-12);
        assert_eq!(pair.library_id, None);
        let note = review.note().unwrap();
        assert!(
            note.contains("U10:1 (/OSD/VIDEO [1]) to U10:2 (GND)"),
            "{note}"
        );
        assert!(
            note.contains("0.0762 mm measured, 0.0900 mm required, 0.0138 mm short"),
            "{note}"
        );
        assert_eq!(millimetres("0.01 in"), Some(0.254));
        for unsafe_value in ["1", "1 unknown", "NaN mm", "-1 mils", "1 mm comment"] {
            assert_eq!(millimetres(unsafe_value), None);
        }
        assert_eq!(
            clearance("Clearance violation ( clearance 0.0900 mm; actual 0.0900 mm)"),
            None
        );
    }

    #[test]
    fn ambiguous_named_pads_and_missing_units_have_no_specific_claim() {
        for item in [
            "Pad 1 [VCC] of U11 on F.Cu",
            "Pad 3 [GND] of U10",
            "Pad 1 [<no net>] of U10",
            "Segment [VCC] of U10",
            "Pad [VCC] of U10",
            "Pad 2 [VCC] of U10",
        ] {
            let finding = finding("0.08 mm", "0.09 mm", [item, "Pad 2 [GND] of U10"]);
            assert!(pad_pair(&finding, None).is_none(), "{item}");
        }
        let uncertain = finding(
            "0.08",
            "0.09 mm",
            ["Pad 1 [VCC] of U10", "Pad 2 [GND] of U10"],
        );
        let action =
            super::super::footprint_findings_note(&[&uncertain], "board.kicad_dru").unwrap();
        assert!(action.starts_with(super::super::INPUT_FOOTPRINT_REVIEW_TAG));
        assert!(
            action.contains("review the items and applicable clearance in drc.json"),
            "{action}"
        );
        assert!(
            !action.contains("(rule ") && !action.contains("required,"),
            "{action}"
        );
    }

    #[test]
    fn the_board_resolves_a_repeated_footprints_pads_by_uuid() {
        let board = parse(
            r#"(kicad_pcb
          (footprint "lib:Actual" (property "Reference" "U10")
            (pad "1" smd rect (net "VCC") (uuid "first"))
            (pad "2" smd rect (net "GND") (uuid "second")))
          (footprint "lib:Different" (property "Reference" "U10")
            (pad "1" smd rect (net "VCC") (uuid "different-first"))
            (pad "2" smd rect (net "GND") (uuid "different-second"))))"#,
        )
        .unwrap();
        let mut finding = finding(
            "0.08 mm",
            "0.09 mm",
            ["Pad 1 [VCC] of U10", "Pad 2 [GND] of U10"],
        );
        assert!(pad_pair(&finding, Some(&board)).is_none());
        finding.items[0].uuid = Some("FIRST".into());
        finding.items[1].uuid = Some("second".into());
        assert_eq!(
            pad_pair(&finding, Some(&board))
                .unwrap()
                .library_id
                .as_deref(),
            Some("lib:Actual")
        );
        finding.items[1].uuid = Some("different-second".into());
        assert!(pad_pair(&finding, Some(&board)).is_none());
    }
}
