// Copyright (C) 2026 Toit contributors.

//! The board's footprints, one per symbol: found in the libraries or the
//! `--footprints-from` board, dressed with the symbol's links, fields, flags
//! and pad nets.

use super::*;
use crate::board_footprints::{BoardFootprint, BoardFootprints, Found};

/// What the symbols' footprints are looked up in.
pub(super) struct Lookup<'a> {
    pub libraries: &'a BTreeMap<String, PathBuf>,
    pub notes: &'a LibraryNotes,
    pub designer: Option<&'a BoardFootprints>,
    pub pins: &'a BTreeMap<(String, String), PinNet>,
    pub symbol_paths: &'a BTreeSet<String>,
}

/// What the symbols' footprints leave for the report and the later stages.
#[derive(Default)]
pub(super) struct Findings {
    pub missing: Vec<String>,
    pub substituted: Vec<String>,
    pub add_on: Vec<String>,
    pub unmatched_pins: Vec<String>,
    /// Footprint ids the libraries know under another name, old to new.
    /// (Old, new): how many symbols; the copied schematic names a footprint
    /// id once for all its symbols, so the most common new name wins.
    pub renamed: BTreeMap<(String, String), usize>,
    /// With `--footprints-from`: footprints taken from the board for want of
    /// a library (to write into the project's libraries, by id), the board's
    /// versions used, the drifted ones (id: sizes, references).
    pub local_footprints: BTreeMap<String, Expr>,
    pub from_board: Vec<String>,
    pub board_versions: usize,
    pub drift: BTreeMap<(String, String), (String, Vec<String>)>,
    pub footprints: usize,
    /// The board footprints whose ids the import keeps (`kept_uuid`).
    pub kept_ids: BTreeSet<String>,
    /// The parts as the key matrix inference sees them.
    pub matrix_parts: Vec<crate::key_matrix::MatrixPart>,
}

/// A netlist component as the import reads it.
struct Symbol<'a> {
    component: &'a Expr,
    reference: String,
    value: String,
    /// Symbol properties without a value are flags.
    flags: BTreeSet<String>,
    sheet_path: Option<&'a Expr>,
    path: Option<String>,
}

/// A footprint form found for a symbol: the id it goes by on the board and
/// the board footprint's uuid where the import keeps it.
struct Resolved {
    footprint: Expr,
    id: String,
    kept: Option<String>,
}

fn read_symbol(component: &Expr) -> Symbol<'_> {
    Symbol {
        component,
        reference: text_of(component, "ref").unwrap_or_default(),
        value: text_of(component, "value").unwrap_or_default(),
        flags: component
            .children()
            .iter()
            .filter(|item| item.head() == Some("property") && form_atom(item, "value", 1).is_none())
            .filter_map(|item| text_of(item, "name"))
            .collect(),
        sheet_path: component.child("sheetpath"),
        path: symbol_path_of(component),
    }
}

/// The symbol as the key matrix inference sees it, with its pins.
fn matrix_part(
    symbol: &Symbol,
    pins: &BTreeMap<(String, String), PinNet>,
) -> crate::key_matrix::MatrixPart {
    let component = symbol.component;
    let reference = &symbol.reference;
    crate::key_matrix::MatrixPart {
        reference: reference.clone(),
        footprint: text_of(component, "footprint").unwrap_or_default(),
        symbol: component
            .child("libsource")
            .and_then(|source| text_of(source, "part"))
            .unwrap_or_default(),
        value: symbol.value.clone(),
        pins: pins
            .range((reference.clone(), String::new())..)
            .take_while(|((owner, _), _)| owner == reference)
            .map(|((_, pin), (net, _, kind))| (pin.clone(), net.clone(), kind.clone()))
            .collect(),
    }
}

