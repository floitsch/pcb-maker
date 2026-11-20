// Copyright (C) 2026 Florian Loitsch.

//! Read-only numbering evidence from a symbol's own board footprint.

use super::*;
use crate::board_footprints::{BoardFootprint, BoardFootprints};

#[derive(Clone, Debug, Serialize)]
pub struct KiCadDonorPinNumbering {
    pub status: &'static str,
    pub reason: &'static str,
    pub board: PathBuf,
    pub board_sha256: String,
    pub footprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footprint_uuid: Option<String>,
    pub candidates: Vec<KiCadDonorPadNumber>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadDonorPadNumber {
    pub number: String,
    pub instances: Vec<KiCadDonorPadInstance>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadDonorPadInstance {
    /// The id visible in the source file, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    /// The pad's position in its original footprint's frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_at_mm: Option<[f64; 2]>,
    pub pad_type: String,
    pub net: Option<String>,
}

pub(super) const REVIEW_MAPPING: &str = "review whether the donor pad number is the physical terminal for this schematic pin. Correct the source footprint or symbol mapping if confirmed, then re-import and check native schematic parity";

#[derive(Clone, Copy)]
pub(super) struct Donor<'a> {
    pub board: &'a BoardFootprints,
    pub part: &'a BoardFootprint,
    pub component: &'a Expr,
    pub library_parts: &'a [Expr],
}

fn pin_numbers(pins: &Expr) -> impl Iterator<Item = &str> {
    pins.children()
        .iter()
        .filter(|pin| pin.head() == Some("pin"))
        .filter_map(|pin| form_atom(pin, "num", 1))
}

fn symbol_numbers<'a>(donor: Donor<'a>, wanted: &str) -> Option<BTreeSet<&'a str>> {
    let units = donor
        .component
        .child("units")
        .into_iter()
        .flat_map(Expr::children)
        .filter(|unit| unit.head() == Some("unit"))
        .filter_map(|unit| unit.child("pins"))
        .flat_map(pin_numbers)
        .collect::<BTreeSet<_>>();
    if units.contains(wanted) {
        return Some(units);
    }
    let source = donor.component.child("libsource")?;
    let library = form_atom(source, "lib", 1)?;
    let part = form_atom(source, "part", 1)?;
    let mut matches = donor.library_parts.iter().filter(|model| {
        model.head() == Some("libpart")
            && form_atom(model, "lib", 1) == Some(library)
            && form_atom(model, "part", 1) == Some(part)
    });
    let model = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let numbers = pin_numbers(model.child("pins")?).collect::<BTreeSet<_>>();
    numbers.contains(wanted).then_some(numbers)
}

