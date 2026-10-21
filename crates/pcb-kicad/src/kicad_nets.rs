// Copyright (C) 2026 Toit contributors.

//! Net names on a board: as pads and copper spell them, as connection ids
//! normalise them, and as the writer spells them back.

use super::*;

/// The net names `root` and the forms in it name (with a stack, not
/// recursion: a tree built in code is not bound by the parser's nesting).
pub(crate) fn collect_named_nets(root: &Expr, named_nets: &mut BTreeSet<String>) {
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if node.head() == Some("net")
            && let Some(name) = net_name_from_form(node)
            && !name.is_empty()
        {
            named_nets.insert(normalize_net(name).to_string());
        }
        pending.extend(
            node.children()
                .iter()
                .filter(|item| matches!(item, Expr::List(_))),
        );
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
            // The field `node_net` read the name from (the third of
            // `(net 1 "GND")`, the second of `(net "GND")`), not the last:
            // `(net 1 "GND" x)` named its copper "x".
            let Some(atom) = pad.child("net").and_then(net_name_field).cloned() else {
                continue;
            };
            if let Some(previous) = names.get(&connection) {
                if previous.atom() != Some(name) {
                    return Err(format!(
                        "ambiguous native net identity {connection:?}: pads name both {:?} and {name:?}",
                        previous.atom().unwrap_or_default()
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
    net_name_field(node)?.atom()
}

/// The field of a `net` form that names the net (`net_name_from_form`).
pub(crate) fn net_name_field(node: &Expr) -> Option<&Expr> {
    if node.head() != Some("net") {
        return None;
    }
    let fields = node.children();
    match fields.len() {
        3.. => fields.get(2).filter(|field| field.atom().is_some()),
        // `(net 5)` is a net code (KiCad 9 and before: segments, vias,
        // zones), not a name; KiCad 10 quotes the name (`(net "5")`), and
        // a net may be named by a number (Soraxem's usb-dmx: nets "14"
        // and "0", whose pads KiCad wants connected like any other).
        2 => match &fields[1] {
            Expr::Atom(raw) if net_code(raw) => None,
            field => field.atom().map(|_| field),
        },
        _ => None,
    }
}

/// The board's net table, code to name (KiCad 9 and before; a KiCad 10
/// board has none).
pub(crate) fn net_codes(pcb: &Expr) -> BTreeMap<String, String> {
    pcb.children()
        .iter()
        .filter(|item| item.head() == Some("net") && item.children().len() >= 3)
        .filter_map(|item| {
            Some((
                item.children()[1].atom()?.to_string(),
                item.children()[2].atom()?.to_string(),
            ))
        })
        .collect()
}

/// The net an item names, by name or (KiCad 9 and before: segments, vias,
/// graphics, `(net 2)`) by its code in `codes` (`net_codes`).
pub(crate) fn item_net<'a>(node: &'a Expr, codes: &'a BTreeMap<String, String>) -> Option<&'a str> {
    node_net(node).or_else(|| {
        let fields = node.child("net")?.children();
        match fields {
            [_, Expr::Atom(raw)] if net_code(raw) => codes.get(raw.as_str()).map(String::as_str),
            _ => None,
        }
    })
}

/// Whether a net form's single field is a net code: an unquoted number.
fn net_code(raw: &str) -> bool {
    !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_net_form_of_four_fields_names_the_third() {
        // `(net 1 "GND" x)`: the copper was renamed "x", and a second pad
        // of the net panicked (`previous.atom().unwrap()` on a list).
        let pcb = parse(
            r#"(kicad_pcb
              (footprint "a" (pad "1" smd rect (net 1 "GND" x)) (pad "2" smd rect (net 1 "GND" (y))))
              (footprint "b" (pad "1" smd rect (net 1 "GND"))))"#,
        )
        .unwrap();
        let names = unambiguous_pad_net_names(&pcb).unwrap();
        assert_eq!(names["GND"].atom(), Some("GND"));
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn a_net_form_names_a_net_unless_it_is_a_code() {
        let name = |text: &str| net_name_from_form(&parse(text).unwrap()).map(str::to_string);
        assert_eq!(name(r#"(net 6 "14")"#).as_deref(), Some("14"));
        assert_eq!(name(r#"(net "14")"#).as_deref(), Some("14"));
        assert_eq!(name(r#"(net "GND")"#).as_deref(), Some("GND"));
        assert_eq!(name("(net 6)"), None);
        assert_eq!(name("(net 0)"), None);
    }

    #[test]
    fn an_item_names_its_net_by_code_in_an_older_board() {
        // justbuchanan's ledboard (KiCad 8): copper arcs of Net-(D1-K) as
        // `(gr_arc ... (net 2))`.
        let pcb = parse(
            r#"(kicad_pcb (net 0 "") (net 2 "Net-(D1-K)") (net 3 "5")
              (gr_arc (start 0 0) (mid 1 1) (end 2 0) (layer "F.Cu") (net 2))
              (segment (start 0 0) (end 1 0) (net "5"))
              (via (at 0 0) (net 3))
              (segment (start 0 0) (end 1 0) (net 7)))"#,
        )
        .unwrap();
        let codes = net_codes(&pcb);
        let nets: Vec<Option<&str>> = pcb
            .children()
            .iter()
            .filter(|item| item.head().is_some_and(|head| head != "net"))
            .map(|item| item_net(item, &codes))
            .collect();
        assert_eq!(nets, vec![Some("Net-(D1-K)"), Some("5"), Some("5"), None]);
    }

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
