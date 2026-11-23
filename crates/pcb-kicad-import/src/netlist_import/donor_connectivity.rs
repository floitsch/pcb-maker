// Copyright (C) 2026 Florian Loitsch.

//! Informational comparison of the footprint donor and the schematic.

use super::*;
use crate::board_footprints::BoardFootprints;

type Terminal<'a> = (&'a str, &'a str);
type TerminalNets<'a> = BTreeMap<Terminal<'a>, Option<&'a str>>;
type NetGroups<'a> = BTreeMap<&'a str, Vec<Terminal<'a>>>;
const EXAMPLES: usize = 8;
const GROUP_SAMPLES: usize = 4;

#[derive(Clone, Debug, Serialize)]
pub struct DonorConnectivity {
    pub status: &'static str,
    pub compared_terminals: usize,
    pub donor_only_terminals: usize,
    pub schematic_only_terminals: usize,
    pub ambiguous_terminals_excluded: usize,
    pub repeated_references_excluded: usize,
    pub repeated_reference_examples: Vec<String>,
    pub donor_multi_pad_groups: usize,
    pub schematic_multi_pad_groups: usize,
    pub unchanged_groups: usize,
    pub split_donor_groups: usize,
    pub merged_schematic_groups: usize,
    pub split_examples: Vec<ChangedGroup>,
    pub merge_examples: Vec<ChangedGroup>,
    pub donor_connected_schematic_isolated: usize,
    pub isolated_examples: Vec<IsolatedTerminal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChangedGroup {
    net: String,
    terminals: usize,
    other_groups: usize,
    examples: Vec<GroupSample>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GroupSample {
    net: Option<String>,
    terminals: usize,
    first_terminal: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct IsolatedTerminal {
    reference: String,
    pad: String,
    donor_net: String,
    schematic_net: Option<String>,
}

fn electrical_net(name: &str) -> Option<&str> {
    (!name.is_empty() && !name.trim_start_matches('/').starts_with("unconnected-(")).then_some(name)
}

fn put<'a>(
    nets: &mut TerminalNets<'a>,
    ambiguous: &mut BTreeSet<Terminal<'a>>,
    terminal: Terminal<'a>,
    net: Option<&'a str>,
) {
    if let Some(previous) = nets.insert(terminal, net)
        && previous != net
    {
        ambiguous.insert(terminal);
    }
}

fn groups<'a>(nets: &TerminalNets<'a>) -> NetGroups<'a> {
    let mut groups = NetGroups::new();
    for (&terminal, net) in nets {
        if let Some(name) = net {
            groups.entry(name).or_default().push(terminal);
        }
    }
    groups.retain(|_, terminals| terminals.len() >= 2);
    groups
}

