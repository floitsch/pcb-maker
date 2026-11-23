// Copyright (C) 2026 Toit contributors.

//! The board's footprints, one per symbol: found in the libraries or the
//! `--footprints-from` board, dressed with the symbol's links, fields, flags
//! and pad nets.

use super::instance_fields::{
    VerifiedFields, clear_example_text, project_sourcing_fields, sourcing_field,
};
use super::*;
use crate::board_footprints::{
    BoardFootprint, BoardFootprints, DonorModels, Found, same_library_model,
    same_library_model_with_fab_text, same_library_model_with_instance_documentation,
    same_two_pad_donor_model,
};

/// What the symbols' footprints are looked up in.
#[derive(Clone, Copy)]
pub(super) struct Lookup<'a> {
    pub libraries: &'a BTreeMap<String, PathBuf>,
    pub notes: &'a LibraryNotes,
    pub designer: Option<&'a BoardFootprints>,
    pub pins: &'a PinNets,
    pub library_parts: &'a [Expr],
    pub verified_fields: Option<&'a VerifiedFields>,
}

/// What the symbols' footprints leave for the report and the later stages.
#[derive(Default)]
pub(super) struct Findings {
    pub missing: Vec<String>,
    pub substituted: Vec<String>,
    pub footprint_id_changes: Vec<KiCadFootprintIdChange>,
    pub add_on: Vec<String>,
    pub unmatched_pins: Vec<String>,
    pub unmatched_pin_details: Vec<KiCadUnmatchedPin>,
    pub resolved_duplicate_pins: Vec<KiCadDuplicatePinBinding>,
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
fn matrix_part(symbol: &Symbol, pins: &PinNets) -> crate::key_matrix::MatrixPart {
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
            .filter_map(|((_, pin), bindings)| {
                let (net, _, kind) = unambiguous_net(bindings)?;
                Some((pin.clone(), net.clone(), kind.clone()))
            })
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
    from_schematic: bool,
) -> Result<(), String> {
    let symbols: Vec<_> = components
        .iter()
        .filter(|item| item.head() == Some("comp"))
        .map(read_symbol)
        .collect();
    let donors = lookup.designer.map_or_else(
        || vec![None; symbols.len()],
        |designer| {
            let identities: Vec<_> = symbols
                .iter()
                .map(|symbol| (symbol.path.as_deref(), symbol.reference.as_str()))
                .collect();
            designer.assign(&identities)
        },
    );
    let verified_fields = from_schematic.then(|| verified_symbol_fields(&symbols, &donors));
    let lookup = &Lookup {
        verified_fields: verified_fields.as_ref(),
        ..*lookup
    };
    for (symbol, own) in symbols.iter().zip(donors) {
        if symbol.flags.contains("exclude_from_board") {
            continue;
        }
        findings.matrix_parts.push(matrix_part(symbol, lookup.pins));
        let Some(resolved) = resolve_footprint(symbol, own, lookup, findings)? else {
            continue;
        };
        let Some(footprint) = dress_footprint(resolved, symbol, own, lookup, findings)? else {
            continue;
        };
        let source = text_of(symbol.component, "footprint").filter(|id| !id.is_empty());
        if let Some(imported) = footprint.children().get(1).and_then(Expr::atom)
            && source.as_deref() != Some(imported)
        {
            findings.footprint_id_changes.push(KiCadFootprintIdChange {
                part: symbol.reference.clone(),
                source_footprint: source,
                imported_footprint: imported.to_owned(),
            });
        }
        if let Expr::List(board_items) = &mut *board {
            board_items.push(footprint);
        }
        findings.footprints += 1;
    }
    Ok(())
}

fn verified_symbol_fields(
    symbols: &[Symbol<'_>],
    donors: &[Option<&BoardFootprint>],
) -> VerifiedFields {
    let mut by_reference = BTreeMap::new();
    for (symbol, donor) in symbols.iter().zip(donors) {
        let proven = (|| {
            let donor = donor.as_ref()?;
            if donor.reference != symbol.reference
                || text_of(symbol.component, "footprint").as_deref() != Some(donor.id.as_str())
                || symbol.flags.contains("exclude_from_board")
            {
                return None;
            }
            let mut fields = BTreeMap::new();
            for (name, value) in
                symbol_fields(symbol.component).filter(|(name, _)| sourcing_field(name))
            {
                fields
                    .entry(name)
                    .and_modify(|entry| *entry = None)
                    .or_insert(Some(value.to_string()));
            }
            Some(
                fields
                    .into_iter()
                    .filter_map(|(name, value)| Some((name, value?)))
                    .collect(),
            )
        })();
        by_reference
            .entry(symbol.reference.clone())
            .and_modify(|entry| *entry = None)
            .or_insert(proven);
    }
    by_reference
        .into_iter()
        .filter_map(|(reference, fields)| Some((reference, fields?)))
        .collect()
}

/// The footprint of a symbol: the library's, the `--footprints-from`
/// board's, or none (then the symbol is in `findings.missing`).
fn resolve_footprint(
    symbol: &Symbol,
    own: Option<&BoardFootprint>,
    lookup: &Lookup,
    findings: &mut Findings,
) -> Result<Option<Resolved>, String> {
    let reference = &symbol.reference;
    let authored = text_of(symbol.component, "footprint").filter(|id| !id.is_empty());
    let qualified = authored.as_deref().filter(|id| id.contains(':'));
    // A same-reference donor proves the model for an authored bare FPID.
    // Keep the FPID so KiCad can match it to the unchanged schematic.
    if let Some(id) = authored.as_deref().filter(|id| !id.contains(':'))
        && let Some(theirs) = own.filter(|item| item.reference == *reference && item.name() == id)
        && lookup.designer.is_some_and(|designer| {
            designer
                .footprints
                .iter()
                .filter(|item| item.reference == *reference)
                .take(2)
                .count()
                == 1
        })
    {
        return footprint_from_board(
            symbol,
            id,
            false,
            None,
            (theirs, Found::Own),
            lookup,
            findings,
        )
        .map(Some);
    }
    // Keep the existing fallback where no library-qualified name resolves.
    let named = qualified.map(str::to_owned);
    let unnamed = named.is_none();
    let Some(footprint_id) = named.or_else(|| {
        let own = own?;
        Some(if own.id.contains(':') {
            own.id.clone()
        } else {
            format!("{FROM_BOARD_LIBRARY}:{}", own.id)
        })
    }) else {
        if let Some(id) = authored {
            findings.missing.push(format!(
                "{reference} ({id} has no library nickname and no matching donor footprint)"
            ));
        } else {
            findings.missing.push(format!("{reference} (no footprint)"));
        }
        return Ok(None);
    };
    let nickname = footprint_id
        .split_once(':')
        .map_or("", |(nickname, _)| nickname);
    let library = (!lookup.notes.legacy.protected.contains(nickname))
        .then(|| find_footprint(lookup.libraries, &footprint_id))
        .flatten();
    let theirs = own.map(|footprint| (footprint, Found::Own)).or_else(|| {
        let designer = lookup.designer.filter(|_| library.is_none())?;
        designer
            .by_name(&footprint_id)
            .map(|footprint| (footprint, Found::ByName))
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
    let existing_model = library.as_ref().and_then(|(file, _)| {
        fs::read_to_string(file)
            .ok()
            .and_then(|text| parse(&text).ok())
    });
    if library.is_some() {
        // How far the library's version, which the import would
        // have used, is from the board's.
        findings.board_versions += 1;
        if let (Some(a), Some(b)) = (
            existing_model
                .as_ref()
                .and_then(crate::board_footprints::body_size),
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
    let authored_bare = !unnamed && !footprint_id.contains(':') && same_name && how == Found::Own;
    let authored_model = !unnamed && footprint_id == theirs.id && how == Found::Own;
    let donor_models =
        lookup
            .designer
            .filter(|_| authored_model)
            .map_or(DonorModels::Same, |donor| {
                donor.donor_models(
                    &wanted,
                    &form,
                    |_, _| {},
                    same_library_model_with_instance_documentation,
                )
            });
    let metadata_model = (donor_models == DonorModels::Incompatible)
        .then(|| schematic_metadata_model(lookup, &wanted, theirs, &form))
        .flatten();
    let donor_models = metadata_model
        .as_ref()
        .map_or(donor_models, |(models, _)| *models);
    let placement_roundoff = authored_model
        && library.is_none()
        && donor_models == DonorModels::Incompatible
        && lookup
            .designer
            .zip(lookup.verified_fields)
            .is_some_and(|(donor, verified)| {
                own_schematic_donors(donor, &wanted, verified)
                    && donor.donor_models(&wanted, &form, |_, _| {}, same_two_pad_donor_model)
                        == DonorModels::PerInstanceDocumentation
            });
    let shared_form = metadata_model.as_ref().map_or(&form, |(_, model)| model);
    let equivalent_presentation = if metadata_model.is_some() {
        same_library_model_with_fab_text
    } else {
        same_library_model_with_instance_documentation
    };
    let ambiguous = donor_models == DonorModels::Incompatible && !placement_roundoff;
    let instance_documentation = donor_models == DonorModels::PerInstanceDocumentation;
    let existing_library = libraries.get(wanted_library);
    let staged_donor_model = library.as_ref().is_some_and(|(file, _)| {
        lookup.notes.is_staged_model(file)
            && existing_model.as_ref().is_some_and(|model| {
                form_atom(model, "generator", 1) == Some("pcb-maker")
                    && (same_library_model(model, shared_form)
                        || (instance_documentation && equivalent_presentation(model, shared_form)))
            })
    });
    let replaceable_legacy = lookup
        .notes
        .legacy
        .replacements
        .contains_key(wanted_library);
    let fill_legacy = replaceable_legacy
        && authored_model
        && (library.is_none()
            || existing_model.as_ref().is_some_and(|model| {
                same_library_model(model, shared_form)
                    || (instance_documentation && equivalent_presentation(model, shared_form))
            }));
    let protected_legacy = lookup.notes.legacy.protected.contains(wanted_library)
        || (replaceable_legacy && !fill_legacy);
    if protected_legacy && lookup.notes.legacy.in_place {
        return Err(crate::exit::input_error(format!(
            "cannot stage the donor for {reference} ({wanted}) in the source project. Import into a separate output directory to preserve the original footprint table"
        )));
    }
    let restore_model = authored_model
        && !ambiguous
        && !protected_legacy
        && (fill_legacy
            || ((library.is_none() || staged_donor_model)
                && existing_library.is_some_and(|path| {
                    lookup.notes.supports_native_additions(wanted_library, path)
                })));
    let id = if authored_bare {
        findings
            .local_footprints
            .entry(format!("{FROM_BOARD_LIBRARY}:{wanted_name}"))
            .or_insert_with(|| form.clone());
        wanted.clone()
    } else if !protected_legacy
        && !restore_model
        && ((library.is_some() && same_name) || (!same_name && resolves))
    {
        wanted.clone()
    } else {
        let id = if restore_model
            || (existing_library.is_none()
                && !wanted_library.is_empty()
                && !ambiguous
                && !protected_legacy)
        {
            wanted.clone()
        } else {
            distinct_donor_id(wanted_name, &form, lookup, findings)
        };
        findings
            .local_footprints
            .entry(id.clone())
            .or_insert_with(|| {
                existing_model
                    .as_ref()
                    .filter(|_| restore_model && staged_donor_model)
                    .unwrap_or(shared_form)
                    .clone()
            });
        id
    };
    let mut whose = match how {
        Found::Own if same_name => format!("the board's {}", theirs.reference),
        Found::Own => format!(
            "the board's {}, which has another footprint than the schematic's",
            theirs.reference
        ),
        Found::ByName => format!("the board's {}, of that footprint name", theirs.reference),
    };
    if restore_model {
        whose.push_str("; restored the missing model under the schematic's footprint ID");
        if fill_legacy {
            whose.push_str(" using a project-local KiCad library for the missing legacy library");
        }
    } else if protected_legacy {
        whose.push_str(
            "; kept the legacy footprint table entry and staged the donor under a separate ID",
        );
    }
    if ambiguous {
        whose.push_str("; donor instances of the schematic's footprint ID have different models");
    }
    if instance_documentation && id == wanted {
        whose.push_str(
            "; shared the footprint ID and retained this instance's documentation graphics and fabrication text presentation",
        );
    }
    if metadata_model.is_some() && id == wanted {
        whose.push_str("; retained sourcing fields separately on the schematic's board instances");
    }
    if placement_roundoff && id == wanted {
        whose.push_str("; matched the two-pad SMD models with one-nanometre coordinate tolerance and retained each instance's geometry");
    }
    if unnamed {
        let why = text_of(symbol.component, "footprint")
            .filter(|value| !value.is_empty())
            .map_or_else(
                || "the schematic names no footprint".into(),
                |id| format!("the schematic names unqualified footprint {id}"),
            );
        findings
            .from_board
            .push(format!("{reference}: {id} ({whose}; {why})"));
    } else if authored_bare {
        findings.from_board.push(format!("{reference}: {id} ({whose}; kept the schematic's bare footprint ID, model in {FROM_BOARD_LIBRARY}.pretty)"));
    } else if restore_model || library.is_none() {
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

fn schematic_metadata_model(
    lookup: &Lookup,
    id: &str,
    donor: &BoardFootprint,
    form: &Expr,
) -> Option<(DonorModels, Expr)> {
    let designer = lookup.designer?;
    let verified = lookup.verified_fields?;
    if !own_schematic_donors(designer, id, verified) {
        return None;
    }
    let mut neutral = form.clone();
    project_sourcing_fields(&mut neutral, verified.get(&donor.reference)?);
    let models = designer.donor_models(
        id,
        &neutral,
        |other, model| project_sourcing_fields(model, &verified[&other.reference]),
        same_library_model_with_fab_text,
    );
    if models == DonorModels::Incompatible {
        return None;
    }
    clear_example_text(&mut neutral, donor.name());
    Some((models, neutral))
}

fn own_schematic_donors(designer: &BoardFootprints, id: &str, verified: &VerifiedFields) -> bool {
    // Only uniquely matched occurrences qualify as schematic instances.
    let mut references = BTreeSet::new();
    designer
        .footprints
        .iter()
        .filter(|other| other.id == id)
        .all(|other| {
            references.insert(other.reference.as_str()) && verified.contains_key(&other.reference)
        })
}

fn distinct_donor_id(name: &str, form: &Expr, lookup: &Lookup, findings: &Findings) -> String {
    if file_name_error(name).is_some() {
        return format!("{FROM_BOARD_LIBRARY}:{name}");
    }
    for suffix in 1.. {
        let name = if suffix == 1 {
            name.into()
        } else {
            format!("{name}_{suffix}")
        };
        let id = format!("{FROM_BOARD_LIBRARY}:{name}");
        if findings
            .local_footprints
            .get(&id)
            .is_some_and(|other| !same_library_model(other, form))
        {
            continue;
        }
        if let Some(folder) = lookup.libraries.get(FROM_BOARD_LIBRARY) {
            let file = folder.join(format!("{name}.kicad_mod"));
            if file.symlink_metadata().is_ok()
                && !fs::read_to_string(&file)
                    .ok()
                    .and_then(|text| parse(&text).ok())
                    .is_some_and(|other| same_library_model(&other, form))
            {
                continue;
            }
        }
        return id;
    }
    unreachable!()
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
    if lookup.notes.legacy.protected.contains(library)
        || lookup.notes.legacy.replacements.contains_key(library)
    {
        return format!(
            "{reference} ({footprint_id} not found: the footprint table names a legacy library. Convert it to a KiCad .pretty library or supply the matching board with --footprints-from and use a separate output project)"
        );
    }
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
    own: Option<&BoardFootprint>,
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
    put_pads_on_nets(
        items,
        reference,
        lookup.pins,
        own.map(|part| &part.expr),
        &mut findings.resolved_duplicate_pins,
    )?;
    let donor = lookup
        .designer
        .zip(own)
        .map(|(board, part)| donor_pin_numbers::Donor {
            board,
            part,
            component: symbol.component,
            library_parts: lookup.library_parts,
        });
    note_unmatched_pins(&footprint, reference, &id, lookup.pins, donor, findings);
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

fn symbol_fields(component: &Expr) -> impl Iterator<Item = (String, &str)> {
    component
        .child("fields")
        .map(Expr::children)
        .unwrap_or_default()
        .iter()
        .filter(|item| item.head() == Some("field"))
        .filter_map(|field| {
            let name = text_of(field, "name")
                .filter(|name| !matches!(name.as_str(), "Footprint" | "Reference" | "Value"))?;
            Some((
                name,
                field.children().get(2).and_then(Expr::atom).unwrap_or(""),
            ))
        })
}

/// The symbol's own fields as properties of the footprint.
fn set_symbol_fields(items: &mut Vec<Expr>, symbol: &Symbol) -> Result<(), String> {
    for (name, value) in symbol_fields(symbol.component) {
        set_property(items, &name, value, &symbol.reference)?;
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

fn unambiguous_net(bindings: &[PinNet]) -> Option<&PinNet> {
    let last = bindings.last()?;
    bindings
        .iter()
        .all(|binding| binding.0 == last.0)
        .then_some(last)
}

fn donor_pin<'a>(bindings: &'a [PinNet], pad: &Expr, number: &str) -> Option<&'a PinNet> {
    let function = text_of(pad, "pinfunction")?;
    let suffix = format!("_{number}");
    let mut matching = bindings.iter().filter(|(_, name, _)| {
        name.as_deref().is_some_and(|name| {
            name == function || name.strip_suffix(&suffix) == Some(function.as_str())
        })
    });
    let first = matching.next()?;
    matching
        .all(|binding| binding.0 == first.0)
        .then_some(first)
}

/// Pads with the same number normally share a net. A same-component donor
/// can identify distinct schematic pins by the function of each physical pad.
fn put_pads_on_nets(
    items: &mut [Expr],
    reference: &str,
    pins: &PinNets,
    own: Option<&Expr>,
    resolved_duplicate_pins: &mut Vec<KiCadDuplicatePinBinding>,
) -> Result<(), String> {
    let mut repeats = BTreeMap::<(String, String), usize>::new();
    // Importing the donor changes each pad's pose and fields in place. It
    // preserves pad order, including separate pads with the same number.
    let mut donor_pads = own
        .into_iter()
        .flat_map(Expr::children)
        .filter(|item| item.head() == Some("pad"));
    for item in items.iter_mut() {
        if let Some("pad") = item.head() {
            let number = item
                .children()
                .get(1)
                .and_then(Expr::atom)
                .unwrap_or("")
                .to_owned();
            let donor = donor_pads
                .next()
                .filter(|pad| pad.children().get(1).and_then(Expr::atom) == Some(number.as_str()));
            if let Some(bindings) = pins.get(&(reference.to_string(), number.clone())) {
                let ordinary_net = unambiguous_net(bindings);
                let (net, function, kind) = ordinary_net
                    .or_else(|| donor.and_then(|pad| donor_pin(bindings, pad, &number)))
                    .ok_or_else(|| {
                        let nets: BTreeSet<_> = bindings.iter().map(|binding| binding.0.as_str()).collect();
                        crate::exit::input_error(format!(
                            "{reference} pin {number} has competing schematic nets {}; give a same-component donor board with an unambiguous pin function on each physical pad, or resolve the schematic pin numbering",
                            nets.into_iter().collect::<Vec<_>>().join(", ")
                        ))
                    })?;
                let seen = repeats.entry((number.clone(), net.clone())).or_insert(0);
                let net = if *seen > 0 && net.starts_with("unconnected-") {
                    format!("{net}_{seen}")
                } else {
                    net.clone()
                };
                *seen += 1;
                if ordinary_net.is_none() {
                    resolved_duplicate_pins.push(KiCadDuplicatePinBinding {
                        reference: reference.into(),
                        pin: number,
                        pad_uuid: pcb_kicad_file::kicad_nets::footprint_uuid(item),
                        net: net.clone(),
                        pin_function: function.clone(),
                    });
                }
                let Expr::List(parts) = item else { continue };
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
    footprint_id: &str,
    pins: &PinNets,
    donor: Option<donor_pin_numbers::Donor<'_>>,
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
        .flat_map(|((_, pin), bindings)| bindings.iter().map(move |binding| (pin, binding)))
        .map(|(pin, (net, function, kind))| {
            let (connection, next_action) = if net.trim_start_matches('/').starts_with("unconnected-(") {
                let connection = if kind.as_ref().is_some_and(|kind| {
                    kind.split('+').any(|kind| kind == "no_connect")
                }) {
                    KiCadUnmatchedPinConnection::NoConnect
                } else {
                    KiCadUnmatchedPinConnection::Unwired
                };
                (connection, "verify which pins the selected package exposes. Use a matching symbol and footprint variant or correct the pad numbers, then re-import and verify schematic parity")
            } else {
                (KiCadUnmatchedPinConnection::NamedNet,
                 "restore the named net's terminal with a matching footprint or corrected pin numbering, then re-import and verify schematic parity")
            };
            let donor_pin_numbering = donor
                .filter(|_| connection == KiCadUnmatchedPinConnection::NamedNet)
                .and_then(|donor| donor.inspect(reference, pin, net, &numbers, pins));
            let next_action = if donor_pin_numbering
                .as_ref()
                .is_some_and(|numbering| numbering.status == "unique_candidate")
            {
                donor_pin_numbers::REVIEW_MAPPING
            } else {
                next_action
            };
            findings.unmatched_pin_details.push(KiCadUnmatchedPin {
                reference: reference.into(),
                footprint: footprint_id.into(),
                pin: pin.clone(),
                net: net.clone(),
                pin_function: function.clone(),
                pin_type: kind.clone(),
                connection,
                donor_pin_numbering,
                next_action,
            });
            format!("{pin} ({net})")
        })
        .collect();
    if !lost.is_empty() {
        findings
            .unmatched_pins
            .push(format!("{reference}: {}", lost.join(", ")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn donor_distinguishes_physical_pads_with_the_same_number() {
        let donor = parse(
            r#"(footprint "test"
          (pad "2" thru_hole circle (at 1 0) (size 1 1) (pinfunction "Thermistor"))
          (pad "2" thru_hole circle (at 2 0) (size 2 2) (pinfunction "GND")))"#,
        )
        .unwrap();
        let mut placed = crate::board_footprints::library_form(&donor, "test").unwrap();
        let Expr::List(items) = &mut placed else {
            panic!("expected footprint form")
        };
        let pins = PinNets::from(donor_pin_bindings());
        let mut resolved = Vec::new();
        assert!(put_pads_on_nets(items, "U1", &pins, None, &mut resolved).is_err());
        put_pads_on_nets(items, "U1", &pins, Some(&donor), &mut resolved).unwrap();
        assert_eq!(
            resolved
                .iter()
                .map(|binding| binding.net.as_str())
                .collect::<Vec<_>>(),
            ["/Thermistor", "GND"]
        );
        let pads: Vec<_> = items
            .iter()
            .filter(|item| item.head() == Some("pad"))
            .collect();
        assert_eq!(form_atom(pads[0], "net", 1), Some("/Thermistor"));
        assert_eq!(form_atom(pads[1], "net", 1), Some("GND"));
        assert_eq!(form_atom(pads[0], "size", 1), Some("1"));
        assert_eq!(form_atom(pads[1], "size", 1), Some("2"));
    }

    fn donor_pin_bindings() -> [((String, String), Vec<PinNet>); 1] {
        [(
            ("U1".into(), "2".into()),
            vec![
                (
                    "/Thermistor".into(),
                    Some("Thermistor_2".into()),
                    Some("output".into()),
                ),
                ("GND".into(), Some("GND_2".into()), Some("power_out".into())),
            ],
        )]
    }

    #[test]
    fn unmatched_pins_distinguish_named_nets_no_connects_and_unwired_pins() {
        let footprint = parse(r#"(footprint "fixture" (pad "4" smd rect))"#).unwrap();
        let pins = [
            (
                ("U1".into(), "1".into()),
                vec![("VCC".into(), Some("Supply".into()), Some("power_in".into()))],
            ),
            (
                ("U1".into(), "2".into()),
                vec![(
                    "unconnected-(U1-SIG-Pad2)".into(),
                    None,
                    Some("input+no_connect".into()),
                )],
            ),
            (
                ("U1".into(), "3".into()),
                vec![(
                    "unconnected-(U1-GPIO-Pad3)".into(),
                    None,
                    Some("input".into()),
                )],
            ),
            (("U1".into(), "4".into()), vec![("GND".into(), None, None)]),
        ]
        .into_iter()
        .collect();
        let mut findings = Findings::default();
        note_unmatched_pins(&footprint, "U1", "fixture", &pins, None, &mut findings);
        let pins = findings.unmatched_pin_details;
        assert_eq!(pins.len(), 3);
        assert_eq!(pins[0].reference, "U1");
        assert_eq!(pins[0].pin, "1");
        assert_eq!(pins[0].net, "VCC");
        assert_eq!(pins[0].connection, KiCadUnmatchedPinConnection::NamedNet);
        assert_eq!(pins[1].connection, KiCadUnmatchedPinConnection::NoConnect);
        assert_eq!(pins[2].connection, KiCadUnmatchedPinConnection::Unwired);
        assert_eq!(
            findings.unmatched_pins,
            ["U1: 1 (VCC), 2 (unconnected-(U1-SIG-Pad2)), 3 (unconnected-(U1-GPIO-Pad3))"]
        );
    }

    #[test]
    fn a_bare_footprint_id_is_preserved_only_with_matching_donor_identity() {
        let footprint = BoardFootprint {
            reference: "J1".into(),
            path: None,
            id: "Header".into(),
            uuid: None,
            expr: parse(
                r#"(footprint "Header" (layer "F.Cu") (at 10 20) (property "Reference" "J1"))"#,
            )
            .unwrap(),
        };
        let designer = BoardFootprints {
            file: PathBuf::new(),
            sha256: String::new(),
            footprints: vec![footprint],
            net_codes: BTreeMap::new(),
            copper_layers: 2,
        };
        let lookup = Lookup {
            libraries: &BTreeMap::new(),
            notes: &LibraryNotes::default(),
            designer: Some(&designer),
            pins: &BTreeMap::new(),
            library_parts: &[],
            verified_fields: None,
        };
        let symbol = parse(r#"(comp (ref "J1") (footprint "Header"))"#).unwrap();
        let mut findings = Findings::default();
        let own = designer.assign(&[(None, "J1")])[0];
        let resolved = resolve_footprint(&read_symbol(&symbol), own, &lookup, &mut findings)
            .unwrap()
            .unwrap();
        assert_eq!(resolved.id, "Header");
        assert!(findings.local_footprints.contains_key("from_board:Header"));
        assert!(findings.from_board[0].contains("kept the schematic's bare footprint ID"));

        let wrong_name = parse(r#"(comp (ref "J1") (footprint "Other"))"#).unwrap();
        let mut findings = Findings::default();
        let resolved = resolve_footprint(&read_symbol(&wrong_name), own, &lookup, &mut findings)
            .unwrap()
            .unwrap();
        assert_eq!(resolved.id, "from_board:Header");
        assert!(findings.from_board[0].contains("the schematic names unqualified footprint Other"));

        let mut ambiguous = designer.clone();
        ambiguous.footprints.push(ambiguous.footprints[0].clone());
        let lookup = Lookup {
            designer: Some(&ambiguous),
            ..lookup
        };
        let mut findings = Findings::default();
        assert!(
            resolve_footprint(
                &read_symbol(&symbol),
                ambiguous.assign(&[(None, "J1")])[0],
                &lookup,
                &mut findings
            )
            .unwrap()
            .is_none()
        );
        assert!(findings.missing[0].contains("no matching donor footprint"));
    }

    #[test]
    fn historical_model_uses_exact_id_and_conflicting_donors_stay_distinct() {
        let footprint = |reference: &str, width: f64, uuid: &str| {
            BoardFootprint {
            reference: reference.into(),
            path: None,
            id: "Stock:Historic".into(),
            uuid: None,
            expr: parse(&format!(r#"(footprint "Stock:Historic" (layer "F.Cu") (at 10 20)
                (fp_text reference "{reference}" (at 0 {width}) (layer "F.SilkS"))
                (pad "1" smd rect (at 0 0) (size {width} 1) (layers "F.Cu" "F.Mask") (uuid "{uuid}")))"#)).unwrap(),
        }
        };
        let mut designer = BoardFootprints {
            file: PathBuf::new(),
            sha256: String::new(),
            footprints: vec![footprint("J1", 1.0, "one"), footprint("J2", 1.0, "two")],
            net_codes: BTreeMap::new(),
            copper_layers: 2,
        };
        let libraries =
            BTreeMap::from([("Stock".into(), PathBuf::from("nonexistent/Stock.pretty"))]);
        let resolve = |designer: &BoardFootprints| {
            let lookup = Lookup {
                libraries: &libraries,
                notes: &LibraryNotes::default(),
                designer: Some(designer),
                pins: &BTreeMap::new(),
                library_parts: &[],
                verified_fields: None,
            };
            let mut findings = Findings::default();
            let ids = designer
                .footprints
                .iter()
                .map(|own| {
                    let symbol = parse(&format!(
                        r#"(comp (ref "{}") (footprint "Stock:Historic"))"#,
                        own.reference
                    ))
                    .unwrap();
                    resolve_footprint(&read_symbol(&symbol), Some(own), &lookup, &mut findings)
                        .unwrap()
                        .unwrap()
                        .id
                })
                .collect::<Vec<_>>();
            (ids, findings)
        };
        let (ids, findings) = resolve(&designer);
        assert_eq!(ids, ["Stock:Historic", "Stock:Historic"]);
        assert_eq!(findings.local_footprints.len(), 1);
        assert!(
            findings
                .from_board
                .iter()
                .all(|line| line.contains("restored the missing model"))
        );

        let Expr::List(items) = &mut designer.footprints[1].expr else {
            unreachable!()
        };
        items.push(parse(r#"(fp_rect (start 3 2) (end 4 3) (layer "Dwgs.User"))"#).unwrap());
        let (ids, findings) = resolve(&designer);
        assert_eq!(ids, ["Stock:Historic", "Stock:Historic"]);
        assert_eq!(findings.local_footprints.len(), 1);
        assert!(
            findings
                .from_board
                .iter()
                .all(|line| line.contains("retained this instance's documentation graphics"))
        );
        assert!(designer.by_name("Stock:Historic").is_none());

        designer.footprints[1] = footprint("J2", 2.0, "two");
        let (ids, findings) = resolve(&designer);
        assert_eq!(ids, ["from_board:Historic", "from_board:Historic_2"]);
        assert_eq!(findings.local_footprints.len(), 2);
        assert!(
            findings
                .from_board
                .iter()
                .all(|line| line.contains("different models"))
        );
        assert!(designer.by_name("Stock:Historic").is_none());
    }

    #[test]
    fn a_shared_land_pattern_keeps_own_schematic_fields_on_each_instance() {
        let footprint = |reference: &str, code: &str, extra: &str| BoardFootprint {
            reference: reference.into(),
            path: None,
            id: "Stock:Case0402".into(),
            uuid: None,
            expr: parse(&format!(
                r#"(footprint "Stock:Case0402" (layer "F.Cu") (at 10 20)
              (property "Reference" "{reference}" (at 0 -1 0) (layer "F.SilkS") (hide yes)
                (render_cache "{reference}" 0 (polygon (pts (xy 0 0) (xy 1 1) (xy 0 1)))))
              (property "LCSC" "{code}" (at 0 0 0) (layer "F.Fab") (hide yes))
              (pad "1" smd rect (at 0 0) (size 0.8 1) (layers "F.Cu" "F.Mask")) {extra})"#
            ))
            .unwrap(),
        };
        let symbols = parse(
            r#"(components
            (comp (ref "C1") (value "100nF") (footprint "Stock:Case0402")
              (fields (field (name "LCSC") "C101")))
            (comp (ref "R2") (value "10k") (footprint "Stock:Case0402")
              (fields (field (name "LCSC") "C202"))))"#,
        )
        .unwrap();
        let mut designer = BoardFootprints {
            file: PathBuf::new(),
            sha256: String::new(),
            footprints: vec![footprint("C1", "C101", ""), footprint("R2", "C202", "")],
            net_codes: BTreeMap::new(),
            copper_layers: 2,
        };
        let import = |designer: &BoardFootprints, from_schematic| {
            let mut board = parse("(kicad_pcb)").unwrap();
            let lookup = Lookup {
                libraries: &BTreeMap::new(),
                notes: &LibraryNotes::default(),
                designer: Some(designer),
                pins: &BTreeMap::new(),
                library_parts: &[],
                verified_fields: None,
            };
            let mut findings = Findings::default();
            place_symbols(
                &mut board,
                symbols.children(),
                &lookup,
                &mut findings,
                from_schematic,
            )
            .unwrap();
            (board, findings)
        };
        let (board, findings) = import(&designer, true);
        assert_eq!(findings.footprints, 2);
        assert_eq!(findings.local_footprints.len(), 1);
        let model = &findings.local_footprints["Stock:Case0402"];
        let template = encode(model);
        assert!(!template.contains("LCSC") && !template.contains("render_cache"));
        assert!(template.contains(r#"(property "Reference" "REF**""#));
        let output = encode(&board);
        for (reference, code) in [("C1", "C101"), ("R2", "C202")] {
            let instance = board
                .children()
                .iter()
                .find(|item| footprint_reference(item).as_deref() == Some(reference))
                .unwrap();
            let text = encode(instance);
            assert!(text.contains(&format!(r#"(property "LCSC" "{code}""#)));
            assert!(text.contains(&format!(r#"(render_cache "{reference}" 0"#)));
        }
        assert!(
            output.contains(r#"(property "Value" "100nF""#)
                && output.contains(r#"(property "Value" "10k""#)
        );
        assert!(
            !import(&designer, false)
                .1
                .local_footprints
                .contains_key("Stock:Case0402")
        );
        for (code, extra) in [
            ("C999", ""),
            ("C202", "(attr smd dnp)"),
            ("C202", "(solder_mask_margin 0.15)"),
            (
                "C202",
                r#"(fp_rect (start 5 5) (end 6 6) (layer "User.Drawings"))"#,
            ),
        ] {
            designer.footprints[1] = footprint("R2", code, extra);
            let (_, findings) = import(&designer, true);
            assert_eq!(findings.local_footprints.len(), 2, "{code}, {extra}");
            assert!(!findings.local_footprints.contains_key("Stock:Case0402"));
        }
    }
}
