// Copyright (C) 2026 Toit contributors.

//! A board from a schematic (or its netlist): every component's footprint
//! from the libraries, pads on their nets, linked to the symbols, all at
//! one point and without an outline, the way KiCad's "Update PCB from
//! Schematic" leaves a new board. `layout-kicad-board` with `move_all` and
//! an automatic outline takes it from there.

use super::*;
use crate::footprint_bodies::edge_connector;

mod added_parts;
mod clearances;
mod donor_connectivity;
mod donor_pin_numbers;
mod footprint_form;
mod instance_fields;
mod legacy_libraries;
mod libraries;
mod mechanical;
mod mechanical_parts;
mod mechanical_perimeter;
mod netlist;
mod parts;
mod project_copy;
mod report;
mod schematic_links;
pub(crate) use schematic_links::schematic_positions;
mod source_findings;
mod stackup;
mod starter;

use self::{
    added_parts::*, clearances::*, footprint_form::*, libraries::*, netlist::*, parts::*,
    project_copy::*, report::*, starter::*,
};
pub use clearances::{KiCadImportClearanceEvidence, KiCadImportNetclassGap};
pub use donor_connectivity::DonorConnectivity;
pub use donor_pin_numbers::{KiCadDonorPadInstance, KiCadDonorPadNumber, KiCadDonorPinNumbering};
pub use mechanical::{
    KiCadImportedMechanicalAddition, KiCadImportedMechanicalLock, KiCadImportedMechanics,
    KiCadUnmatchedMechanicalLock,
};
pub use mechanical_parts::{
    KiCadExcludedMechanicalObject, KiCadImportedMechanicalObject, KiCadMechanicalFiducial,
    KiCadMechanicalMetadataUuid, KiCadMechanicalPlatedDrill,
};
pub use source_findings::{
    KiCadImportFootprintEvidence, KiCadImportFootprintFinding, KiCadImportFootprintItem,
};
pub use stackup::KiCadImportedStackup;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadNetlistImport {
    pub board: PathBuf,
    pub footprints: usize,
    pub nets: usize,
    /// Components without a footprint field, or whose footprint was not
    /// found in the libraries (left out of the board): one line per
    /// footprint and reason, the parts that share it folded into it
    /// (`8 parts: D27, D29, ... (LLP1006-2M:VISHAY-LLP1006-2M not found:
    /// ...)`; a single part's line is `R2 (...)`).
    pub missing: Vec<String>,
    /// The parts `missing` leaves out of the board (its lines fold parts
    /// of one footprint).
    pub missing_parts: usize,
    /// Footprints not in the library the schematic names, taken from
    /// another library, under KiCad 9's name, under KiCad 10's (a vendor
    /// prefix: `Texas_DSG0008A_WSON-8-...`) or from an add-on library under
    /// its PCM nickname: `C1: <asked> -> <used> (<why>)`.
    /// The board uses the new name, and the importer updates the copied
    /// schematic where it can express the selected footprint.
    pub substituted: Vec<String>,
    /// The source assignment and the ID written on each affected PCB instance.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub footprint_id_changes: Vec<KiCadFootprintIdChange>,
    /// Board footprint links the copied schematic could not express per symbol.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub schematic_footprint_link_conflicts: Vec<String>,
    /// Footprints taken from KiCad's add-on libraries (its Plugin and
    /// Content Manager's 3rdparty folder): `U10: Espressif:ESP32-S3-MINI-1
    /// (<folder>)`. Used, not copied into the project.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub add_on_libraries: Vec<String>,
    /// Footprint libraries a fp-lib-table names at a folder that is not
    /// there (a path on the designer's machine), found by their last
    /// folders under the schematic's folder or one of its three parents:
    /// `AudioJacksFixed: the fp-lib-table's /home/.../AudioJacksFixed.pretty
    /// is not here; used <checkout>/hardware/libs/custom/AudioJacksFixed.pretty`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub relocated_libraries: Vec<String>,
    /// The board's copper layers and where the count came from: `--layers`,
    /// else `--stackup-from`, the mechanical board, the footprint donor,
    /// or the default of 2.
    pub copper_layers: usize,
    pub copper_layers_from: String,
    /// Explicit fabrication stack transferred from a source board.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stackup_from: Option<KiCadImportedStackup>,
    /// Board edges, rule areas and schematic-backed locked poses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mechanical_from: Option<KiCadImportedMechanics>,
    /// Symbol pins on a net that the footprint has no pad for (`J1: M1
    /// (GND)`): the footprint does not match the symbol. A pin on no net
    /// (KiCad's `unconnected-(...)`) loses no connection, but KiCad's
    /// schematic parity check still reports it (`net_conflict`).
    pub unmatched_pins: Vec<String>,
    /// Missing physical pads and their current schematic netlist evidence.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unmatched_pin_details: Vec<KiCadUnmatchedPin>,
    /// Conflicting schematic pin numbers resolved to physical donor pads.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub resolved_duplicate_pins: Vec<KiCadDuplicatePinBinding>,
    /// Footprints whose own pads are closer than the board's clearance (a
    /// solder jumper's) and got their pads' gap as their own clearance.
    pub clearances: Vec<String>,
    /// Whether import may synthesize per-footprint netclass exceptions.
    pub footprint_clearance_policy: &'static str,
    /// Supplied local policy and KiCad's measured own-pad clearance evidence.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub footprint_clearance_evidence: Vec<KiCadImportClearanceEvidence>,
    /// Own-pad gaps that violate a minimum or explicit rule a footprint
    /// clearance cannot replace. The importer keeps those rules in force.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub clearance_conflicts: Vec<String>,
    /// KiCad's built-in hole and pad geometry errors on the items of a
    /// single footprint, from the same isolated check used for clearances.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub footprint_findings: Vec<KiCadImportFootprintFinding>,
    /// Connectors the starter constraints put on an edge (USB, RF, audio,
    /// barrel and D-sub, network, video, card edge, terminal blocks).
    pub edge_connectors: Vec<String>,
    /// With `--footprints-from`: footprints no library has, taken from the
    /// board (`R1: lib:R_0402 (the board's R1)`) and written to a library
    /// in the project (`<library>.pretty`, in its `fp-lib-table`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footprints_from_board: Option<Vec<String>>,
    /// With `--footprints-from`: footprints the libraries have that the
    /// board has too, in the board's version, and of those the ones whose
    /// library version differs from the board's by more than a fifth on a
    /// side or in pad count (`lib:SW (64 parts: SW1, ...): library 15.9 x
    /// 12.2 mm, 4 pads; board 12.85 x 11.6 mm, 4 pads`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board_versions: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drifted_footprints: Option<Vec<String>>,
    /// Observed donor vs schematic connectivity among shared, unambiguous pads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub donor_connectivity: Option<DonorConnectivity>,
    /// Parts `constraints.json`'s `add` put on the board.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
    /// The key matrix found in the netlist and the starter constraints
    /// written for it (`constraints.starter.json`); absent when no part
    /// looks like a key switch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starter: Option<crate::key_matrix::KiCadStarterReport>,
    /// The starter constraints and layout config: written when new, never
    /// over existing files (a second import keeps the agent's edits: then
    /// listed in `kept`).
    pub constraints: Option<PathBuf>,
    pub layout: Option<PathBuf>,
    /// The files of these two that existed and were kept as they are.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub kept: Vec<String>,
    /// With `--near-decoupling`: the `near` entries of the decoupling
    /// capacitors (`describe-kicad-board --constraints-template`'s) the new
    /// `constraints.json` carries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub near_entries: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadFootprintIdChange {
    pub part: String,
    pub source_footprint: Option<String>,
    pub imported_footprint: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadUnmatchedPin {
    pub reference: String,
    pub footprint: String,
    pub pin: String,
    pub net: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_function: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_type: Option<String>,
    pub connection: KiCadUnmatchedPinConnection,
    /// Evidence from the matching physical instance on the supplied PCB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub donor_pin_numbering: Option<KiCadDonorPinNumbering>,
    pub next_action: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadUnmatchedPinConnection {
    NamedNet,
    NoConnect,
    Unwired,
}

const HEADER: &str = r#"(kicad_pcb
	(version 20260206)
	(generator "pcb-maker")
	(generator_version "10.0")
	(general (thickness 1.6) (legacy_teardrops no))
	(paper "A4")
	(layers
		(0 "F.Cu" signal)
		{inner}
		(2 "B.Cu" signal)
		(9 "F.Adhes" user "F.Adhesive")
		(11 "B.Adhes" user "B.Adhesive")
		(13 "F.Paste" user)
		(15 "B.Paste" user)
		(5 "F.SilkS" user "F.Silkscreen")
		(7 "B.SilkS" user "B.Silkscreen")
		(1 "F.Mask" user)
		(3 "B.Mask" user)
		(17 "Dwgs.User" user "User.Drawings")
		(19 "Cmts.User" user "User.Comments")
		(21 "Eco1.User" user "User.Eco1")
		(23 "Eco2.User" user "User.Eco2")
		(25 "Edge.Cuts" user)
		(27 "Margin" user)
		(31 "F.CrtYd" user "F.Courtyard")
		(29 "B.CrtYd" user "B.Courtyard")
		(35 "F.Fab" user)
		(33 "B.Fab" user)
	)
	(setup
		(pad_to_mask_clearance 0)
		(allow_soldermask_bridges_in_footprints no)
		(tenting (front yes) (back yes))
		(pcbplotparams
			(layerselection 0x00000000_00000000_55555555_5755f5ff)
			(plot_on_all_layers_selection 0x00000000_00000000_00000000_00000000)
			(usegerberextensions no)
			(usegerberattributes yes)
			(usegerberadvancedattributes yes)
			(creategerberjobfile yes)
			(outputformat 1)
			(mirror no)
			(drillshape 1)
			(scaleselection 1)
			(outputdirectory "")
		)
	)
)"#;

/// Writes `<output>/<board_id>.kicad_pcb` (and copies the schematic and
/// project files next to it) from a `.kicad_sch` or a KiCad netlist.
pub fn import_kicad_netlist(
    input: &Path,
    output: &Path,
    board_id: &str,
    layers: usize,
) -> Result<KiCadNetlistImport, String> {
    import_kicad_netlist_from(input, output, board_id, layers, None)
}

/// The library footprints taken from the board are written to: the
/// schematic's library when no library of that name exists here, else
/// this one (a library that lacks the footprint stays as it is).
const FROM_BOARD_LIBRARY: &str = "from_board";

/// A pin's net name, pin function and pin type.
type PinNet = (String, Option<String>, Option<String>);
type PinNets = BTreeMap<(String, String), Vec<PinNet>>;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadDuplicatePinBinding {
    pub reference: String,
    pub pin: String,
    pub pad_uuid: Option<String>,
    pub net: String,
    pub pin_function: Option<String>,
}

/// `import_kicad_netlist` with `--footprints-from <board>`: footprints no
/// library has are taken from that board, and the board's version is
/// preferred where it has the symbol's footprint (`board_footprints.rs`).
pub fn import_kicad_netlist_from(
    input: &Path,
    output: &Path,
    board_id: &str,
    layers: usize,
    footprints_from: Option<&Path>,
) -> Result<KiCadNetlistImport, String> {
    import_kicad_netlist_with(input, output, board_id, Some(layers), footprints_from)
}

/// `import_kicad_netlist_from` with the copper layers optional (the
/// command line's `--layers`): without them the `--footprints-from` board's
/// count, else 2. Fresh-agent trial 6's eurorack-pmod came out on two layers
/// from a six-layer designer's board, which the report did not say.
pub fn import_kicad_netlist_with(
    input: &Path,
    output: &Path,
    board_id: &str,
    layers: Option<usize>,
    footprints_from: Option<&Path>,
) -> Result<KiCadNetlistImport, String> {
    import_kicad_netlist_options(input, output, board_id, layers, footprints_from, false)
}

/// `import_kicad_netlist_with` and `--near-decoupling`: a new
/// `constraints.json` carries the decoupling capacitors' `near` entries
/// that `describe-kicad-board --constraints-template` writes for the
/// imported board (the same code: each capacitor at a supply pin of the
/// IC the schematic draws it nearest to).
pub fn import_kicad_netlist_options(
    input: &Path,
    output: &Path,
    board_id: &str,
    layers: Option<usize>,
    footprints_from: Option<&Path>,
    near_decoupling: bool,
) -> Result<KiCadNetlistImport, String> {
    import_kicad_netlist_with_options(
        input,
        output,
        board_id,
        KiCadNetlistImportOptions {
            layers,
            footprints_from,
            near_decoupling,
            ..Default::default()
        },
    )
}

#[derive(Clone, Copy, Debug, Default)]
pub struct KiCadNetlistImportOptions<'a> {
    pub layers: Option<usize>,
    pub footprints_from: Option<&'a Path>,
    pub stackup_from: Option<&'a Path>,
    pub mechanical_from: Option<&'a Path>,
    pub mechanical_footprints: &'a [String],
    pub near_decoupling: bool,
    pub keep_footprint_clearances: bool,
}