#[derive(Clone, Copy, Ord, PartialOrd, Eq, PartialEq)]
enum Group<'a> {
    Connected(&'a str),
    Isolated(Terminal<'a>),
}

fn opposing_groups<'a>(
    groups: &NetGroups<'a>,
    other_nets: &TerminalNets<'a>,
    other_groups: &NetGroups<'a>,
) -> (usize, usize, Vec<ChangedGroup>) {
    let mut unchanged = 0;
    let mut split = 0;
    let mut examples = Vec::new();
    for (&name, terminals) in groups {
        let mut fragments: BTreeMap<_, (usize, Terminal<'_>)> = BTreeMap::new();
        for &terminal in terminals {
            let opposite = other_nets[&terminal].filter(|name| other_groups.contains_key(name));
            let key = opposite.map_or(Group::Isolated(terminal), Group::Connected);
            fragments.entry(key).or_insert((0, terminal)).0 += 1;
        }
        if let Some((Group::Connected(other), _)) = fragments.first_key_value()
            && fragments.len() == 1
            && other_groups[other].len() == terminals.len()
        {
            unchanged += 1;
        }
        if fragments.len() < 2 {
            continue;
        }
        split += 1;
        if examples.len() < EXAMPLES {
            examples.push(ChangedGroup {
                net: name.to_owned(),
                terminals: terminals.len(),
                other_groups: fragments.len(),
                examples: fragments
                    .into_iter()
                    .take(GROUP_SAMPLES)
                    .map(|(group, (count, terminal))| {
                        let net = match group {
                            Group::Connected(net) => Some(net.to_owned()),
                            Group::Isolated(_) => None,
                        };
                        GroupSample {
                            net,
                            terminals: count,
                            first_terminal: format!("{}:{}", terminal.0, terminal.1),
                        }
                    })
                    .collect(),
            });
        }
    }
    (unchanged, split, examples)
}

pub(super) fn compare(
    designer: &BoardFootprints,
    netlist: &Expr,
    components: &[Expr],
) -> DonorConnectivity {
    let mut donor_refs = BTreeSet::new();
    let mut schematic_refs = BTreeSet::new();
    let mut repeated = BTreeSet::new();
    for part in &designer.footprints {
        if !part.reference.is_empty() && !donor_refs.insert(part.reference.as_str()) {
            repeated.insert(part.reference.as_str());
        }
    }
    for part in components.iter().filter(|part| part.head() == Some("comp")) {
        if part.children().iter().any(|item| {
            item.head() == Some("property")
                && form_atom(item, "name", 1) == Some("exclude_from_board")
                && form_atom(item, "value", 1).is_none()
        }) {
            continue;
        }
        if let Some(reference) = form_atom(part, "ref", 1).filter(|reference| !reference.is_empty())
            && !schematic_refs.insert(reference)
        {
            repeated.insert(reference);
        }
    }
    let mut ambiguous = BTreeSet::new();
    let mut donor = TerminalNets::new();
    for part in &designer.footprints {
        let reference = part.reference.as_str();
        if reference.is_empty() || repeated.contains(reference) {
            continue;
        }
        for pad in part
            .expr
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
        {
            if let Some(number) = pad
                .children()
                .get(1)
                .and_then(Expr::atom)
                .filter(|n| !n.is_empty())
            {
                put(
                    &mut donor,
                    &mut ambiguous,
                    (reference, number),
                    item_net(pad, &designer.net_codes).and_then(electrical_net),
                );
            }
        }
    }
    let mut schematic = TerminalNets::new();
    for net in netlist
        .child("nets")
        .into_iter()
        .flat_map(Expr::children)
        .filter(|net| net.head() == Some("net"))
    {
        let Some(name) = form_atom(net, "name", 1) else {
            continue;
        };
        for pin in net
            .children()
            .iter()
            .filter(|item| item.head() == Some("node"))
        {
            if let (Some(reference), Some(number)) =
                (form_atom(pin, "ref", 1), form_atom(pin, "pin", 1))
                && schematic_refs.contains(reference)
                && !repeated.contains(reference)
                && !number.is_empty()
            {
                put(
                    &mut schematic,
                    &mut ambiguous,
                    (reference, number),
                    electrical_net(name),
                );
            }
        }
    }
    donor.retain(|terminal, _| !ambiguous.contains(terminal));
    schematic.retain(|terminal, _| !ambiguous.contains(terminal));
    let original_donor = donor.len();
    let original_schematic = schematic.len();
    donor.retain(|terminal, _| schematic.contains_key(terminal));
    schematic.retain(|terminal, _| donor.contains_key(terminal));
    let compared_terminals = donor.len();
    let donor_groups = groups(&donor);
    let schematic_groups = groups(&schematic);
    let (unchanged_groups, split_donor_groups, split_examples) =
        opposing_groups(&donor_groups, &schematic, &schematic_groups);
    let (_, merged_schematic_groups, merge_examples) =
        opposing_groups(&schematic_groups, &donor, &donor_groups);
    let mut donor_connected_schematic_isolated = 0;
    let mut isolated_examples = Vec::new();
    for (&name, terminals) in &donor_groups {
        for &(reference, pad) in terminals {
            let schematic_net = schematic[&(reference, pad)];
            if schematic_net.is_some_and(|net| schematic_groups.contains_key(net)) {
                continue;
            }
            donor_connected_schematic_isolated += 1;
            if isolated_examples.len() < EXAMPLES {
                isolated_examples.push(IsolatedTerminal {
                    reference: reference.into(),
                    pad: pad.into(),
                    donor_net: name.into(),
                    schematic_net: schematic_net.map(str::to_owned),
                });
            }
        }
    }
    let changed = split_donor_groups > 0 || merged_schematic_groups > 0;
    DonorConnectivity {
        status: if compared_terminals < 2 { "insufficient_shared_terminals" } else if changed { "different" } else { "matches" },
        compared_terminals,
        donor_only_terminals: original_donor - compared_terminals,
        schematic_only_terminals: original_schematic - compared_terminals,
        ambiguous_terminals_excluded: ambiguous.len(),
        repeated_references_excluded: repeated.len(),
        repeated_reference_examples: repeated.into_iter().take(EXAMPLES).map(str::to_owned).collect(),
        donor_multi_pad_groups: donor_groups.len(),
        schematic_multi_pad_groups: schematic_groups.len(),
        unchanged_groups,
        split_donor_groups,
        merged_schematic_groups,
        split_examples,
        merge_examples,
        donor_connected_schematic_isolated,
        isolated_examples,
        next_action: changed.then_some("review the named terminals and confirm that the schematic or netlist is the intended electrical revision before layout; the imported board uses that input's nets"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board_footprints::BoardFootprint;

    #[test]
    fn compares_pin_groups_and_omits_ambiguous_identities() {
        let footprint = |reference: &str, pads: &str| BoardFootprint {
            reference: reference.into(),
            path: None,
            id: "model".into(),
            uuid: None,
            expr: parse(&format!("(footprint model {pads})")).unwrap(),
        };
        let donor = BoardFootprints {
            file: PathBuf::new(),
            sha256: String::new(),
            footprints: vec![
                footprint(
                    "J1",
                    r#"(pad "1" (net 1)) (pad "2" (net 1)) (pad "3" (net 2)) (pad "4" (net 2)) (pad "5") (pad "6" (net 3)) (pad "6" (net 4))"#,
                ),
                footprint("J2", r#"(pad "1" (net 5))"#),
                footprint("J3", r#"(pad "1" (net 5))"#),
                footprint("J3", r#"(pad "1" (net 5))"#),
                footprint("J4", r#"(pad "1" (net 5)) (pad "1" (net 5))"#),
            ],
            net_codes: BTreeMap::from([
                ("1".into(), "POWER".into()),
                ("2".into(), "PAIR".into()),
                ("3".into(), "CONFLICTING".into()),
                ("4".into(), "AMBIGUOUS".into()),
                ("5".into(), "SAME".into()),
            ]),
            copper_layers: 2,
        };
        let netlist = parse(r#"(export (components (comp (ref "J1")) (comp (ref "J2")) (comp (ref "J3")) (comp (ref "J4")))
            (nets
              (net (name "BUS") (node (ref "J1") (pin "1")) (node (ref "J1") (pin "3")) (node (ref "J1") (pin "4")))
              (net (name "unconnected-(J1-2)") (node (ref "J1") (pin "2")))
              (net (name "unconnected-(J1-5)") (node (ref "J1") (pin "5")))
              (net (name "CONFLICTING") (node (ref "J1") (pin "6")))
              (net (name "renamed") (node (ref "J2") (pin "1")) (node (ref "J3") (pin "1")) (node (ref "J4") (pin "1")))))"#).unwrap();
        let report = compare(
            &donor,
            &netlist,
            netlist.child("components").unwrap().children(),
        );
        assert_eq!(report.status, "different");
        assert_eq!(report.compared_terminals, 7);
        assert_eq!(report.ambiguous_terminals_excluded, 1);
        assert_eq!(report.repeated_reference_examples, ["J3"]);
        assert_eq!(report.unchanged_groups, 1);
        assert_eq!(report.split_donor_groups, 1);
        assert_eq!(report.merged_schematic_groups, 1);
        assert_eq!(report.isolated_examples.len(), 1);
        assert_eq!(report.isolated_examples[0].pad, "2");
        assert_eq!(report.isolated_examples[0].donor_net, "POWER");
    }
}