/// Puts a footprint on `board` for each symbol of `components` that has
/// one; the ones without are in `findings.missing`.
pub(super) fn place_symbols(
    board: &mut Expr,
    components: &[Expr],
    lookup: &Lookup,
    findings: &mut Findings,
) -> Result<(), String> {
    for component in components.iter().filter(|item| item.head() == Some("comp")) {
        let symbol = read_symbol(component);
        if symbol.flags.contains("exclude_from_board") {
            continue;
        }
        findings
            .matrix_parts
            .push(matrix_part(&symbol, lookup.pins));
        let Some(resolved) = resolve_footprint(&symbol, lookup, findings)? else {
            continue;
        };
        let Some(footprint) = dress_footprint(resolved, &symbol, lookup, findings)? else {
            continue;
        };
        if let Expr::List(board_items) = &mut *board {
            board_items.push(footprint);
        }
        findings.footprints += 1;
    }
    Ok(())
}

/// The footprint of a symbol: the library's, the `--footprints-from`
/// board's, or none (then the symbol is in `findings.missing`).
fn resolve_footprint(
    symbol: &Symbol,
    lookup: &Lookup,
    findings: &mut Findings,
) -> Result<Option<Resolved>, String> {
    let reference = &symbol.reference;
    // A symbol without a footprint takes the board's footprint of it,
    // where a board is given and has it.
    let named = text_of(symbol.component, "footprint").filter(|id| id.contains(':'));
    let unnamed = named.is_none();
    let Some(footprint_id) = named.or_else(|| {
        let own = lookup
            .designer?
            .own(symbol.path.as_deref(), reference, lookup.symbol_paths)?;
        Some(if own.id.contains(':') {
            own.id.clone()
        } else {
            format!("{FROM_BOARD_LIBRARY}:{}", own.id)
        })
    }) else {
        findings.missing.push(format!("{reference} (no footprint)"));
        return Ok(None);
    };
    let library = find_footprint(lookup.libraries, &footprint_id);
    let theirs = lookup.designer.and_then(|designer| {
        designer.for_symbol(
            symbol.path.as_deref(),
            reference,
            &footprint_id,
            library.is_none(),
            lookup.symbol_paths,
        )
    });
    match (library, theirs) {
        (library, Some((theirs, how))) => footprint_from_board(
            symbol,
            &footprint_id,
            unnamed,
            library,
            (theirs, how),
            lookup,
            findings,
        )
        .map(Some),
        (Some((file, found)), None) => {
            footprint_from_library(symbol, &footprint_id, &file, found, findings)
        }
        (None, None) => {
            findings
                .missing
                .push(not_found_line(reference, &footprint_id, lookup));
            Ok(None)
        }
    }
}

