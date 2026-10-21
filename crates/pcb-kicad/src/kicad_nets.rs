// Copyright (C) 2026 Toit contributors.

//! Net names on a board: as pads and copper spell them, as connection ids
//! normalise them, and as the writer spells them back.

use super::*;

pub(crate) fn collect_named_nets(root: &Expr, named_nets: &mut BTreeSet<String>) {
    if root.head() == Some("net")
        && let Some(name) = net_name_from_form(root)
        && !name.is_empty()
        && name.parse::<u32>().is_err()
    {
        named_nets.insert(normalize_net(name).to_string());
    }
    for child in root.children() {
        collect_named_nets(child, named_nets);
    }
}

/// Connection IDs omit one hierarchical slash, but native net names do not.
/// Pads identify the exact spelling to use for emitted/retained copper. Reject
/// aliases with two distinct pad nets rather than silently shorting them.
pub(crate) fn unambiguous_pad_net_names(pcb: &Expr) -> Result<BTreeMap<String, Expr>, String> {
    note_net_names(pcb);
    let mut names: BTreeMap<String, Expr> = BTreeMap::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|n| n.head() == Some("footprint"))
    {
        for pad in footprint
            .children()
            .iter()
            .filter(|n| n.head() == Some("pad"))
        {
            let Some(name) = node_net(pad).filter(|name| !name.is_empty()) else {
                continue;
            };
            let connection = normalize_net(name).to_string();
            let atom = pad.child("net").unwrap().children().last().unwrap().clone();
            if let Some(previous) = names.get(&connection) {
                if previous.atom() != Some(name) {
                    return Err(format!(
                        "ambiguous native net identity {connection:?}: pads name both {:?} and {name:?}",
                        previous.atom().unwrap()
                    ));
                }
            } else {
                names.insert(connection, atom);
            }
        }
    }
    Ok(names)
}

pub(crate) fn canonicalize_copper_net_names(pcb: &mut Expr, names: &BTreeMap<String, Expr>) {
    let Expr::List(items) = pcb else {
        return;
    };
    for item in items {
        if !matches!(item.head(), Some("segment" | "arc" | "via" | "zone")) {
            continue;
        }
        // The writer names a connection "/X"; the pads' map is keyed by the
        // normalised name, which keeps the slash when a bare "X" exists too
        // (`note_net_names`): then "/GND" must still find "GND", or KiCad
        // makes a net "/GND" of our copper (Hub on x153: 636 items, 36
        // clearance errors between "/GND" and "GND" vias).
        let Some(name) = node_net(item)
            .and_then(|name| {
                names
                    .get(normalize_net(name))
                    .or_else(|| name.strip_prefix('/').and_then(|bare| names.get(bare)))
                    .or_else(|| names.get(name))
            })
            .cloned()
        else {
            continue;
        };
        let Expr::List(parts) = item else {
            continue;
        };
        for part in parts {
            if let Some(head @ ("net" | "net_name")) = part.head() {
                // Preserve the source atom, including escaping, rather than
                // quoting its already escaped contents a second time.
                *part = Expr::List(vec![Expr::Atom(head.into()), name.clone()]);
            }
        }
    }
}

thread_local! {
    /// The board's net names without a leading slash (its global labels
    /// and power nets), noted by `note_net_names`: a connection id drops
    /// one hierarchical slash, which must not fold `/VBUS` (a root-sheet
    /// label) into `VBUS` when the board has both (a business-card panel
    /// had both, and lowering rejected it as ambiguous).
    static BARE_NETS: std::cell::RefCell<std::collections::HashSet<String>> = Default::default();
}

/// Records `pcb`'s bare net names for `normalize_net` (on this thread,
/// for the lowering that follows).
pub(crate) fn note_net_names(pcb: &Expr) {
    // The net table; a KiCad 10 board has none (its items name their
    // nets), so the pads' names too: gametank's expansion-port board has
    // pads on "/A15" and on "A15", one net read before.
    let table = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("net"))
        .filter_map(|item| item.children().get(2).and_then(Expr::atom));
    let pads = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .flat_map(|footprint| footprint.children())
        .filter(|item| item.head() == Some("pad"))
        .filter_map(node_net);
    let bare: std::collections::HashSet<String> = table
        .chain(pads)
        .filter(|name| !name.starts_with('/') && !name.is_empty())
        .map(str::to_string)
        .collect();
    BARE_NETS.with(|names| *names.borrow_mut() = bare);
}

/// The net name copper of `connection` is written with: the connection
/// with the hierarchical slash it dropped, or as it is when the board has
/// a bare net of that name (`/A15` then is another net, its connection
/// `/A15`, written `//A15`); `canonicalize_copper_net_names` then takes
/// the pads' own spelling.
pub(crate) fn written_net_name(connection: &str) -> String {
    if BARE_NETS.with(|names| names.borrow().contains(connection)) {
        connection.to_string()
    } else {
        format!("/{connection}")
    }
}

pub(crate) fn normalize_net(net: &str) -> &str {
    match net.strip_prefix('/') {
        Some(bare) if !BARE_NETS.with(|names| names.borrow().contains(bare)) => bare,
        _ => net,
    }
}

pub(crate) fn node_net(node: &Expr) -> Option<&str> {
    net_name_from_form(node.child("net")?)
}

pub(crate) fn net_expr(name: &str) -> Expr {
    Expr::List(vec![
        Expr::Atom("net".into()),
        Expr::Atom(format!("\"{name}\"")),
    ])
}

