// Copyright (C) 2026 Florian Loitsch.

use super::*;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadHeldPad {
    pub reference: String,
    pub number: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    pub uuid: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadHeldPadShort {
    pub references: [String; 2],
    pub footprint_uuids: [String; 2],
    pub shorting_items: usize,
    pub samples: Vec<[KiCadHeldPad; 2]>,
}

struct HeldPad {
    footprint_uuid: String,
    pad: KiCadHeldPad,
}

/// Match native findings to unambiguous objects in the saved PCB.
pub(super) struct HeldGeometry {
    route_only: bool,
    footprints: BTreeMap<String, Option<String>>,
    pads: BTreeMap<String, Option<HeldPad>>,
    board_edges: BTreeSet<String>,
}

impl HeldGeometry {
    pub(super) fn load(facts: &KiCadRunFacts) -> Option<Self> {
        let route_only = facts.command == "route-kicad-board";
        let brief = facts.authored_layout_constraints.as_ref();
        let board = crate::parse(&fs::read_to_string(facts.outputs.get("board")?).ok()?).ok()?;
        let codes = crate::kicad_nets::net_codes(&board);
        let items = board
            .children()
            .iter()
            .filter(|item| matches!(item.head(), Some("footprint" | "module")));
        let mut references = BTreeMap::<_, usize>::new();
        for reference in items.clone().filter_map(crate::footprint_reference) {
            *references.entry(reference).or_default() += 1;
        }
        let mut footprints = BTreeMap::new();
        let mut pads = BTreeMap::new();
        for footprint in items {
            let identity =
                crate::footprint_reference(footprint).zip(crate::footprint_uuid(footprint));
            let held_identity = identity.as_ref().and_then(|(reference, uuid)| {
                let held = route_only
                    || (facts.fixed_parts.contains(reference)
                        && (crate::kicad_nets::footprint_locked(footprint)
                            || brief.is_some_and(|brief| {
                                holds(brief, reference, uuid, references[reference] == 1)
                            })));
                held.then_some((reference, uuid))
            });
            for pad in footprint
                .children()
                .iter()
                .filter(|item| item.head() == Some("pad"))
            {
                if let Some(uuid) = crate::footprint_uuid(pad) {
                    pads.entry(uuid.to_ascii_lowercase())
                        .and_modify(|owner| *owner = None)
                        .or_insert_with(|| {
                            let (reference, footprint_uuid) = held_identity?;
                            Some(HeldPad {
                                footprint_uuid: footprint_uuid.to_ascii_lowercase(),
                                pad: KiCadHeldPad {
                                    reference: reference.clone(),
                                    number: pad.children().get(1)?.atom()?.into(),
                                    net: crate::kicad_nets::item_net(pad, &codes)
                                        .filter(|net| !net.is_empty())
                                        .map(|net| crate::unescape(net).into_owned()),
                                    uuid,
                                },
                            })
                        });
                }
            }
            if let Some((_, uuid)) = &identity {
                footprints
                    .entry(uuid.to_ascii_lowercase())
                    .and_modify(|entry| *entry = None)
                    .or_insert_with(|| held_identity.map(|(reference, _)| reference.clone()));
            }
        }
        let mut edges = BTreeMap::<_, usize>::new();
        for item in board.children().iter().filter(|item| {
            matches!(
                item.head(),
                Some("gr_line" | "gr_arc" | "gr_circle" | "gr_rect" | "gr_poly" | "gr_curve")
            ) && crate::form_atom(item, "layer", 1) == Some("Edge.Cuts")
        }) {
            if let Some(uuid) = crate::footprint_uuid(item) {
                *edges.entry(uuid.to_ascii_lowercase()).or_default() += 1;
            }
        }
        let board_edges = edges
            .into_iter()
            .filter_map(|(uuid, count)| (count == 1).then_some(uuid))
            .collect();
        Some(Self {
            route_only,
            footprints,
            pads,
            board_edges,
        })
    }

    pub(super) fn shorts(&self, findings: &[DrcFinding]) -> Vec<KiCadHeldPadShort> {
        let mut groups = BTreeMap::<_, KiCadHeldPadShort>::new();
        for finding in findings
            .iter()
            .filter(|finding| finding.kind == "shorting_items")
        {
            let ([a, b], [name_a, name_b]) = (
                finding.items.as_slice(),
                finding.item_descriptions.as_slice(),
            ) else {
                continue;
            };
            let Some((a, b)) = self.short_pad(a, name_a).zip(self.short_pad(b, name_b)) else {
                continue;
            };
            if a.footprint_uuid == b.footprint_uuid || a.pad.net == b.pad.net {
                continue;
            }
            let mut pair = [a, b];
            pair.sort_by_key(|pad| (&pad.pad.reference, &pad.footprint_uuid));
            let ids = pair.map(|pad| pad.footprint_uuid.clone());
            let group = groups
                .entry(ids.clone())
                .or_insert_with(|| KiCadHeldPadShort {
                    references: pair.map(|pad| pad.pad.reference.clone()),
                    footprint_uuids: ids,
                    shorting_items: 0,
                    samples: Vec::new(),
                });
            group.shorting_items += 1;
            if group.samples.len() < 3 {
                group.samples.push(pair.map(|pad| pad.pad.clone()));
            }
        }
        let mut groups: Vec<_> = groups.into_values().collect();
        groups.sort_by(|a, b| {
            b.shorting_items
                .cmp(&a.shorting_items)
                .then(a.references.cmp(&b.references))
        });
        groups.truncate(12);
        groups
    }

    fn short_pad(&self, item: &DrcItem, description: &str) -> Option<&HeldPad> {
        let saved = self
            .pads
            .get(&item.uuid.as_ref()?.to_ascii_lowercase())?
            .as_ref()?;
        let (reference, number) = report_pad(description)?;
        let net = item_net(description)?;
        (reference == saved.pad.reference
            && number == saved.pad.number
            && saved.pad.net.as_deref() == Some(net.as_str()))
        .then_some(saved)
    }

    pub(super) fn courtyard_action<'a>(
        &self,
        findings: &[&'a DrcFinding],
    ) -> Option<(String, Vec<&'a DrcFinding>)> {
        let mut pairs = BTreeSet::new();
        let mut remaining = Vec::new();
        for &finding in findings {
            if let Some(pair) = pair(finding, &self.footprints) {
                pairs.insert(pair);
            } else {
                remaining.push(finding);
            }
        }
        if pairs.is_empty() {
            return None;
        }
        let mut examples: Vec<_> = pairs
            .iter()
            .take(4)
            .map(|(a, b)| format!("{a} and {b}"))
            .collect();
        if pairs.len() > examples.len() {
            examples.push(format!("{} more pairs", pairs.len() - examples.len()));
        }
        let review = if self.route_only {
            "Inspect the source footprint courtyards and their poses in the input board. Correct inaccurate footprint geometry or revise an input pose before rerouting."
        } else {
            "Inspect the source footprint courtyards and the held interface poses. Correct inaccurate footprint geometry or revise a held pose in the mechanical brief before rerunning layout."
        };
        Some((
            format!(
                "courtyards_overlap {} at held footprint pairs {}. {review}",
                findings.len() - remaining.len(),
                examples.join(", ")
            ),
            remaining,
        ))
    }

    pub(super) fn edge_action<'a>(
        &self,
        findings: &[&'a DrcFinding],
    ) -> Option<(String, Vec<&'a DrcFinding>)> {
        let mut examples = BTreeSet::new();
        let mut remaining = Vec::new();
        for &finding in findings {
            if let Some(pad) = self.edge_pad(finding) {
                let mut text = format!("{}:{}", pad.reference, pad.number);
                if let Some((measured, required)) =
                    super::footprint_clearance::clearance(&finding.description)
                {
                    text.push_str(&format!(
                        " ({measured:.4} mm measured, {required:.4} mm required)"
                    ));
                }
                examples.insert(text);
            } else {
                remaining.push(finding);
            }
        }
        if examples.is_empty() {
            return None;
        }
        let references = examples.into_iter().collect::<Vec<_>>();
        let source = if self.route_only {
            "held input pose"
        } else {
            "held pose in the mechanical brief"
        };
        Some((
            format!(
                "copper_edge_clearance {} at held pads near board Edge.Cuts: {}. Inspect the source footprint's pad geometry, the {source}, and the outline. If the geometry must change, revise the source footprint or the interface pose or outline permitted by the enclosure, then rerun KiCad verification.",
                findings.len() - remaining.len(),
                super::parity::brief(&references, 4),
            ),
            remaining,
        ))
    }

    fn edge_pad(&self, finding: &DrcFinding) -> Option<&KiCadHeldPad> {
        let [a, b] = finding.items.as_slice() else {
            return None;
        };
        let [first, second] = finding.item_descriptions.as_slice() else {
            return None;
        };
        for ((edge, edge_label), (pad, pad_label)) in
            [((a, first), (b, second)), ((b, second), (a, first))]
        {
            if !edge_label.ends_with(" on Edge.Cuts") || edge_label.contains(" of ") {
                continue;
            }
            let id = edge.uuid.as_ref()?.to_ascii_lowercase();
            if !self.board_edges.contains(&id) {
                continue;
            }
            let reported = report_pad(pad_label)?;
            let saved = self
                .pads
                .get(&pad.uuid.as_ref()?.to_ascii_lowercase())?
                .as_ref()?;
            return (reported.0 == saved.pad.reference && reported.1 == saved.pad.number)
                .then_some(&saved.pad);
        }
        None
    }
}

fn holds(
    brief: &crate::KiCadPlacementConstraints,
    reference: &str,
    uuid: &str,
    unique: bool,
) -> bool {
    brief
        .fixed
        .iter()
        .any(|pattern| crate::kicad_nets::glob_matches(pattern, reference))
        || brief.place.iter().any(|entry| {
            entry.part == reference
                && entry.x.is_some()
                && entry.y.is_some()
                && entry
                    .uuid
                    .as_ref()
                    .map_or(unique, |id| id.eq_ignore_ascii_case(uuid))
        })
        || (unique
            && brief
                .add
                .iter()
                .any(|entry| entry.part == reference && entry.x.is_some() && entry.y.is_some()))
}

fn pair<'a>(
    finding: &DrcFinding,
    held: &'a BTreeMap<String, Option<String>>,
) -> Option<(&'a str, &'a str)> {
    let ([a, b], [a_name, b_name]) = (
        finding.items.as_slice(),
        finding.item_descriptions.as_slice(),
    ) else {
        return None;
    };
    let [a, b] = [(a, a_name), (b, b_name)].map(|(item, label)| {
        let name = held
            .get(&item.uuid.as_deref()?.to_ascii_lowercase())?
            .as_deref()?;
        label
            .strip_prefix("Footprint ")
            .filter(|observed| *observed == name)
            .map(|_| name)
    });
    match (a?, b?) {
        (a, b) if a < b => Some((a, b)),
        (a, b) if b < a => Some((b, a)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_native_short_between_held_pads_names_the_exact_pair() {
        let path = crate::fs_util::temporary_next_to(
            &std::env::temp_dir().join("pcb-maker-held-short.kicad_pcb"),
        );
        fs::write(
            &path,
            r#"(kicad_pcb (net 2 "USB_D-")
              (footprint "Test:Connector" (uuid "j5") (locked yes) (property "Reference" "J5")
                (pad "35" smd rect (net 2) (uuid "p1")))
              (footprint "Test:IC" (uuid "u30") (property "Reference" "U30")
                (pad "A1" smd rect (net "QA0") (uuid "p2"))))"#,
        )
        .unwrap();
        let first = json!({"description":"Pad 35 [USB_D-] of J5 on F.Cu", "uuid":"p1"});
        let second = json!({"description":"Pad A1 [QA0] of U30 on F.Cu", "uuid":"p2"});
        let finding = |a, b| {
            json!({
                "type":"shorting_items", "description":"Items shorting two nets", "severity":"error",
                "items":[a,b]
            })
        };
        let native = |items| tally_drc(&json!({"violations":items,"unconnected_items":[]}));
        let mut facts = KiCadRunFacts::new("layout-kicad-board");
        facts
            .outputs
            .insert("board".into(), path.display().to_string());
        facts.fixed_parts = ["J5".into(), "U30".into()].into();
        facts.authored_layout_constraints = Some(
            serde_json::from_value(json!({
                "version":1, "place":[{"part":"U30","origin":"absolute","x":12,"y":34}]
            }))
            .unwrap(),
        );
        facts.input_warnings = vec!["fixed parts J5 and U30 overlap".into()];
        facts.native = Some(native(vec![
            finding(first.clone(), second.clone()),
            finding(
                first.clone(),
                json!({"description":"Pad A1 [BAD] of U30 on F.Cu", "uuid":"p2"}),
            ),
        ]));
        let result = judge(&facts);
        assert_eq!(result.status, KiCadRunStatus::Incomplete);
        let [pair] = result.held_pad_shorts.as_slice() else {
            panic!()
        };
        assert_eq!(pair.references, ["J5", "U30"]);
        assert_eq!(pair.shorting_items, 1);
        assert!(
            result.next_actions[0].contains("J5:35 [USB_D-]")
                && result.next_actions[0].contains("U30:A1 [QA0]")
        );
        assert!(
            !result
                .next_actions
                .join("\n")
                .contains("costs no connection")
        );
        facts.fixed_parts.retain(|part| part != "U30");
        assert!(judge(&facts).held_pad_shorts.is_empty());
        facts.native = None;
        let unknown = judge(&facts);
        assert!(unknown.held_pad_shorts.is_empty());
        assert!(
            unknown
                .next_actions
                .iter()
                .any(|line| line.contains("review held placement") && line.contains("J5 and U30"))
        );
        assert!(
            !unknown
                .next_actions
                .join("\n")
                .contains("costs no connection")
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_held_courtyard_pair_keeps_its_error_and_names_its_mechanical_fix() {
        let path = std::env::temp_dir().join(format!(
            "pcb-maker-held-courtyard-{}.kicad_pcb",
            std::process::id()
        ));
        fs::write(
            &path,
            r#"(kicad_pcb
              (footprint "Test:Hole" (uuid "hole") (property "Reference" "H4"))
              (footprint "Test:Socket" (uuid "socket") (property "Reference" "J2"))
              (footprint "Test:Resistor" (uuid "resistor") (property "Reference" "R1")))"#,
        )
        .unwrap();
        let finding = |first: &str, first_id: &str, second: &str, second_id: &str| {
            json!({
                "type": "courtyards_overlap", "description": "Courtyards overlap", "severity": "error",
                "items": [
                    {"description": first, "uuid": first_id, "pos": {"x": 1.0, "y": 2.0}},
                    {"description": second, "uuid": second_id, "pos": {"x": 2.0, "y": 2.0}}
                ]
            })
        };
        let fixed = finding("Footprint H4", "hole", "Footprint J2", "socket");
        let movable = finding("Footprint J2", "socket", "Footprint R1", "resistor");
        let ambiguous = finding("Footprint H4", "different", "Footprint J2", "socket");
        let mut facts = KiCadRunFacts::new("layout-kicad-board");
        facts
            .outputs
            .insert("board".into(), path.display().to_string());
        facts.fixed_parts = ["H4".into(), "J2".into()].into();
        facts.authored_layout_constraints = Some(
            serde_json::from_value(json!({
                "version": 1, "fixed": ["H4"],
                "place": [{"part": "J2", "x": 2.0, "y": 2.0, "origin": "absolute"}]
            }))
            .unwrap(),
        );
        facts.native = Some(tally_drc(&json!({"violations": [fixed]})));
        let verdict = judge(&facts);
        assert_eq!(verdict.new_findings_strict["courtyards_overlap"], 1);
        assert_eq!(verdict.exit_code, EXIT_INCOMPLETE);
        let actions = verdict.next_actions.join("\n");
        assert!(
            actions.contains("held footprint pairs H4 and J2"),
            "{actions}"
        );
        assert!(actions.contains("revise a held pose"), "{actions}");
        assert!(!actions.contains("tight_bodies"), "{actions}");

        facts.native = Some(tally_drc(
            &json!({"violations": [fixed, movable, ambiguous]}),
        ));
        let verdict = judge(&facts);
        assert_eq!(verdict.new_findings_strict["courtyards_overlap"], 3);
        let actions = verdict.next_actions.join("\n");
        assert!(actions.contains("courtyards_overlap 1 at held footprint pairs H4 and J2"));
        assert!(actions.contains(
            "courtyards_overlap 2 (first: Footprint J2; Footprint R1): placer.tight_bodies"
        ));
        facts.command = "route-kicad-board".into();
        facts.native = Some(tally_drc(&json!({"violations": [fixed]})));
        let verdict = judge(&facts);
        let actions = verdict.next_actions.join("\n");
        assert!(actions.contains("revise an input pose before rerouting"));
        assert!(!actions.contains("tight_bodies"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn held_edge_advice_requires_a_saved_board_edge_and_the_held_pad() {
        let path = std::env::temp_dir().join(format!(
            "pcb-maker-held-edge-{}.kicad_pcb",
            std::process::id(),
        ));
        fs::write(
            &path,
            r#"(kicad_pcb
            (gr_line (start 0 0) (end 0 1) (layer "Edge.Cuts") (uuid "edge"))
            (footprint "Test:Connector" (uuid "connector") (property "Reference" "J1")
                (fp_line (start 0 0) (end 0 1) (layer "Edge.Cuts") (uuid "slot"))
                (pad "15" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (uuid "held")))
            (footprint "Test:Resistor" (uuid "resistor") (property "Reference" "R1")
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (uuid "movable"))))"#,
        )
        .unwrap();
        let edge = |shape: &str, id: &str, label: &str, pad: &str, rule: &str| {
            json!({
                "type": "copper_edge_clearance", "severity": "error", "description": rule,
                "items": [
                    {"description": shape, "uuid": id, "pos": {"x": 2.0, "y": 1.0}},
                    {"description": label, "uuid": pad, "pos": {"x": 2.2, "y": 1.0}}
                ]
            })
        };
        let rule = "Board edge clearance violation (board setup constraints edge clearance 0.3720 mm; actual 0.2164 mm)";
        let held = edge(
            "Segment on Edge.Cuts",
            "edge",
            "Pad 15 [GND] of J1 on F.Cu",
            "held",
            rule,
        );
        let movable = edge(
            "Segment on Edge.Cuts",
            "edge",
            "Pad 1 [GND] of R1 on F.Cu",
            "movable",
            rule,
        );
        let copper = edge(
            "Segment on Edge.Cuts",
            "edge",
            "Track [GND] on F.Cu",
            "track",
            rule,
        );
        let slot = edge(
            "Segment of J1 on Edge.Cuts",
            "slot",
            "Pad 15 [GND] of J1 on F.Cu",
            "held",
            rule,
        );
        let mismatch = edge(
            "Segment on Edge.Cuts",
            "edge",
            "Pad 99 [GND] of J1 on F.Cu",
            "held",
            rule,
        );
        let mut facts = KiCadRunFacts::new("layout-kicad-board");
        facts
            .outputs
            .insert("board".into(), path.display().to_string());
        facts.fixed_parts.insert("J1".into());
        facts.authored_layout_constraints = Some(
            serde_json::from_value(json!({
                "version": 1, "place": [{"part": "J1", "x": 2.0, "y": 2.0, "origin": "absolute"}]
            }))
            .unwrap(),
        );
        facts.native = Some(tally_drc(&json!({"violations": [held.clone()]})));
        let verdict = judge(&facts);
        assert_eq!(verdict.exit_code, EXIT_INCOMPLETE);
        assert_eq!(verdict.new_findings_strict["copper_edge_clearance"], 1);
        let actions = verdict.next_actions.join("\n");
        assert!(
            actions.contains("J1:15 (0.2164 mm measured, 0.3720 mm required)"),
            "{actions}"
        );
        assert!(
            actions.contains("held pose in the mechanical brief"),
            "{actions}"
        );
        assert!(!actions.contains("max_mm or flush"), "{actions}");

        let room = HeldGeometry::load(&facts).unwrap();
        let mixed =
            tally_drc(&json!({"violations": [held, movable.clone(), copper, slot, mismatch]}));
        let findings: Vec<_> = mixed.findings.iter().collect();
        let (_, remaining) = room.edge_action(&findings).unwrap();
        assert_eq!(remaining.len(), 4);
        facts.native = Some(mixed);
        let actions = judge(&facts).next_actions.join("\n");
        assert!(
            actions.contains("copper_edge_clearance 1 at held pads"),
            "{actions}"
        );
        assert!(actions.contains("copper_edge_clearance 4"), "{actions}");
        assert!(actions.contains("max_mm or flush"), "{actions}");

        facts.command = "route-kicad-board".into();
        facts.authored_layout_constraints = None;
        facts.native = Some(tally_drc(&json!({"violations": [movable]})));
        let actions = judge(&facts).next_actions.join("\n");
        assert!(actions.contains("held input pose"), "{actions}");
        assert!(actions.contains("R1:1"), "{actions}");
        let _ = fs::remove_file(path);
    }
}