/// The footprint the `--footprints-from` board has for the symbol, under
/// the board's name where it differs; notes what it was taken for and how
/// far the library's version is from it.
fn footprint_from_board(
    symbol: &Symbol,
    footprint_id: &str,
    unnamed: bool,
    library: Option<(PathBuf, String)>,
    (theirs, how): (&BoardFootprint, Found),
    lookup: &Lookup,
    findings: &mut Findings,
) -> Result<Resolved, String> {
    let reference = &symbol.reference;
    let libraries = lookup.libraries;
    // A library footprint under another id (KiCad 10's name, an
    // add-on's nickname) is no library version of this one: the
    // board's goes into the project's libraries.
    let library = library.filter(|(_, found)| found == footprint_id);
    // The board's footprint, under the board's name where it
    // differs from the schematic's (the board was changed
    // after the schematic, or the other way round).
    let same_name = theirs.name() == crate::board_footprints::name_of(footprint_id);
    let wanted = if same_name {
        footprint_id.to_string()
    } else {
        theirs.id.clone()
    };
    let (wanted_library, wanted_name) = wanted.split_once(':').unwrap_or(("", &wanted));
    let form = crate::board_footprints::library_form(&theirs.expr, wanted_name)?;
    if let Some((file, _)) = &library {
        // How far the library's version, which the import would
        // have used, is from the board's.
        findings.board_versions += 1;
        let ours = fs::read_to_string(file)
            .ok()
            .and_then(|text| parse(&text).ok());
        if let (Some(a), Some(b)) = (
            ours.as_ref().and_then(crate::board_footprints::body_size),
            crate::board_footprints::body_size(&form),
        ) && crate::board_footprints::drifted(a, b)
        {
            let entry = findings
                .drift
                .entry((footprint_id.to_string(), theirs.id.clone()))
                .or_insert_with(|| {
                    let mut line = format!(
                        "library {:.2} x {:.2} mm, {} pads; board {:.2} x {:.2} mm, {} pads",
                        a.0[0], a.0[1], a.1, b.0[0], b.0[1], b.1
                    );
                    if !same_name {
                        line.push_str(&format!("; the board's footprint is {}", theirs.id));
                    }
                    (line, Vec::new())
                });
            entry.1.push(reference.clone());
        }
    }
    let resolves = find_footprint(libraries, &wanted).is_some_and(|(_, found)| found == wanted);
    let id = if (library.is_some() && same_name) || (!same_name && resolves) {
        wanted.clone()
    } else {
        // Into a library of the project: under the footprint's
        // library name where no library of that name exists
        // here, else `from_board`.
        let id = if libraries.contains_key(wanted_library) || wanted_library.is_empty() {
            format!("{FROM_BOARD_LIBRARY}:{wanted_name}")
        } else {
            wanted.clone()
        };
        findings
            .local_footprints
            .entry(id.clone())
            .or_insert_with(|| form.clone());
        id
    };
    if id != footprint_id {
        *findings
            .renamed
            .entry((footprint_id.to_string(), id.clone()))
            .or_default() += 1;
    }
    let whose = match how {
        Found::Own if same_name => format!("the board's {}", theirs.reference),
        Found::Own => format!(
            "the board's {}, which has another footprint than the schematic's",
            theirs.reference
        ),
        Found::ByName => format!("the board's {}, of that footprint name", theirs.reference),
    };
    if unnamed {
        findings.from_board.push(format!(
            "{reference}: {id} ({whose}; the schematic names no footprint)"
        ));
    } else if library.is_none() {
        findings
            .from_board
            .push(format!("{reference}: {id} ({whose})"));
    } else if id != footprint_id {
        findings
            .substituted
            .push(format!("{reference}: {footprint_id} -> {id} ({whose})"));
    }
    let kept = if how == Found::Own {
        kept_uuid(theirs, &mut findings.kept_ids)
    } else {
        None
    };
    Ok(Resolved {
        footprint: form,
        id,
        kept,
    })
}

/// The library's footprint file, under the id it was found by (`found`);
/// an unreadable one puts the symbol in `findings.missing`.
fn footprint_from_library(
    symbol: &Symbol,
    footprint_id: &str,
    file: &Path,
    found: String,
    findings: &mut Findings,
) -> Result<Option<Resolved>, String> {
    let reference = &symbol.reference;
    if file.parent().is_some_and(add_on_folder) {
        findings.add_on.push(format!(
            "{reference}: {found} ({})",
            file.parent()
                .map_or(String::new(), |folder| folder.display().to_string())
        ));
    }
    if found != footprint_id {
        findings.substituted.push(format!(
            "{reference}: {footprint_id} -> {found}{}",
            substitution_reason(footprint_id, &found)
        ));
        *findings
            .renamed
            .entry((footprint_id.to_string(), found.clone()))
            .or_default() += 1;
    }
    match fs::read_to_string(file)
        .map_err(|error| error.to_string())
        .and_then(|text| parse(&text))
        .and_then(|footprint| match footprint_form_error(&footprint) {
            Some(why) => Err(format!("the file holds {why}")),
            None => Ok(footprint),
        }) {
        Ok(footprint) => Ok(Some(Resolved {
            footprint,
            id: found,
            kept: None,
        })),
        Err(error) => {
            findings
                .missing
                .push(format!("{reference} ({found} unreadable: {error})"));
            Ok(None)
        }
    }
}