/// A footprint's reference: KiCad 8 and later write it as a property,
/// KiCad 6 and 7 as `(fp_text reference "R1" ...)`.
pub(crate) fn footprint_reference(node: &Expr) -> Option<String> {
    node.children().iter().find_map(|item| {
        let kind = item.children().get(1)?.atom()?;
        match item.head()? {
            "property" if kind == "Reference" => Some(item.children().get(2)?.atom()?.to_string()),
            "fp_text" if kind == "reference" => Some(item.children().get(2)?.atom()?.to_string()),
            _ => None,
        }
    })
}

/// A footprint's own identity: its `uuid` (KiCad 8 on) or `tstamp` (KiCad
/// 5 to 7), which stays with it when it moves and tells apart footprints
/// that share a reference (a panel's copies).
pub(crate) fn footprint_uuid(node: &Expr) -> Option<String> {
    node.children().iter().find_map(|item| match item.head()? {
        "uuid" | "tstamp" => Some(item.children().get(1)?.atom()?.to_string()),
        _ => None,
    })
}

pub(crate) fn net_name_from_form(node: &Expr) -> Option<&str> {
    if node.head() != Some("net") {
        return None;
    }
    let fields = node.children();
    match fields.len() {
        3.. => fields.get(2)?.atom(),
        2 => fields.get(1)?.atom(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copper_application_uses_exact_pad_net_spelling() {
        let mut pcb = parse(
            r#"(kicad_pcb
            (footprint "test"
              (pad "1" smd (net "GND"))
              (pad "2" smd (net "/DATA"))
              (pad "3" smd (net "N\"Q")))
            (segment (net "/GND"))
            (via (net "/GND"))
            (arc (net "/DATA"))
            (zone (net "/GND") (net_name "/GND"))
            (segment (net "/N\"Q"))
            (segment (net "ORPHAN")))"#,
        )
        .unwrap();
        let names = unambiguous_pad_net_names(&pcb).unwrap();
        canonicalize_copper_net_names(&mut pcb, &names);
        let copper = pcb
            .children()
            .iter()
            .filter(|n| matches!(n.head(), Some("segment" | "via" | "arc" | "zone")))
            .collect::<Vec<_>>();
        assert_eq!(
            copper
                .iter()
                .map(|n| node_net(n).unwrap())
                .collect::<Vec<_>>(),
            vec!["GND", "GND", "/DATA", "GND", r#"N\"Q"#, "ORPHAN"]
        );
        assert_eq!(
            copper[3].child("net_name").unwrap().children()[1].atom(),
            Some("GND")
        );
        // Raw escaped atoms must survive a complete text round trip.
        let reparsed = parse(&encode(&pcb)).unwrap();
        assert_eq!(encode(&reparsed), encode(&pcb));
    }

    #[test]
    fn colliding_normalized_pad_names_stay_two_nets_when_the_board_declares_both() {
        // A KiCad 10 board has no net table: its pads name both nets
        // (gametank's expansion-port board: "/A15" and "A15"), and they
        // stay two, through the copper written back.
        let mut pcb = parse(
            r#"(kicad_pcb (version 20260206) (footprint "test"
            (pad "1" smd (net "A15")) (pad "2" smd (net "/A15")) (pad "3" smd (net "/D0"))))"#,
        )
        .unwrap();
        let names = unambiguous_pad_net_names(&pcb).unwrap();
        assert_eq!(names.len(), 3);
        assert_eq!(normalize_net("/A15"), "/A15");
        assert_eq!(normalize_net("/D0"), "D0");
        let segment = |connection: &str| SupplementalSegment {
            connection: connection.into(),
            start: [0.0, 0.0],
            end: [1.0, 0.0],
            width: 0.2,
            layer: "F.Cu".into(),
        };
        if let Expr::List(items) = &mut pcb {
            for connection in ["A15", "/A15", "D0"] {
                items.push(supplemental_segment_expr(&segment(connection)));
            }
            items.push(supplemental_via_expr(&SupplementalVia {
                connection: "A15".into(),
                at: [0.0, 0.0],
                size: 0.6,
                drill: 0.3,
                layers: ["F.Cu".into(), "B.Cu".into()],
            }));
        }
        canonicalize_copper_net_names(&mut pcb, &names);
        let written: Vec<&str> = pcb
            .children()
            .iter()
            .filter(|item| matches!(item.head(), Some("segment" | "via")))
            .filter_map(node_net)
            .collect();
        assert_eq!(written, ["A15", "/A15", "/D0", "A15"]);
        // With both declared (a global and a root-sheet label), two nets.
        let pcb = parse(
            r#"(kicad_pcb (net 1 "SIGNAL") (net 2 "/SIGNAL") (footprint "test"
            (pad "1" smd (net "SIGNAL")) (pad "2" smd (net "/SIGNAL"))))"#,
        )
        .unwrap();
        let names = unambiguous_pad_net_names(&pcb).unwrap();
        assert_eq!(names.len(), 2);
        assert_eq!(normalize_net("/SIGNAL"), "/SIGNAL");
        assert_eq!(normalize_net("//SIGNAL"), "/SIGNAL");
        note_net_names(&parse("(kicad_pcb)").unwrap());
        assert_eq!(normalize_net("/SIGNAL"), "SIGNAL");
    }
}