impl Donor<'_> {
    pub(super) fn inspect(
        self,
        reference: &str,
        pin: &str,
        net: &str,
        selected: &BTreeSet<&str>,
        pins: &PinNets,
    ) -> Option<KiCadDonorPinNumbering> {
        if self.part.reference != reference || net.is_empty() {
            return None;
        }
        let result = |status, reason, candidates| {
            Some(KiCadDonorPinNumbering {
                status,
                reason,
                board: self.board.file.clone(),
                board_sha256: self.board.sha256.clone(),
                footprint: self.part.id.clone(),
                footprint_uuid: self.part.uuid.clone(),
                candidates,
            })
        };
        let mut donor_pads = BTreeMap::<_, Vec<_>>::new();
        for item in self.part.expr.children() {
            if item.head() == Some("pad")
                && let Some(number) = item.children().get(1).and_then(Expr::atom)
                && !number.is_empty()
            {
                donor_pads.entry(number).or_default().push(item);
            }
        }
        if donor_pads.contains_key(pin) {
            return result("no_candidate", "pin_number_exists_on_donor", Vec::new());
        }
        let Some(symbol_numbers) = symbol_numbers(self, pin) else {
            return result("no_candidate", "symbol_pin_numbers_unavailable", Vec::new());
        };
        let bindings: BTreeMap<_, _> = pins
            .range((reference.to_owned(), String::new())..)
            .take_while(|((owner, _), _)| owner == reference)
            .map(|((_, number), bindings)| (number.as_str(), bindings))
            .collect();
        let candidates: Vec<_> = donor_pads
            .into_iter()
            .filter(|(number, pads)| {
                selected.contains(number)
                    && !symbol_numbers.contains(number)
                    && !bindings.contains_key(number)
                    && pads
                        .iter()
                        .any(|pad| item_net(pad, &self.board.net_codes) == Some(net))
            })
            .map(|(number, pads)| KiCadDonorPadNumber {
                number: number.into(),
                instances: pads
                    .into_iter()
                    .map(|pad| KiCadDonorPadInstance {
                        uuid: form_atom(pad, "uuid", 1)
                            .or_else(|| form_atom(pad, "tstamp", 1))
                            .map(str::to_owned),
                        local_at_mm: form_at(pad).ok().map(|[x, y, _]| [x, y]),
                        pad_type: pad
                            .children()
                            .get(2)
                            .and_then(Expr::atom)
                            .unwrap_or("")
                            .into(),
                        net: item_net(pad, &self.board.net_codes).map(str::to_owned),
                    })
                    .collect(),
            })
            .collect();
        match candidates.as_slice() {
            [] => result("no_candidate", "no_unused_donor_number_on_net", candidates),
            [single] => {
                let ambiguous =
                    if bindings
                        .get(pin)
                        .is_none_or(|bindings| bindings.iter().any(|binding| binding.0 != net))
                    {
                        Some("conflicting_schematic_pin_bindings")
                    } else if bindings.iter().any(|(&number, values)| {
                        number != pin
                            && !selected.contains(number)
                            && values.iter().any(|binding| binding.0 == net)
                    }) {
                        Some("multiple_missing_pins_on_net")
                    } else if single
                        .instances
                        .iter()
                        .any(|pad| pad.net.as_deref() != Some(net))
                    {
                        Some("conflicting_donor_pad_bindings")
                    } else if single.instances.iter().any(|pad| {
                        !matches!(pad.pad_type.as_str(), "smd" | "thru_hole" | "connect")
                    }) {
                        Some("non_electrical_donor_pad_instances")
                    } else {
                        None
                    };
                if let Some(reason) = ambiguous {
                    result("ambiguous", reason, candidates)
                } else {
                    result(
                        "unique_candidate",
                        "same_net_on_unused_donor_number",
                        candidates,
                    )
                }
            }
            _ => result(
                "ambiguous",
                "multiple_unused_donor_numbers_on_net",
                candidates,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(
        pads: &str,
        selected: &[&str],
        symbol_pins: &str,
        pin_nets: &[(&str, &str)],
    ) -> KiCadDonorPinNumbering {
        let board = BoardFootprints {
            file: "donor.kicad_pcb".into(),
            sha256: "fixture".into(),
            footprints: vec![BoardFootprint {
                reference: "J7".into(),
                path: None,
                id: "fixture:DSUB".into(),
                uuid: None,
                expr: parse(&format!("(footprint fixture {pads})")).unwrap(),
            }],
            net_codes: BTreeMap::from([("2".into(), "GNDD".into())]),
            copper_layers: 2,
        };
        let component = parse(&format!("(comp (units (unit (pins {symbol_pins}))))")).unwrap();
        let mut bindings = PinNets::new();
        for &(number, net) in pin_nets {
            bindings
                .entry(("J7".into(), number.into()))
                .or_default()
                .push((net.into(), None, None));
        }
        Donor {
            board: &board,
            part: &board.footprints[0],
            component: &component,
            library_parts: &[],
        }
        .inspect(
            "J7",
            "SH",
            "GNDD",
            &selected.iter().copied().collect(),
            &bindings,
        )
        .unwrap()
    }

    #[test]
    fn two_native_instances_with_one_number_are_one_candidate() {
        let report = case(
            r#"(pad "0" thru_hole circle (at 2 5) (net "GNDD") (uuid first))
               (pad "0" thru_hole circle (at 2 -5) (net 2 "GNDD") (tstamp second))"#,
            &["0"],
            "(pin (num SH))",
            &[("SH", "GNDD")],
        );
        assert_eq!(report.status, "unique_candidate");
        let [candidate] = report.candidates.as_slice() else {
            panic!("one donor pad number must be supported");
        };
        assert_eq!(candidate.number, "0");
        assert_eq!(candidate.instances.len(), 2);
        assert_eq!(candidate.instances[0].uuid.as_deref(), Some("first"));
        assert_eq!(candidate.instances[1].uuid.as_deref(), Some("second"));
        assert_eq!(candidate.instances[1].local_at_mm, Some([2.0, -5.0]));
    }

    #[test]
    fn competing_numbers_and_nonuniform_bindings_stay_ambiguous() {
        let one = r#"(pad "0" thru_hole circle (net "GNDD"))"#;
        let report = case(
            &format!(r#"{one} (pad "9" thru_hole circle (net "GNDD"))"#),
            &["0", "9"],
            "(pin (num SH))",
            &[("SH", "GNDD")],
        );
        assert_eq!(report.reason, "multiple_unused_donor_numbers_on_net");
        let report = case(
            &format!(r#"{one} (pad "0" thru_hole circle (net "GND"))"#),
            &["0"],
            "(pin (num SH))",
            &[("SH", "GNDD")],
        );
        assert_eq!(report.reason, "conflicting_donor_pad_bindings");
        let report = case(
            one,
            &["0"],
            "(pin (num SH)) (pin (num SH2))",
            &[("SH", "GNDD"), ("SH2", "GNDD")],
        );
        assert_eq!(report.reason, "multiple_missing_pins_on_net");
    }

    #[test]
    fn schematic_owned_off_net_and_missing_selected_numbers_are_excluded() {
        let pads = r#"(pad "0" thru_hole circle (net "GNDD"))
                      (pad "8" thru_hole circle (net "GND"))
                      (pad "9" thru_hole circle (net "GNDD"))"#;
        let report = case(
            pads,
            &["0", "8"],
            "(pin (num SH)) (pin (num 0))",
            &[("SH", "GNDD")],
        );
        assert_eq!(report.status, "no_candidate");
        assert!(report.candidates.is_empty());
    }
}
