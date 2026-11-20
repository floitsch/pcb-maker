// Copyright (C) 2026 Florian Loitsch.

use super::*;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadTranslatedEndpoint {
    pub reference: String,
    pub pad: String,
    pub pad_at_mm: [f64; 2],
    pub footprint_at_mm: [f64; 2],
    pub rotation_degrees: f64,
    pub side: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadTranslatedConnection {
    pub open_endpoints: [KiCadTranslatedEndpoint; 2],
    pub routed_net: String,
    pub routed_endpoints: [KiCadTranslatedEndpoint; 2],
    /// The offset from the routed pair to the open pair.
    pub translation_mm: [f64; 2],
    pub next_action: &'static str,
}

#[derive(Clone, Copy)]
struct Footprint {
    shape: usize,
    at: [f64; 3],
    side: &'static str,
}

fn footprint_shapes(pcb: &Expr) -> BTreeMap<String, Option<Footprint>> {
    let mut shapes = BTreeMap::new();
    let mut references = BTreeMap::new();
    for node in pcb
        .children()
        .iter()
        .filter(|node| node.head() == Some("footprint"))
    {
        let Some(reference) = footprint_reference(node).filter(|reference| !reference.is_empty())
        else {
            continue;
        };
        if let Some(previous) = references.get_mut(&reference) {
            *previous = None;
            continue;
        }
        // Keep the library identity, pad positions, construction, layers
        // and pin roles. Ignore only the individual pad identity and net.
        let footprint = (|| {
            let library = node.children().get(1)?.atom()?.to_string();
            if library.is_empty() {
                return None;
            }
            let mut at = form_at(node).ok()?;
            at[2] = at[2].rem_euclid(360.0) + 0.0;
            let side = match form_atom(node, "layer", 1)? {
                "F.Cu" => "front",
                "B.Cu" => "back",
                _ => return None,
            };
            let mut pads = node
                .children()
                .iter()
                .filter(|pad| pad.head() == Some("pad"))
                .map(|pad| {
                    encode_canonical(&Expr::List(
                        pad.children()
                            .iter()
                            .filter(|field| {
                                !matches!(field.head(), Some("uuid" | "tstamp" | "net"))
                            })
                            .cloned()
                            .collect(),
                    ))
                })
                .collect::<Vec<_>>();
            pads.sort_unstable();
            let next = shapes.len();
            let shape = *shapes.entry((library, pads)).or_insert(next);
            Some(Footprint { shape, at, side })
        })();
        references.insert(reference, footprint);
    }
    references
}

#[derive(Clone, Copy)]
struct Endpoint<'a> {
    pad: &'a BoardPad,
    footprint: Footprint,
}

impl<'a> Endpoint<'a> {
    fn key(&self) -> (usize, &'a str, &'static str, u64) {
        (
            self.footprint.shape,
            &self.pad.number,
            self.footprint.side,
            self.footprint.at[2].to_bits(),
        )
    }

    fn xy(&self) -> [i64; 2] {
        [
            native_units(self.footprint.at[0]),
            native_units(self.footprint.at[1]),
        ]
    }

    fn report(self) -> KiCadTranslatedEndpoint {
        let [x, y, rotation_degrees] = self.footprint.at;
        KiCadTranslatedEndpoint {
            reference: self.pad.reference.clone(),
            pad: self.pad.number.clone(),
            pad_at_mm: self.pad.at.map(round),
            footprint_at_mm: [x, y],
            rotation_degrees,
            side: self.footprint.side,
        }
    }
}

fn native_units(mm: f64) -> i64 {
    (mm * 1_000_000.0).round() as i64
}

fn delta(from: [i64; 2], to: [i64; 2]) -> [i64; 2] {
    [to[0] - from[0], to[1] - from[1]]
}

