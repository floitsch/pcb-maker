// Copyright (C) 2026 Toit contributors.

//! A board from a schematic (or its netlist): every component's footprint
//! from the libraries, pads on their nets, linked to the symbols, all at
//! one point and without an outline, the way KiCad's "Update PCB from
//! Schematic" leaves a new board. `layout-kicad-board` with `move_all` and
//! an automatic outline takes it from there.

use super::*;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadNetlistImport {
    pub board: PathBuf,
    pub footprints: usize,
    pub nets: usize,
    /// Components without a footprint field, or whose footprint was not
    /// found in the libraries (left out of the board).
    pub missing: Vec<String>,
    /// Footprints not in the library the schematic names, taken from
    /// another library or under KiCad 9's name: `C1: <asked> -> <used>`.
    pub substituted: Vec<String>,
    /// Footprints whose own pads are closer than the board's clearance (a
    /// solder jumper's) and got their pads' gap as their own clearance.
    pub clearances: Vec<String>,
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

/// A deterministic UUID from text (version 4 layout).
fn uuid_of(text: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut second = 0x8422_2325_cbf2_9ce4u64;
    for byte in text.bytes() {
        hash = (hash ^ byte as u64).wrapping_mul(0x100_0000_01b3);
        second = (second ^ byte as u64).wrapping_mul(0x100_0000_01b3).rotate_left(5);
    }
    format!(
        "{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}",
        hash >> 32,
        (hash >> 16) & 0xffff,
        hash & 0xfff,
        second & 0xfff,
        (second >> 12) & 0xffff_ffff_ffff
    )
}

/// Footprint libraries by name, from fp-lib-tables (the project's first,
/// then the user's), with KiCad's usual variables.
fn footprint_libraries(project: &Path) -> BTreeMap<String, PathBuf> {
    let system = std::env::var("KICAD10_FOOTPRINT_DIR")
        .or_else(|_| std::env::var("KICAD9_FOOTPRINT_DIR"))
        .unwrap_or_else(|_| "/usr/share/kicad/footprints".into());
    let mut tables = vec![project.join("fp-lib-table")];
    if let Ok(home) = std::env::var("HOME") {
        for version in ["10.0", "9.0"] {
            tables.push(Path::new(&home).join(".config/kicad").join(version).join("fp-lib-table"));
        }
    }
    let mut libraries = BTreeMap::new();
    for table in tables {
        let Ok(text) = fs::read_to_string(&table) else {
            continue;
        };
        let Ok(parsed) = parse(&text) else {
            continue;
        };
        for lib in parsed.children().iter().filter(|item| item.head() == Some("lib")) {
            let (Some(name), Some(uri)) = (form_atom(lib, "name", 1), form_atom(lib, "uri", 1)) else {
                continue;
            };
            let mut path = uri.to_string();
            for (variable, value) in [
                ("KIPRJMOD", project.display().to_string()),
                ("KICAD10_FOOTPRINT_DIR", system.clone()),
                ("KICAD9_FOOTPRINT_DIR", system.clone()),
                ("KICAD8_FOOTPRINT_DIR", system.clone()),
            ] {
                path = path.replace(&format!("${{{variable}}}"), &value).replace(&format!("$({variable})"), &value);
            }
            libraries.entry(name.to_string()).or_insert_with(|| PathBuf::from(path));
        }
    }
    // KiCad's own libraries, even without a table.
    if let Ok(entries) = fs::read_dir(&system) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|name| name.to_str()).and_then(|name| name.strip_suffix(".pretty")) {
                libraries.entry(name.to_string()).or_insert(path);
            }
        }
    }
    libraries
}

/// A footprint's file: in its library, else the same name in any library
/// (projects often carry copies), also under KiCad 9's names for
/// connectors (Female and Male became Socket and Plug).
fn find_footprint(libraries: &BTreeMap<String, PathBuf>, id: &str) -> Option<(PathBuf, String)> {
    let (library, name) = id.split_once(':')?;
    let mut names = vec![name.to_string()];
    let renamed = name.replace("_Female", "_Socket").replace("_Male", "_Plug");
    if renamed != name {
        names.push(renamed);
    }
    let file_in = |library: &str, name: &str| {
        let file = libraries.get(library)?.join(format!("{name}.kicad_mod"));
        file.exists().then(|| (file, format!("{library}:{name}")))
    };
    names.iter().find_map(|name| file_in(library, name).or_else(|| libraries.keys().find_map(|other| file_in(other, name))))
}