/// The `missing` line of a footprint no library has, saying where it was
/// looked for, so that the fix is plain.
fn not_found_line(reference: &str, footprint_id: &str, lookup: &Lookup) -> String {
    let (library, name) = footprint_id.split_once(':').unwrap_or(("", footprint_id));
    let mut line = match lookup.libraries.get(library) {
        Some(folder) => format!(
            "{reference} ({footprint_id} not found: {} has no {name}.kicad_mod",
            folder.display()
        ),
        None if lookup.notes.absent.contains_key(library) => format!(
            "{reference} ({footprint_id} not found: the fp-lib-table names {library:?} at {}, which is not there, nor are its last folders under the schematic's folder or its three parents: put the library there or in the output directory as {library}.pretty",
            lookup.notes.absent[library]
        ),
        None => format!(
            "{reference} ({footprint_id} not found: no library {library:?} in the fp-lib-table files, the schematic folder's or the output directory's .pretty folders, KiCad's own or its add-on libraries{}",
            ADD_ON_LIBRARIES
                .iter()
                .find(|(name, _)| *name == library.strip_prefix("PCM_").unwrap_or(library))
                .map_or(String::new(), |(_, source)| format!("; it is {source}"))
        ),
    };
    if let Some(designer) = lookup.designer {
        line.push_str(&format!(
            "; nor has {} this part or a footprint of that name",
            designer.file.display()
        ));
    }
    line.push(')');
    line
}

/// Makes the found footprint the board's footprint of the symbol: ids, the
/// placement fields, the links to the symbol, its fields and flags, the pads'
/// nets. `None` where the form is no list.
fn dress_footprint(
    resolved: Resolved,
    symbol: &Symbol,
    lookup: &Lookup,
    findings: &mut Findings,
) -> Result<Option<Expr>, String> {
    let Resolved {
        mut footprint,
        id,
        kept,
    } = resolved;
    let reference = &symbol.reference;
    let Expr::List(items) = &mut footprint else {
        return Ok(None);
    };
    items.retain(|item| {
        !matches!(
            item.head(),
            Some("version" | "generator" | "generator_version")
        )
    });
    if kept.is_none() {
        for item in items.iter_mut() {
            renew_uuids(item, reference);
        }
    }
    // KiCad 5 library files say `module`; KiCad renames it on loading,
    // pcb-maker reads only `footprint` (coco2's 16 footprints from its
    // KiCad 5 library were not placed or routed).
    items[0] = Expr::Atom("footprint".into());
    items[1] = Expr::Atom(requote(&id));
    // Placement fields after the layer.
    let layer_index = items
        .iter()
        .position(|item| item.head() == Some("layer"))
        .map_or(2, |index| index + 1);
    let uuid = kept.unwrap_or_else(|| uuid_of(&format!("footprint {reference}")));
    items.insert(layer_index, parse(&format!("(uuid \"{uuid}\")"))?);
    items.insert(layer_index + 1, parse("(at 0 0)")?);
    add_symbol_links(items, symbol)?;
    // The symbol's fields (datasheet, description, LCSC numbers, ...).
    legacy_texts_to_properties(items, reference)?;
    legacy_arcs_to_three_points(items)?;
    set_property(items, "Reference", reference, reference)?;
    set_property(items, "Value", &symbol.value, reference)?;
    set_symbol_fields(items, symbol)?;
    set_fitted_flags(items, &symbol.flags);
    put_pads_on_nets(items, reference, lookup.pins)?;
    note_unmatched_pins(&footprint, reference, lookup.pins, findings);
    Ok(Some(footprint))
}

/// The links to the symbol, where KiCad keeps them: before `attr` and the
/// drawings.
fn add_symbol_links(items: &mut Vec<Expr>, symbol: &Symbol) -> Result<(), String> {
    let mut links = Vec::new();
    if let Some(symbol_path) = &symbol.path {
        links.push(parse(&format!("(path {})", requote(symbol_path)))?);
    }
    if let Some(names) = symbol.sheet_path.and_then(|sheet| text_of(sheet, "names")) {
        links.push(parse(&format!("(sheetname {})", requote(&names)))?);
    }
    let sheet_file = symbol
        .component
        .children()
        .iter()
        .filter(|item| item.head() == Some("property"))
        .find(|item| form_atom(item, "name", 1) == Some("Sheetfile"))
        .and_then(|item| text_of(item, "value"));
    if let Some(sheet_file) = sheet_file {
        links.push(parse(&format!("(sheetfile {})", requote(&sheet_file)))?);
    }
    let at = items
        .iter()
        .position(|item| {
            matches!(
                item.head(),
                Some(
                    "attr"
                        | "clearance"
                        | "zone_connect"
                        | "solder_mask_margin"
                        | "solder_paste_margin"
                        | "pad"
                )
            ) || item.head().is_some_and(|head| head.starts_with("fp_"))
        })
        .unwrap_or(items.len());
    items.splice(at..at, links);
    Ok(())
}