fn exact_open(ends: [Endpoint<'_>; 2], pairs: &[KiCadNativePair]) -> bool {
    let [pair] = pairs else { return false };
    let [a, b] = pair.items.as_slice() else {
        return false;
    };
    if !pair.retained
        || a.uuid == b.uuid
        || !a.description.starts_with("Pad ")
        || !b.description.starts_with("Pad ")
    {
        return false;
    }
    ends.iter().all(|end| {
        [a, b]
            .iter()
            .any(|item| item.uuid == end.pad.uuid && distance_squared(item.at, end.pad.at) < 1.0e-6)
    })
}

fn routed(result: &LoweredBoard, net: &str, ends: [Endpoint<'_>; 2]) -> bool {
    let Some(id) = result.net_id(net) else {
        return false;
    };
    let copper = net_copper(&result.pcb, &result.names, net);
    if copper.segments + copper.arcs == 0 {
        return false;
    }
    let groups = result.components(id);
    let [group] = groups.as_slice() else {
        return false;
    };
    ends.iter()
        .all(|end| group.pads.iter().any(|pad| pad.uuid == end.pad.uuid))
}

pub(super) fn diagnose(
    result: &LoweredBoard,
    drc: &serde_json::Value,
    opens: &BTreeMap<String, Vec<KiCadNativePair>>,
    selected: &[String],
) -> BTreeMap<String, KiCadTranslatedConnection> {
    if !selected.iter().any(|net| opens.contains_key(net))
        || !drc["included_severities"]
            .as_array()
            .is_some_and(|checked| {
                ["error", "warning", "exclusion"]
                    .iter()
                    .all(|level| checked.iter().any(|value| value == *level))
            })
        || drc["ignored_checks"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|check| check["key"] == "unconnected_items")
    {
        return BTreeMap::new();
    }

    let footprints = footprint_shapes(&result.pcb);
    let mut terminals: BTreeMap<&str, Vec<&BoardPad>> = BTreeMap::new();
    let mut by_number = BTreeMap::new();
    for pad in &result.pads {
        *by_number.entry((&pad.reference, &pad.number)).or_insert(0) += 1;
        if let Some(net) = pad.net.as_deref() {
            terminals.entry(net).or_default().push(pad);
        }
    }
    let mut matches = BTreeMap::new();
    let mut connections = BTreeMap::new();
    for (net, pads) in terminals {
        let [a, b] = pads.as_slice() else { continue };
        if a.reference == b.reference
            || a.uuid == b.uuid
            || [a, b]
                .iter()
                .any(|pad| pad.uuid.is_none() || by_number[&(&pad.reference, &pad.number)] != 1)
        {
            continue;
        }
        let (Some(Some(first)), Some(Some(second))) =
            (footprints.get(&a.reference), footprints.get(&b.reference))
        else {
            continue;
        };
        let mut ends = [
            Endpoint {
                pad: a,
                footprint: *first,
            },
            Endpoint {
                pad: b,
                footprint: *second,
            },
        ];
        ends.sort_by_key(|end| (end.key(), end.xy()));
        let key = (ends.map(|end| end.key()), delta(ends[0].xy(), ends[1].xy()));
        matches.entry(key).or_insert_with(Vec::new).push(net);
        connections.insert(net, (ends, key));
    }

    selected.iter().filter_map(|net| {
        let (ends, key) = *connections.get(net.as_str())?;
        if !exact_open(ends, opens.get(net)?) {
            return None;
        }
        // Multiple valid comparison channels make this choice ambiguous.
        let mut candidates = matches[&key].iter().filter_map(|candidate| {
            let candidate_ends = connections[candidate].0;
            if opens.contains_key(*candidate) || candidate_ends.iter().any(|other| {
                ends.iter().any(|end| end.pad.reference == other.pad.reference)
            }) || !routed(result, candidate, candidate_ends) {
                return None;
            }
            Some((candidate, candidate_ends))
        });
        let (routed_net, routed_ends) = candidates.next()?;
        if candidates.next().is_some() {
            return None;
        }
        Some((net.clone(), KiCadTranslatedConnection {
            open_endpoints: ends.map(Endpoint::report),
            routed_net: routed_net.to_string(),
            routed_endpoints: routed_ends.map(Endpoint::report),
            translation_mm: delta(routed_ends[0].xy(), ends[0].xy()).map(|coord| coord as f64 / 1_000_000.0),
            next_action: "Compare pad escapes and nearby copper at the open endpoints with the routed endpoints.",
        }))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BOARD: &str = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
      (general (thickness 1.6)) (paper "A4")
      (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
      (gr_rect (start 0 0) (end 20 20) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))
      (footprint "Chip" (layer "F.Cu") (at 2 5) (property "Reference" "U1")
        (pad "1" smd rect (at 0 0) (size 0.8 1) (layers "F.Cu") (net "closed") (uuid "one")))
      (footprint "Flash" (layer "F.Cu") (at 12 5) (property "Reference" "U2")
        (pad "2" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "closed") (uuid "two")))
      (footprint "Chip" (layer "F.Cu") (at 4 15) (property "Reference" "U3")
        (pad "1" smd rect (at 0 0) (size 0.8 1) (layers "F.Cu") (net "broken") (uuid "three")))
      (footprint "Flash" (layer "F.Cu") (at 14 15) (property "Reference" "U4")
        (pad "2" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "broken") (uuid "four")))
      (segment (start 2 5) (end 12 5) (width 0.2) (layer "F.Cu") (net "closed")))"#;

    fn inspect(
        board: &str,
        end_id: &str,
        severity: &str,
    ) -> BTreeMap<String, KiCadTranslatedConnection> {
        let pcb = parse(board).unwrap();
        let names = NetNames::of(&pcb);
        let pads = board_pads(&pcb, &names).unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..Default::default()
        };
        let result = LoweredBoard::new(pcb, &config, names, pads).unwrap();
        let drc = json!({
            "source": "test.kicad_pcb", "coordinate_units": "mm", "kicad_version": "10.0.8",
            "included_severities": ["warning", "error", "exclusion"],
            "unconnected_items": [{"severity": severity, "items": [
                {"description": "Pad 1 [broken] of U3", "uuid": "three", "pos": {"x": 4, "y": 15}},
                {"description": "Pad 2 [broken] of U4", "uuid": end_id, "pos": {"x": 14, "y": 15}}
            ]}],
        });
        let (opens, _) = native_pairs_by_net(Some(&drc), &result.names);
        diagnose(&result, &drc, &opens, &["broken".into()])
    }

    #[test]
    fn translated_match_requires_native_open_both_pads_and_connected_peer() {
        let matched = inspect(BOARD, "four", "error");
        let diagnosis = &matched["broken"];
        assert_eq!(diagnosis.routed_net, "closed");
        assert_eq!(diagnosis.translation_mm, [2.0, 10.0]);
        assert_eq!(diagnosis.open_endpoints[0].reference, "U3");
        assert_eq!(diagnosis.routed_endpoints[1].pad, "2");

        assert!(inspect(BOARD, "two", "error").is_empty());
        assert!(inspect(BOARD, "four", "exclusion").is_empty());
        let stub = BOARD.replace("(end 12 5) (width", "(end 8 5) (width");
        assert!(inspect(&stub, "four", "error").is_empty());
    }

    #[test]
    fn translated_match_rejects_geometry_changes_on_otherwise_identical_footprints() {
        let moved = BOARD.replace("(at 12 5) (property", "(at 12.1 5) (property");
        assert!(inspect(&moved, "four", "error").is_empty());
        let reshaped = BOARD.replace(
            "(size 0.8 1) (layers \"F.Cu\") (net \"closed\")",
            "(size 0.9 1) (layers \"F.Cu\") (net \"closed\")",
        );
        assert!(inspect(&reshaped, "four", "error").is_empty());
    }

    #[test]
    fn translated_match_omits_ambiguous_routed_matches() {
        let repeats = format!(
            r#"{}
            (footprint "Chip" (layer "F.Cu") (at 3 10) (property "Reference" "U5")
              (pad "1" smd rect (at 0 0) (size 0.8 1) (layers "F.Cu") (net "other") (uuid "five")))
            (footprint "Flash" (layer "F.Cu") (at 13 10) (property "Reference" "U6")
              (pad "2" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "other") (uuid "six")))
            (segment (start 3 10) (end 13 10) (width 0.2) (layer "F.Cu") (net "other")))"#,
            BOARD.strip_suffix(')').unwrap()
        );
        assert!(inspect(&repeats, "four", "error").is_empty());
    }
}