pub fn import_kicad_netlist_with_options(
    input: &Path,
    output: &Path,
    board_id: &str,
    options: KiCadNetlistImportOptions<'_>,
) -> Result<KiCadNetlistImport, String> {
    check_board_id(board_id)?;
    let board_path = output.join(format!("{board_id}.kicad_pcb"));
    let mut mechanical = options
        .mechanical_from
        .map(|source| mechanical::Mechanical::read(source, &board_path))
        .transpose()?;
    if let Some(mechanical) = &mut mechanical {
        mechanical.select_footprints(options.mechanical_footprints)?;
    } else if !options.mechanical_footprints.is_empty() {
        return Err(exit::input_error(
            "--mechanical-footprint requires --mechanical-from",
        ));
    }
    let stackup = options
        .stackup_from
        .map(|path| stackup::Stackup::read(path, &board_path, options.layers))
        .transpose()?;
    let designer = options
        .footprints_from
        .map(crate::board_footprints::BoardFootprints::read)
        .transpose()?;
    let (layers, copper_layers_from) = match &stackup {
        Some(stackup) => (
            stackup.report.copper_layers.len(),
            if options.layers.is_some() {
                "--layers"
            } else {
                "--stackup-from"
            }
            .to_string(),
        ),
        None => match (options.layers, &mechanical) {
            (None, Some(mechanical)) => (
                mechanical.copper_layers,
                if mechanical.report.preserved_existing_geometry {
                    "existing mechanical board"
                } else {
                    "--mechanical-from"
                }
                .into(),
            ),
            _ => copper_layers(options.layers, designer.as_ref()),
        },
    };
    let mut board = empty_board(layers)?;
    if let Some(stackup) = &stackup {
        stackup.apply(&mut board);
    }
    if let Some(mechanical) = &mechanical {
        mechanical.validate_layers(&board)?;
    }
    let project = input
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(output)
        .map_err(|error| format!("failed to create {}: {error}", output.display()))?;
    let is_schematic = input
        .extension()
        .is_some_and(|extension| extension == "kicad_sch");
    let netlist_path = if is_schematic {
        export_schematic(input, output, board_id, project)?
    } else {
        input.to_path_buf()
    };
    let netlist = read_netlist(&netlist_path)?;
    if let Some(mechanical) = &mechanical {
        mechanical.validate_schematic_nets(&netlist)?;
    }
    if !is_schematic {
        copy_netlist_project(input, output, board_id)?;
    }
    copy_custom_rules(
        input,
        output,
        board_id,
        project,
        (!is_schematic).then_some(&netlist),
    )?;
    let (libraries, library_notes) = find_libraries(project, output, &netlist, is_schematic);
    let (pins, nets) = pin_nets(&netlist);

    let components = netlist
        .child("components")
        .map(Expr::children)
        .unwrap_or_default();
    let donor_connectivity = designer
        .as_ref()
        .map(|designer| donor_connectivity::compare(designer, &netlist, components));
    let lookup = Lookup {
        libraries: &libraries,
        notes: &library_notes,
        designer: designer.as_ref(),
        pins: &pins,
        library_parts: netlist
            .child("libparts")
            .map(Expr::children)
            .unwrap_or_default(),
        verified_fields: None,
    };
    let mut findings = Findings::default();
    place_symbols(&mut board, components, &lookup, &mut findings, is_schematic)?;
    // Parts the schematic does not have (`add` in an existing
    // constraints.json).
    let added = add_parts(
        &mut board,
        output,
        &libraries,
        designer.as_ref(),
        &mut findings.missing,
        &mut findings.kept_ids,
    )?;
    if !findings.local_footprints.is_empty() {
        write_local_libraries(
            output,
            &findings.local_footprints,
            &libraries,
            &library_notes,
            designer.as_ref().map(|designer| designer.file.as_path()),
        )?;
    }
    fill_pad_uuids(&mut board);
    if let Some(mechanical) = &mut mechanical {
        let same_donor_additions = designer
            .as_ref()
            .filter(|donor| donor.sha256 == mechanical.report.source_sha256)
            .map(|_| &added.donor_instances);
        let references = components
            .iter()
            .filter_map(|component| form_atom(component, "ref", 1))
            .collect();
        mechanical.apply(
            &mut board,
            findings.footprints,
            &references,
            same_donor_additions,
        )?;
    }
    write_board(&board_path, &board)?;
    let schematic_footprint_link_conflicts = if is_schematic {
        schematic_links::update(project, output, board_id, components, &board)?
    } else {
        Vec::new()
    };
    let footprint_checks = apply_pad_gap_clearances(
        &mut board,
        output,
        board_id,
        &board_path,
        options.keep_footprint_clearances,
    )?;
    let (edges, edge_connectors) = edge_entries(&board)?;
    let mut files = StarterFiles::new(output);
    let mut starter = serde_json::json!({"version": 1, "move_all": true, "outline": {}});
    if let Some(mechanical) = &mechanical {
        starter
            .as_object_mut()
            .expect("starter object")
            .remove("outline");
        mechanical.constrain(&mut starter);
    } else if !edges.is_empty() {
        starter["edge"] = edges.into();
    }
    let starter_report = key_matrix_starter(
        &board,
        &findings.matrix_parts,
        &starter,
        findings.missing.len(),
        output,
    )?;
    let near_entries = if options.near_decoupling {
        Some(add_near_entries(output, board_id, &mut starter)?)
    } else {
        None
    };
    let constraints = files.write_new("constraints.json", starter)?;
    let layout = files.write_new(
        "layout.json",
        serde_json::json!({"placer": {"constraints": "constraints.json"}}),
    )?;
    Ok(KiCadNetlistImport {
        board: board_path,
        footprints: findings.footprints,
        nets,
        missing_parts: findings.missing.len(),
        missing: fold_missing(&findings.missing),
        substituted: findings.substituted,
        footprint_id_changes: findings.footprint_id_changes,
        schematic_footprint_link_conflicts,
        add_on_libraries: findings.add_on,
        relocated_libraries: library_notes.relocated.clone(),
        copper_layers: layers,
        copper_layers_from,
        stackup_from: stackup.map(|stackup| stackup.report),
        mechanical_from: mechanical.map(|mechanical| mechanical.report),
        unmatched_pins: findings.unmatched_pins,
        unmatched_pin_details: findings.unmatched_pin_details,
        resolved_duplicate_pins: findings.resolved_duplicate_pins,
        clearances: footprint_checks.clearances,
        footprint_clearance_policy: if options.keep_footprint_clearances {
            "preserve_supplied"
        } else {
            "adapt_to_pad_gap"
        },
        footprint_clearance_evidence: footprint_checks.clearance_evidence,
        clearance_conflicts: footprint_checks.clearance_conflicts,
        footprint_findings: footprint_checks.footprint_findings,
        edge_connectors,
        footprints_from_board: designer.is_some().then_some(findings.from_board),
        board_versions: designer.is_some().then_some(findings.board_versions),
        drifted_footprints: designer.is_some().then(|| drifted_lines(findings.drift)),
        donor_connectivity,
        added: added.lines,
        starter: starter_report,
        constraints,
        layout,
        kept: files.kept,
        near_entries,
    })
}