/// The symbol's own fields as properties of the footprint.
fn set_symbol_fields(items: &mut Vec<Expr>, symbol: &Symbol) -> Result<(), String> {
    for field in symbol
        .component
        .child("fields")
        .map(Expr::children)
        .unwrap_or_default()
        .iter()
        .filter(|item| item.head() == Some("field"))
    {
        let Some(name) = text_of(field, "name")
            .filter(|name| !matches!(name.as_str(), "Footprint" | "Reference" | "Value"))
        else {
            continue;
        };
        set_property(
            items,
            &name,
            field.children().get(2).and_then(Expr::atom).unwrap_or(""),
            &symbol.reference,
        )?;
    }
    Ok(())
}

/// The symbol says whether the part is fitted and in the BOM (a
/// mounting hole's footprint may say otherwise).
fn set_fitted_flags(items: &mut Vec<Expr>, flags: &BTreeSet<String>) {
    let attr = match items.iter().position(|item| item.head() == Some("attr")) {
        Some(index) => index,
        None => {
            let at = items
                .iter()
                .position(|item| {
                    item.head() == Some("pad")
                        || item.head().is_some_and(|head| head.starts_with("fp_"))
                })
                .unwrap_or(items.len());
            items.insert(at, Expr::List(vec![Expr::Atom("attr".into())]));
            at
        }
    };
    if let Expr::List(parts) = &mut items[attr] {
        parts.retain(|item| !matches!(item.atom(), Some("dnp" | "exclude_from_bom")));
        for flag in ["exclude_from_bom", "dnp", "exclude_from_pos_files"] {
            if flags.contains(flag) && !parts.iter().any(|item| item.atom() == Some(flag)) {
                parts.push(Expr::Atom(flag.into()));
            }
        }
    }
    if items[attr].children().len() == 1 {
        items.remove(attr);
    }
}

/// Pads sharing a number share the pin's net, except a pin nothing
/// connects: KiCad gives each such pad its own net (`_1`, `_2`).
fn put_pads_on_nets(
    items: &mut [Expr],
    reference: &str,
    pins: &BTreeMap<(String, String), PinNet>,
) -> Result<(), String> {
    let mut repeats: BTreeMap<String, usize> = BTreeMap::new();
    for item in items.iter_mut() {
        if let Some("pad") = item.head() {
            let number = item
                .children()
                .get(1)
                .and_then(Expr::atom)
                .unwrap_or("")
                .to_string();
            if let Some((net, function, kind)) = pins.get(&(reference.to_string(), number.clone()))
                && let Expr::List(parts) = item
            {
                let seen = repeats.entry(number).or_insert(0);
                let net = if *seen > 0 && net.starts_with("unconnected-") {
                    format!("{net}_{seen}")
                } else {
                    net.clone()
                };
                *seen += 1;
                parts.push(parse(&format!("(net {})", requote(&net)))?);
                if let Some(function) = function {
                    parts.push(parse(&format!("(pinfunction {})", requote(function)))?);
                }
                if let Some(kind) = kind {
                    parts.push(parse(&format!("(pintype {})", requote(kind)))?);
                }
            }
        }
    }
    Ok(())
}

/// Symbol pins the footprint has no pad for (a library that changed).
fn note_unmatched_pins(
    footprint: &Expr,
    reference: &str,
    pins: &BTreeMap<(String, String), PinNet>,
    findings: &mut Findings,
) {
    let numbers: BTreeSet<&str> = footprint
        .children()
        .iter()
        .filter(|item| item.head() == Some("pad"))
        .filter_map(|pad| pad.children().get(1).and_then(Expr::atom))
        .collect();
    let lost: Vec<String> = pins
        .range((reference.to_string(), String::new())..)
        .take_while(|((owner, _), _)| owner == reference)
        .filter(|((_, pin), _)| !numbers.contains(pin.as_str()))
        .map(|((_, pin), (net, _, _))| format!("{pin} ({net})"))
        .collect();
    if !lost.is_empty() {
        findings
            .unmatched_pins
            .push(format!("{reference}: {}", lost.join(", ")));
    }
}