/// Sets a footprint property's value, adding it (hidden, on the fab layer,
/// as KiCad does) when the footprint has none of that name.
fn set_property(items: &mut Vec<Expr>, name: &str, value: &str, reference: &str) -> Result<(), String> {
    for item in items.iter_mut().filter(|item| item.head() == Some("property")) {
        if let Expr::List(parts) = item
            && parts.get(1).and_then(Expr::atom) == Some(name)
            && parts.len() > 2
        {
            parts[2] = Expr::Atom(quote(value));
            return Ok(());
        }
    }
    let last = items.iter().rposition(|item| item.head() == Some("property")).map_or(items.len(), |index| index + 1);
    items.insert(
        last,
        parse(&format!(
            "(property {} {} (at 0 0 0) (unlocked yes) (layer \"F.Fab\") (hide yes) (uuid \"{}\") (effects (font (size 1.27 1.27) (thickness 0.15))))",
            quote(name),
            quote(value),
            uuid_of(&format!("property {reference} {name}"))
        ))?,
    );
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    if from.is_dir() {
        fs::create_dir_all(to).map_err(|error| format!("failed to create {}: {error}", to.display()))?;
        for entry in fs::read_dir(from).map_err(|error| error.to_string())?.flatten() {
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else if from.exists() {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::copy(from, to).map_err(|error| format!("failed to copy {}: {error}", from.display()))?;
    }
    Ok(())
}

fn text_of(node: &Expr, head: &str) -> Option<String> {
    form_atom(node, head, 1).map(str::to_owned)
}

/// Writes `<output>/<board_id>.kicad_pcb` (and copies the schematic and
/// project files next to it) from a `.kicad_sch` or a KiCad netlist.
pub fn import_kicad_netlist(input: &Path, output: &Path, board_id: &str, layers: usize) -> Result<KiCadNetlistImport, String> {
    let project = input.parent().unwrap_or(Path::new(".")).to_path_buf();
    fs::create_dir_all(output).map_err(|error| format!("failed to create {}: {error}", output.display()))?;
    let is_schematic = input.extension().is_some_and(|extension| extension == "kicad_sch");
    let netlist_path = if is_schematic {
        let path = output.join(format!("{board_id}.net"));
        let result = Command::new("kicad-cli")
            .args(["sch", "export", "netlist", "--format", "kicadsexpr", "-o"])
            .arg(&path)
            .arg(input)
            .output()
            .map_err(|error| format!("kicad-cli: {error}"))?;
        if !result.status.success() {
            return Err(format!("kicad-cli sch export netlist failed: {}", String::from_utf8_lossy(&result.stderr)));
        }
        // The project travels with the board: schematic sheets (the root
        // renamed after the board), project settings, library tables.
        let stem = input.file_stem().and_then(|stem| stem.to_str()).unwrap_or("");
        for entry in fs::read_dir(&project).map_err(|error| error.to_string())?.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("").to_string();
            let target = if name == format!("{stem}.kicad_sch") {
                format!("{board_id}.kicad_sch")
            } else if name == format!("{stem}.kicad_pro") {
                format!("{board_id}.kicad_pro")
            } else if name.ends_with(".kicad_sch") || name == "sym-lib-table" || name == "fp-lib-table" {
                name
            } else {
                continue;
            };
            fs::copy(&path, output.join(target)).map_err(|error| error.to_string())?;
        }
        // And the project's own libraries its tables name.
        for table in ["fp-lib-table", "sym-lib-table"] {
            let Ok(parsed) = fs::read_to_string(project.join(table)).map_err(|error| error.to_string()).and_then(|text| parse(&text)) else {
                continue;
            };
            for lib in parsed.children().iter().filter(|item| item.head() == Some("lib")) {
                let uri = form_atom(lib, "uri", 1).unwrap_or("");
                let Some(relative) = uri.strip_prefix("${KIPRJMOD}/").or_else(|| uri.strip_prefix("$(KIPRJMOD)/")) else {
                    continue;
                };
                copy_tree(&project.join(relative), &output.join(relative))?;
            }
        }
        path
    } else {
        input.to_path_buf()
    };
    let netlist = parse(&fs::read_to_string(&netlist_path).map_err(|error| format!("failed to read {}: {error}", netlist_path.display()))?)?;
    let libraries = footprint_libraries(&project);

    // Nets by (reference, pin): name, pin function and type.
    let mut pins: BTreeMap<(String, String), (String, Option<String>, Option<String>)> = BTreeMap::new();
    let mut nets = 0;
    if let Some(section) = netlist.child("nets") {
        for net in section.children().iter().filter(|item| item.head() == Some("net")) {
            let Some(name) = text_of(net, "name") else {
                continue;
            };
            nets += 1;
            for node in net.children().iter().filter(|item| item.head() == Some("node")) {
                let (Some(reference), Some(pin)) = (text_of(node, "ref"), text_of(node, "pin")) else {
                    continue;
                };
                pins.insert((reference, pin), (name.clone(), text_of(node, "pinfunction"), text_of(node, "pintype")));
            }
        }
    }

    let mut board = parse(&HEADER.replace(
        "{inner}",
        &(1..layers.saturating_sub(1)).map(|n| format!("({} \"In{n}.Cu\" signal)", 2 + 2 * n)).collect::<Vec<_>>().join(" "),
    ))?;
    let mut missing = Vec::new();
    let mut substituted = Vec::new();
    let mut footprints = 0;
    let components = netlist.child("components").map(Expr::children).unwrap_or_default();
    for component in components.iter().filter(|item| item.head() == Some("comp")) {
        let reference = text_of(component, "ref").unwrap_or_default();
        let value = text_of(component, "value").unwrap_or_default();
        let Some(footprint_id) = text_of(component, "footprint").filter(|id| id.contains(':')) else {
            missing.push(format!("{reference} (no footprint)"));
            continue;
        };
        // Symbol properties without a value are flags.
        let flags: BTreeSet<String> = component
            .children()
            .iter()
            .filter(|item| item.head() == Some("property") && form_atom(item, "value", 1).is_none())
            .filter_map(|item| text_of(item, "name"))
            .collect();
        if flags.contains("exclude_from_board") {
            continue;
        }
        let Some((file, found)) = find_footprint(&libraries, &footprint_id) else {
            missing.push(format!("{reference} ({footprint_id} not found)"));
            continue;
        };
        if found != footprint_id {
            substituted.push(format!("{reference}: {footprint_id} -> {found}"));
        }
        let mut footprint = parse(&fs::read_to_string(&file).map_err(|error| format!("failed to read {}: {error}", file.display()))?)?;
        let Expr::List(items) = &mut footprint else {
            continue;
        };
        items.retain(|item| !matches!(item.head(), Some("version" | "generator" | "generator_version")));
        items[1] = Expr::Atom(quote(&footprint_id));
        // Placement fields after the layer.
        let layer_index = items.iter().position(|item| item.head() == Some("layer")).map_or(2, |index| index + 1);
        items.insert(layer_index, parse(&format!("(uuid \"{}\")", uuid_of(&format!("footprint {reference}"))))?);
        items.insert(layer_index + 1, parse("(at 0 0)")?);
        // The links to the symbol, where KiCad keeps them: before `attr`
        // and the drawings.
        let mut links = Vec::new();
        let sheet_path = component.child("sheetpath");
        let sheet = sheet_path.and_then(|sheet| text_of(sheet, "tstamps")).unwrap_or_else(|| "/".into());
        if let Some(stamp) = text_of(component, "tstamps").or_else(|| text_of(component, "tstamp")) {
            links.push(parse(&format!("(path {})", quote(&format!("{}{stamp}", if sheet.ends_with('/') { sheet.clone() } else { format!("{sheet}/") }))))?);
        }
        if let Some(names) = sheet_path.and_then(|sheet| text_of(sheet, "names")) {
            links.push(parse(&format!("(sheetname {})", quote(&names)))?);
        }
        let sheet_file = component
            .children()
            .iter()
            .filter(|item| item.head() == Some("property"))
            .find(|item| form_atom(item, "name", 1) == Some("Sheetfile"))
            .and_then(|item| text_of(item, "value"));
        if let Some(sheet_file) = sheet_file {
            links.push(parse(&format!("(sheetfile {})", quote(&sheet_file)))?);
        }
        let at = items
            .iter()
            .position(|item| matches!(item.head(), Some("attr" | "clearance" | "zone_connect" | "solder_mask_margin" | "solder_paste_margin" | "pad") ) || item.head().is_some_and(|head| head.starts_with("fp_")))
            .unwrap_or(items.len());
        items.splice(at..at, links);
        // The symbol's fields (datasheet, description, LCSC numbers, ...).
        set_property(items, "Reference", &reference, &reference)?;
        set_property(items, "Value", &value, &reference)?;
        for field in component.child("fields").map(Expr::children).unwrap_or_default().iter().filter(|item| item.head() == Some("field")) {
            let Some(name) = text_of(field, "name").filter(|name| !matches!(name.as_str(), "Footprint" | "Reference" | "Value")) else {
                continue;
            };
            set_property(items, &name, field.children().get(2).and_then(Expr::atom).unwrap_or(""), &reference)?;
        }
        for flag in ["dnp", "exclude_from_bom", "exclude_from_pos_files"] {
            if flags.contains(flag)
                && let Some(Expr::List(attr)) = items.iter_mut().find(|item| item.head() == Some("attr"))
                && !attr.iter().any(|item| item.atom() == Some(flag))
            {
                attr.push(Expr::Atom(flag.into()));
            }
        }
        for item in items.iter_mut() {
            match item.head() {
                Some("pad") => {
                    let number = item.children().get(1).and_then(Expr::atom).unwrap_or("").to_string();
                    if let Some((net, function, kind)) = pins.get(&(reference.clone(), number))
                        && let Expr::List(parts) = item
                    {
                        parts.push(parse(&format!("(net {})", quote(net)))?);
                        if let Some(function) = function {
                            parts.push(parse(&format!("(pinfunction {})", quote(function)))?);
                        }
                        if let Some(kind) = kind {
                            parts.push(parse(&format!("(pintype {})", quote(kind)))?);
                        }
                    }
                }
                _ => {}
            }
        }
        if let Expr::List(board_items) = &mut board {
            board_items.push(footprint);
        }
        footprints += 1;
    }
    let board_path = output.join(format!("{board_id}.kicad_pcb"));
    fs::write(&board_path, format!("{}\n", encode(&board))).map_err(|error| format!("failed to write {}: {error}", board_path.display()))?;
    // Pads of one footprint closer than the board's clearance (a solder
    // jumper's): no placement or routing changes that. The footprint gets
    // its pads' gap as its own clearance, as designers do.
    let mut clearances = Vec::new();
    let gaps = own_pad_gaps(&board, output, board_id)?;
    if !gaps.is_empty() {
        let Expr::List(items) = &mut board else {
            unreachable!("the board is a list")
        };
        for footprint in items.iter_mut().filter(|item| item.head() == Some("footprint")) {
            let reference = footprint
                .children()
                .iter()
                .find(|item| item.head() == Some("property") && item.children().get(1).and_then(Expr::atom) == Some("Reference"))
                .and_then(|item| item.children().get(2).and_then(Expr::atom))
                .unwrap_or("")
                .to_string();
            let Some((gap, rule)) = gaps.get(&reference) else {
                continue;
            };
            let clearance = (gap * 100.0).floor() / 100.0;
            let Expr::List(parts) = footprint else {
                continue;
            };
            parts.retain(|item| item.head() != Some("clearance"));
            let at = parts.iter().position(|item| item.head() == Some("attr")).unwrap_or(parts.len());
            parts.insert(at, parse(&format!("(clearance {clearance})"))?);
            clearances.push(format!("{reference}: {clearance} mm (its pads are {gap} mm apart, the rule is {rule} mm)"));
        }
        fs::write(&board_path, format!("{}\n", encode(&board))).map_err(|error| format!("failed to write {}: {error}", board_path.display()))?;
    }
    Ok(KiCadNetlistImport { board: board_path, footprints, nets, missing, substituted, clearances })
}

/// Per footprint, the smallest gap between its own pads that KiCad's DRC
/// finds below the clearance rule, and that rule. The footprints are
/// checked apart from each other, in rows, with the project's rules (on a
/// stack of footprints KiCad stops reporting after 499 clearance errors).
fn own_pad_gaps(board: &Expr, project: &Path, board_id: &str) -> Result<BTreeMap<String, (f64, f64)>, String> {
    let mut spread = board.clone();
    let Expr::List(items) = &mut spread else {
        return Ok(BTreeMap::new());
    };
    let (mut x, mut y, mut row) = (0.0f64, 0.0f64, 0.0f64);
    for footprint in items.iter_mut().filter(|item| item.head() == Some("footprint")) {
        // Pads reach this far from the origin.
        let reach = footprint
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
            .map(|pad| {
                let at = form_at(pad).unwrap_or([0.0; 3]);
                let size = form_f64(pad, "size", 1).unwrap_or(0.0).max(form_f64(pad, "size", 2).unwrap_or(0.0));
                at[0].abs().max(at[1].abs()) + size
            })
            .fold(0.0, f64::max)
            + 5.0;
        if x + 2.0 * reach > 1000.0 {
            (x, y, row) = (0.0, y + row, 0.0);
        }
        if let Expr::List(parts) = footprint
            && let Some(at) = parts.iter_mut().find(|item| item.head() == Some("at"))
        {
            *at = parse(&format!("(at {} {})", x + reach, y + reach))?;
        }
        x += 2.0 * reach;
        row = row.max(2.0 * reach);
    }
    let directory = project.join(".pad-check");
    fs::create_dir_all(&directory).map_err(|error| format!("failed to create {}: {error}", directory.display()))?;
    let settings = project.join(format!("{board_id}.kicad_pro"));
    if settings.exists() {
        fs::copy(&settings, directory.join(format!("{board_id}.kicad_pro"))).map_err(|error| error.to_string())?;
    }
    let board = directory.join(format!("{board_id}.kicad_pcb"));
    fs::write(&board, format!("{}\n", encode(&spread))).map_err(|error| format!("failed to write {}: {error}", board.display()))?;
    let report = directory.join("drc.json");
    // Without kicad-cli (a netlist from elsewhere) there is no check.
    let Ok(output) = Command::new("kicad-cli")
        .args(["pcb", "drc", "--format", "json", "--severity-error", "-o"])
        .arg(&report)
        .arg(&board)
        .output()
    else {
        let _ = fs::remove_dir_all(&directory);
        return Ok(BTreeMap::new());
    };
    let text = fs::read_to_string(&report);
    let _ = fs::remove_dir_all(&directory);
    if !output.status.success() {
        return Err(format!("kicad-cli pcb drc failed: {}", String::from_utf8_lossy(&output.stderr)));
    }
    let text = text.map_err(|error| format!("failed to read the pad check's DRC report: {error}"))?;
    let drc: serde_json::Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    // "Clearance violation (netclass 'Default' clearance 0.2800 mm; actual 0.2496 mm)"
    let number_after = |text: &str, key: &str| -> Option<f64> {
        let rest = &text[text.find(key)? + key.len()..];
        rest.trim_start().split_whitespace().next()?.parse().ok()
    };
    let mut gaps: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    for violation in drc["violations"].as_array().into_iter().flatten() {
        if violation["type"] != "clearance" {
            continue;
        }
        let owners: Vec<Option<&str>> = violation["items"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|item| {
                let description = item["description"].as_str()?;
                description.starts_with("Pad ").then_some(())?;
                description.rsplit_once(" of ")?.1.split_whitespace().next()
            })
            .collect();
        let description = violation["description"].as_str().unwrap_or("");
        if let ([Some(a), Some(b)], Some(gap), Some(rule)) =
            (owners.as_slice(), number_after(description, "actual"), number_after(description, "clearance"))
            && a == b
        {
            let entry = gaps.entry(a.to_string()).or_insert((gap, rule));
            entry.0 = entry.0.min(gap);
        }
    }
    Ok(gaps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netlist_becomes_a_board_with_nets_fields_and_symbol_links() {
        let directory = std::env::temp_dir().join(format!(
            "pcb-maker-netlist-import-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        let library = directory.join("libs/parts.pretty");
        fs::create_dir_all(&library).unwrap();
        fs::write(
            directory.join("fp-lib-table"),
            r#"(fp_lib_table (version 7) (lib (name "parts") (type "KiCad") (uri "$(KIPRJMOD)/libs/parts.pretty") (options "") (descr "")))"#,
        )
        .unwrap();
        fs::write(
            library.join("R_Test.kicad_mod"),
            r#"(footprint "R_Test" (version 20260206) (generator "test") (layer "F.Cu")
                (property "Reference" "REF**" (at 0 -1.5 0) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
                (property "Value" "R_Test" (at 0 1.5 0) (layer "F.Fab") (effects (font (size 1 1) (thickness 0.15))))
                (attr smd)
                (pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu" "F.Paste" "F.Mask"))
                (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu" "F.Paste" "F.Mask")))"#,
        )
        .unwrap();
        let netlist = directory.join("test.net");
        fs::write(
            &netlist,
            r#"(export (version "E")
              (components
                (comp (ref "R1") (value "10k") (footprint "parts:R_Test")
                  (fields (field (name "Footprint") "parts:R_Test") (field (name "Datasheet")) (field (name "LCSC") "C25744"))
                  (property (name "Sheetfile") (value "test.kicad_sch"))
                  (sheetpath (names "/") (tstamps "/")) (tstamps "1111"))
                (comp (ref "R2") (value "1k") (footprint "Missing:R_Nowhere")
                  (property (name "dnp")) (sheetpath (names "/") (tstamps "/")) (tstamps "2222"))
                (comp (ref "R3") (value "1k") (footprint "other:R_Test")
                  (property (name "dnp")) (sheetpath (names "/sub/") (tstamps "/abcd/")) (tstamps "3333")))
              (nets
                (net (code "1") (name "/VIN") (node (ref "R1") (pin "1") (pintype "passive")))
                (net (code "2") (name "GND") (node (ref "R1") (pin "2") (pintype "passive")) (node (ref "R3") (pin "2") (pintype "passive")))))"#,
        )
        .unwrap();
        let output = directory.join("out");
        let report = import_kicad_netlist(&netlist, &output, "test", 4).unwrap();
        assert_eq!(report.footprints, 2);
        assert_eq!(report.nets, 2);
        assert_eq!(report.missing, vec!["R2 (Missing:R_Nowhere not found)".to_string()]);
        assert_eq!(report.substituted, vec!["R3: other:R_Test -> parts:R_Test".to_string()]);

        let board = parse(&fs::read_to_string(&report.board).unwrap()).unwrap();
        let layers = board.child("layers").unwrap().children();
        assert!(layers.iter().any(|layer| layer.children().get(1).and_then(Expr::atom) == Some("In2.Cu")));
        let footprints: Vec<&Expr> = board.children().iter().filter(|item| item.head() == Some("footprint")).collect();
        let property = |footprint: &Expr, name: &str| {
            footprint
                .children()
                .iter()
                .find(|item| item.head() == Some("property") && item.children().get(1).and_then(Expr::atom) == Some(name))
                .and_then(|item| item.children().get(2).and_then(Expr::atom))
                .map(str::to_owned)
        };
        let r1 = footprints[0];
        assert_eq!(r1.children()[1].atom(), Some("parts:R_Test"));
        assert_eq!(property(r1, "Reference").as_deref(), Some("R1"));
        assert_eq!(property(r1, "Value").as_deref(), Some("10k"));
        assert_eq!(property(r1, "LCSC").as_deref(), Some("C25744"));
        assert_eq!(form_atom(r1, "path", 1), Some("/1111"));
        assert_eq!(form_atom(r1, "sheetfile", 1), Some("test.kicad_sch"));
        assert!(r1.child("version").is_none() && r1.child("at").is_some() && r1.child("uuid").is_some());
        let nets: Vec<Option<&str>> = r1.children().iter().filter(|item| item.head() == Some("pad")).map(|pad| form_atom(pad, "net", 1)).collect();
        assert_eq!(nets, vec![Some("/VIN"), Some("GND")]);
        let r3 = footprints[1];
        assert_eq!(form_atom(r3, "path", 1), Some("/abcd/3333"));
        assert_eq!(form_atom(r3, "sheetname", 1), Some("/sub/"));
        assert!(r3.child("attr").unwrap().children().iter().any(|flag| flag.atom() == Some("dnp")));
        let pads: Vec<Option<&str>> = r3.children().iter().filter(|item| item.head() == Some("pad")).map(|pad| form_atom(pad, "net", 1)).collect();
        assert_eq!(pads, vec![None, Some("GND")]);
        fs::remove_dir_all(&directory).unwrap();
    }
}