/// The copper layer count and where it came from: `--layers`, else the
/// `--footprints-from` board's, else 2.
fn copper_layers(
    layers: Option<usize>,
    designer: Option<&crate::board_footprints::BoardFootprints>,
) -> (usize, String) {
    let board_layers = designer
        .map(|designer| designer.copper_layers)
        .filter(|count| *count >= 2 && count % 2 == 0);
    match (layers, board_layers) {
        (Some(count), Some(board)) if board != count => (
            count,
            format!("--layers (the --footprints-from board has {board})"),
        ),
        (Some(count), _) => (count, "--layers".to_string()),
        (None, Some(board)) => (
            board,
            "the --footprints-from board's (--layers sets another)".to_string(),
        ),
        (None, None) => (2, "the default (--layers sets another)".to_string()),
    }
}

/// The board before any footprint: the header with `layers` copper layers.
fn empty_board(layers: usize) -> Result<Expr, String> {
    parse(
        &HEADER.replace(
            "{inner}",
            &(1..layers.saturating_sub(1))
                .map(|n| format!("({} \"In{n}.Cu\" signal)", 2 + 2 * n))
                .collect::<Vec<_>>()
                .join(" "),
        ),
    )
}

/// Writes the board file.
fn write_board(path: &Path, board: &Expr) -> Result<(), String> {
    fs::write(path, format!("{}\n", encode(board)))
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests;
