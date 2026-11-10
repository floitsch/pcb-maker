// Copyright (C) 2026 Toit contributors.

//! Reading the netlist: the file, the libraries its project has, the pins'
//! nets and the symbols' paths.

use super::*;

/// Reads and parses the KiCad netlist at `netlist_path`; anything else is an
/// input error.
pub(super) fn read_netlist(netlist_path: &Path) -> Result<Expr, String> {
    let not_a_netlist = || {
        crate::verdict::input_error(format!(
            "{} is not a KiCad netlist: give the schematic, or export one with kicad-cli sch export netlist --format kicadsexpr",
            netlist_path.display()
        ))
    };
    let netlist = fs::read_to_string(netlist_path)
        .map_err(|error| {
            crate::verdict::input_error(format!(
                "failed to read {}: {error}",
                netlist_path.display()
            ))
        })
        .and_then(|text| parse(&text).map_err(|_| not_a_netlist()))?;
    if netlist.head() != Some("export") || netlist.child("components").is_none() {
        return Err(not_a_netlist());
    }
    Ok(netlist)
}

/// The footprint libraries the import searches: the project's and KiCad's,
/// and for a netlist the libraries of the project it names as its source.
pub(super) fn find_libraries(
    project: &Path,
    output: &Path,
    netlist: &Expr,
    is_schematic: bool,
) -> (BTreeMap<String, PathBuf>, LibraryNotes) {
    let (mut libraries, library_notes) = footprint_library_search(project, Some(output));
    // A netlist exported elsewhere names its schematic: that project's
    // libraries (its fp-lib-table, its .pretty folders) come next.
    if !is_schematic
        && let Some(source) = netlist
            .child("design")
            .and_then(|design| text_of(design, "source"))
        && let Some(folder) = Path::new(&source).parent().filter(|folder| folder.is_dir())
    {
        for (name, path) in footprint_libraries(folder) {
            libraries.entry(name).or_insert(path);
        }
    }
    (libraries, library_notes)
}

/// Nets by (reference, pin): name, pin function and type, and the number of
/// named nets.
pub(super) fn pin_nets(netlist: &Expr) -> (BTreeMap<(String, String), PinNet>, usize) {
    let mut pins: BTreeMap<(String, String), PinNet> = BTreeMap::new();
    let mut nets = 0;
    if let Some(section) = netlist.child("nets") {
        for net in section
            .children()
            .iter()
            .filter(|item| item.head() == Some("net"))
        {
            let Some(name) = text_of(net, "name") else {
                continue;
            };
            nets += 1;
            for node in net
                .children()
                .iter()
                .filter(|item| item.head() == Some("node"))
            {
                let (Some(reference), Some(pin)) = (text_of(node, "ref"), text_of(node, "pin"))
                else {
                    continue;
                };
                pins.insert(
                    (reference, pin),
                    (
                        name.clone(),
                        text_of(node, "pinfunction"),
                        text_of(node, "pintype"),
                    ),
                );
            }
        }
    }
    (pins, nets)
}

/// The symbol a footprint is linked to: `/<sheet uuids>/<symbol uuid>`.
pub(super) fn symbol_path_of(component: &Expr) -> Option<String> {
    let sheet = component
        .child("sheetpath")
        .and_then(|sheet| text_of(sheet, "tstamps"))
        .unwrap_or_else(|| "/".into());
    let stamp = text_of(component, "tstamps").or_else(|| text_of(component, "tstamp"))?;
    Some(format!(
        "{}{stamp}",
        if sheet.ends_with('/') {
            sheet
        } else {
            format!("{sheet}/")
        }
    ))
}

/// Every symbol's path, so that a board footprint linked to one symbol is
/// not taken for another of its reference.
pub(super) fn symbol_paths(components: &[Expr]) -> BTreeSet<String> {
    components
        .iter()
        .filter(|item| item.head() == Some("comp"))
        .filter_map(symbol_path_of)
        .collect()
}
