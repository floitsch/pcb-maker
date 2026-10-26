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
    /// another library, under KiCad 9's name, under KiCad 10's (a vendor
    /// prefix: `Texas_DSG0008A_WSON-8-...`) or from an add-on library under
    /// its PCM nickname: `C1: <asked> -> <used> (<why>)`.
    /// The board and the copied schematic use the new name.
    pub substituted: Vec<String>,
    /// Footprints taken from KiCad's add-on libraries (its Plugin and
    /// Content Manager's 3rdparty folder): `U10: Espressif:ESP32-S3-MINI-1
    /// (<folder>)`. Used, not copied into the project.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub add_on_libraries: Vec<String>,
    /// Symbol pins on a net that the footprint has no pad for (`J1: M1
    /// (GND)`): the footprint does not match the symbol. A pin on no net
    /// (KiCad's `unconnected-(...)`) loses no connection, but KiCad's
    /// schematic parity check still reports it (`net_conflict`).
    pub unmatched_pins: Vec<String>,
    /// Footprints whose own pads are closer than the board's clearance (a
    /// solder jumper's) and got their pads' gap as their own clearance.
    pub clearances: Vec<String>,
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
    /// Parts `constraints.json`'s `add` put on the board.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
    /// The key matrix found in the netlist and the starter constraints
    /// written for it (`constraints.starter.json`); absent when no part
    /// looks like a key switch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starter: Option<crate::key_matrix::KiCadStarterReport>,
    /// The starter constraints and layout config, when written (never over
    /// existing files).
    pub constraints: Option<PathBuf>,
    pub layout: Option<PathBuf>,
}

/// Connectors people plug something into from outside the device, by
/// footprint name: designers put 90 % of USB and RF connectors and all
/// audio jacks and card edges of 616 open-source boards at an edge.
pub(crate) fn edge_connector(footprint_name: &str) -> bool {
    let name = footprint_name.to_ascii_lowercase();
    let name = name.rsplit(':').next().unwrap_or(&name);
    if name.contains("u.fl") {
        return false;
    }
    // Short names as whole words (`USB_C`, `USB2`, `SMA_Amphenol`, not
    // `Small`), longer ones anywhere (`BarrelJack`, `AudioJack`).
    let words = name.split(|c: char| !c.is_ascii_alphanumeric());
    let short = ["usb", "sma", "bnc", "rj45", "rj11", "rj12", "hdmi"];
    let word = |word: &str| {
        short.iter().any(|key| {
            word.strip_prefix(key).is_some_and(|rest| {
                rest.is_empty() || rest.starts_with(|c: char| c.is_ascii_digit())
            })
        })
    };
    words.into_iter().any(word)
        || [
            "coax",
            "displayport",
            "dsub",
            "d-sub",
            "barrel",
            "jack",
            "cardedge",
            "card_edge",
            "pcie",
            "pciexpress",
            "pci_express",
            "terminalblock",
            "terminal_block",
        ]
        .iter()
        .any(|key| name.contains(key))
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
        second = (second ^ byte as u64)
            .wrapping_mul(0x100_0000_01b3)
            .rotate_left(5);
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

/// Expands `${VAR}` and `$(VAR)`: the project directory, KiCad's footprint
/// directory under any version's name, else the environment.
fn expand(uri: &str, project: &Path, system: &str) -> String {
    let mut path = String::new();
    let mut rest = uri;
    while let Some(start) = rest.find('$') {
        path.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let close = match after.chars().next() {
            Some('{') => '}',
            Some('(') => ')',
            _ => {
                path.push('$');
                rest = after;
                continue;
            }
        };
        let Some(end) = after.find(close) else {
            path.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let variable = &after[1..end];
        if variable == "KIPRJMOD" {
            path.push_str(&project.display().to_string());
        } else if variable.starts_with("KICAD") && variable.ends_with("_FOOTPRINT_DIR") {
            path.push_str(system);
        } else if variable.starts_with("KICAD")
            && variable.ends_with("_3RD_PARTY")
            && std::env::var(variable).is_err()
            && let Some(directory) = third_party_directories(&|name: &str| {
                std::env::var(name).ok().filter(|value| !value.is_empty())
            })
            .into_iter()
            .find(|directory| directory.is_dir())
        {
            path.push_str(&directory.display().to_string());
        } else if let Ok(value) = std::env::var(variable) {
            path.push_str(&value);
        } else {
            path.push_str(&rest[start..start + 2 + end]);
        }
        rest = &after[end + 1..];
    }
    path.push_str(rest);
    path
}

/// Where KiCad's own footprint libraries may be, in order: the
/// `KICAD10_FOOTPRINT_DIR` family (KiCad's own variables, set by its
/// installers and honoured on every platform: 10, 9, 8), then the
/// standard places on Linux, macOS and Windows. The first that exists is
/// taken.
fn stock_footprint_directories(variable: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let mut directories: Vec<PathBuf> = [
        "KICAD10_FOOTPRINT_DIR",
        "KICAD9_FOOTPRINT_DIR",
        "KICAD8_FOOTPRINT_DIR",
    ]
    .iter()
    .filter_map(|name| variable(name).map(PathBuf::from))
    .collect();
    directories.extend(
        [
            "/usr/share/kicad/footprints",
            "/usr/local/share/kicad/footprints",
            "/Applications/KiCad/KiCad.app/Contents/SharedSupport/footprints",
        ]
        .iter()
        .map(PathBuf::from),
    );
    if let Some(programs) = variable("ProgramFiles") {
        for version in ["10.0", "9.0"] {
            directories.push(
                Path::new(&programs)
                    .join("KiCad")
                    .join(version)
                    .join("share/kicad/footprints"),
            );
        }
    }
    directories
}

/// Where KiCad's Plugin and Content Manager installs add-on libraries
/// (`<3rdparty>/footprints/<package>/<library>.pretty`), in order:
/// `KICAD10_3RD_PARTY` (KiCad's own variable, 9's after it), then the
/// default places on Linux (`~/.local/share/kicad/<version>/3rdparty`),
/// macOS and Windows (`Documents/KiCad/<version>/3rdparty`).
fn third_party_directories(variable: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let mut directories: Vec<PathBuf> = ["KICAD10_3RD_PARTY", "KICAD9_3RD_PARTY"]
        .iter()
        .filter_map(|name| variable(name).map(PathBuf::from))
        .collect();
    for version in ["10.0", "9.0"] {
        if let Some(data) = variable("XDG_DATA_HOME") {
            directories.push(
                Path::new(&data)
                    .join("kicad")
                    .join(version)
                    .join("3rdparty"),
            );
        }
        if let Some(home) = variable("HOME") {
            directories.push(
                Path::new(&home)
                    .join(".local/share/kicad")
                    .join(version)
                    .join("3rdparty"),
            );
            directories.push(
                Path::new(&home)
                    .join("Documents/KiCad")
                    .join(version)
                    .join("3rdparty"),
            );
        }
        if let Some(profile) = variable("USERPROFILE") {
            directories.push(
                Path::new(&profile)
                    .join("Documents\\KiCad")
                    .join(version)
                    .join("3rdparty"),
            );
        }
    }
    directories
}

/// Add-on libraries a schematic may name that KiCad does not ship, with
/// where to get them (docs/agents.md, "From a schematic").
const ADD_ON_LIBRARIES: [(&str, &str); 1] = [(
    "Espressif",
    "Espressif's library: KiCad's Plugin and Content Manager, \"Espressif KiCad Library\" (installed as PCM_Espressif), or github.com/espressif/kicad-libraries",
)];

/// The user's global footprint tables, KiCad 10's before 9's: under
/// `KICAD_CONFIG_HOME`, else `$XDG_CONFIG_HOME/kicad` or `~/.config/kicad`
/// (Linux), `~/Library/Preferences/kicad` (macOS), `%APPDATA%\kicad`
/// (Windows). Missing ones are skipped when read.
fn user_footprint_tables(variable: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let mut homes: Vec<PathBuf> = Vec::new();
    if let Some(home) = variable("KICAD_CONFIG_HOME") {
        homes.push(PathBuf::from(home));
    }
    if let Some(base) = variable("XDG_CONFIG_HOME") {
        homes.push(Path::new(&base).join("kicad"));
    }
    if let Some(home) = variable("HOME") {
        homes.push(Path::new(&home).join(".config/kicad"));
        homes.push(Path::new(&home).join("Library/Preferences/kicad"));
    }
    if let Some(data) = variable("APPDATA") {
        homes.push(Path::new(&data).join("kicad"));
    }
    let mut tables = Vec::new();
    for version in ["10.0", "9.0"] {
        for home in &homes {
            let table = home.join(version).join("fp-lib-table");
            if !tables.contains(&table) {
                tables.push(table);
            }
        }
    }
    tables
}

/// Footprint libraries by name: from fp-lib-tables (the project's first,
/// then the user's), then `.pretty` folders in the project, then KiCad's
/// own libraries. A table entry whose folder does not exist here falls back
/// to the others.
fn footprint_libraries(project: &Path) -> BTreeMap<String, PathBuf> {
    let variable = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let candidates = stock_footprint_directories(&variable);
    let system = candidates
        .iter()
        .find(|directory| directory.is_dir())
        .or(candidates.first())
        .map(|directory| directory.display().to_string())
        .unwrap_or_default();
    let mut tables = vec![project.join("fp-lib-table")];
    tables.extend(user_footprint_tables(&variable));
    let mut libraries = BTreeMap::new();
    for table in tables {
        let Ok(text) = fs::read_to_string(&table) else {
            continue;
        };
        let Ok(parsed) = parse(&text) else {
            continue;
        };
        for lib in parsed
            .children()
            .iter()
            .filter(|item| item.head() == Some("lib"))
        {
            let (Some(name), Some(uri)) = (form_atom(lib, "name", 1), form_atom(lib, "uri", 1))
            else {
                continue;
            };
            let path = PathBuf::from(expand(uri, project, &system));
            if path.is_dir() {
                libraries.entry(name.to_string()).or_insert(path);
            }
        }
    }
    // Libraries shipped in the project without a table entry.
    let mut folders = vec![(project.to_path_buf(), 0)];
    while let Some((folder, depth)) = folders.pop() {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            match path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".pretty"))
            {
                Some(name) => {
                    libraries.entry(name.to_string()).or_insert(path);
                }
                None if depth < 3 => folders.push((path, depth + 1)),
                None => {}
            }
        }
    }
    // KiCad's own libraries, even without a table.
    if let Ok(entries) = fs::read_dir(&system) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".pretty"))
            {
                libraries.entry(name.to_string()).or_insert(path);
            }
        }
    }
    // KiCad's add-on libraries (its Plugin and Content Manager's), without
    // a table entry too: under their folder's name and PCM's nickname.
    for directory in third_party_directories(&variable) {
        let Ok(packages) = fs::read_dir(directory.join("footprints")) else {
            continue;
        };
        let mut found: Vec<PathBuf> = packages
            .flatten()
            .filter_map(|package| fs::read_dir(package.path()).ok())
            .flat_map(|entries| entries.flatten().map(|entry| entry.path()))
            .filter(|path| path.is_dir())
            .collect();
        found.sort();
        for path in found {
            if let Some(name) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".pretty"))
            {
                let plain = name.strip_prefix("PCM_").unwrap_or(name);
                libraries.entry(plain.to_string()).or_insert(path.clone());
                libraries.entry(format!("PCM_{plain}")).or_insert(path);
            }
        }
    }
    libraries
}

/// Whether a library folder is one of KiCad's add-on libraries (under a
/// `third_party_directories` folder).
fn add_on_folder(path: &Path) -> bool {
    let variable = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    third_party_directories(&variable)
        .iter()
        .any(|directory| path.starts_with(directory.join("footprints")))
}

/// A footprint's file: in its library, else the same name in any library
/// (projects often carry copies), also under KiCad 9's names for
/// connectors (Female and Male became Socket and Plug); else in KiCad's
/// add-on library of PCM's nickname (`PCM_Espressif` for `Espressif`, and
/// the other way round); else, in its library, the one footprint whose
/// name ends in `_<name>` (KiCad 10 put a vendor prefix on some:
/// `WSON-8-1EP_2x2mm_P0.5mm_EP0.9x1.6mm` is now
/// `Texas_DSG0008A_WSON-8-1EP_2x2mm_P0.5mm_EP0.9x1.6mm`). The id found
/// names the library and footprint used.
fn find_footprint(libraries: &BTreeMap<String, PathBuf>, id: &str) -> Option<(PathBuf, String)> {
    let (library, name) = id.split_once(':')?;
    // A name that is no file name (`lib:../../../tmp/evil`) reads no file
    // outside the libraries: not found.
    if file_name_error(name).is_some() {
        return None;
    }
    let mut names = vec![name.to_string()];
    let renamed = name.replace("_Female", "_Socket").replace("_Male", "_Plug");
    if renamed != name {
        names.push(renamed);
    }
    let file_in = |library: &str, name: &str| {
        let file = libraries.get(library)?.join(format!("{name}.kicad_mod"));
        file.exists().then(|| (file, format!("{library}:{name}")))
    };
    let nickname = match library.strip_prefix("PCM_") {
        Some(plain) => plain.to_string(),
        None => format!("PCM_{library}"),
    };
    names
        .iter()
        .find_map(|name| {
            file_in(library, name)
                .or_else(|| libraries.keys().find_map(|other| file_in(other, name)))
        })
        .or_else(|| names.iter().find_map(|name| file_in(&nickname, name)))
        .or_else(|| {
            let folder = libraries.get(library)?;
            let suffix = format!("_{name}.kicad_mod");
            let mut matches = fs::read_dir(folder)
                .ok()?
                .flatten()
                .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
                .filter(|file| file.ends_with(&suffix));
            let only = matches.next()?;
            if matches.next().is_some() {
                return None;
            }
            let found = only.strip_suffix(".kicad_mod")?.to_string();
            Some((folder.join(&only), format!("{library}:{found}")))
        })
}

/// Why a footprint was found under another id, for the import's
/// `substituted` (empty when it says nothing more than the ids).
fn substitution_reason(asked: &str, found: &str) -> &'static str {
    let (Some((asked_library, asked_name)), Some((found_library, found_name))) =
        (asked.split_once(':'), found.split_once(':'))
    else {
        return "";
    };
    if asked_name == found_name
        && (found_library.strip_prefix("PCM_") == Some(asked_library)
            || asked_library.strip_prefix("PCM_") == Some(found_library))
    {
        " (KiCad's add-on library under its Plugin and Content Manager nickname)"
    } else if found_name.ends_with(&format!("_{asked_name}")) {
        " (KiCad 10's name: a vendor prefix)"
    } else {
        ""
    }
}

/// Turns a KiCad 6 library footprint's `(fp_text reference ...)` and
/// `(fp_text value ...)` into the `Reference` and `Value` properties KiCad 8
/// and later write, keeping their place, layer and font. Left as they were,
/// KiCad reads the library's text (`Bat+0`) as the reference over the
/// property the import sets: every copy of the footprint got one reference,
/// and the schematic parity check found 73 duplicate and 73 missing
/// footprints on urchin.
fn legacy_texts_to_properties(items: &mut [Expr], reference: &str) -> Result<(), String> {
    for item in items.iter_mut() {
        let Expr::List(parts) = item else { continue };
        let name = match (
            parts.first().and_then(Expr::atom),
            parts.get(1).and_then(Expr::atom),
        ) {
            (Some("fp_text"), Some("reference")) => "Reference",
            (Some("fp_text"), Some("value")) => "Value",
            _ => continue,
        };
        let mut converted = vec![Expr::Atom("property".into()), Expr::Atom(requote(name))];
        converted.extend(parts.get(2).cloned());
        let mut hidden = false;
        let mut unlocked = false;
        for part in parts.iter().skip(3) {
            match part {
                Expr::Atom(atom) if atom == "hide" => hidden = true,
                Expr::List(at) if part.head() == Some("at") => {
                    // `(at x y [angle] [unlocked])`: the angle is required.
                    let mut numbers: Vec<Expr> = at
                        .iter()
                        .skip(1)
                        .filter(|value| value.atom() != Some("unlocked"))
                        .cloned()
                        .collect();
                    unlocked |= at.iter().any(|value| value.atom() == Some("unlocked"));
                    while numbers.len() < 3 {
                        numbers.push(Expr::Atom("0".into()));
                    }
                    converted.push(Expr::List(
                        [vec![Expr::Atom("at".into())], numbers].concat(),
                    ));
                }
                Expr::List(_) if part.head() == Some("tstamp") => {}
                other => converted.push(other.clone()),
            }
        }
        if unlocked {
            converted.push(parse("(unlocked yes)")?);
        }
        if hidden {
            converted.push(parse("(hide yes)")?);
        }
        converted.push(parse(&format!(
            "(uuid \"{}\")",
            uuid_of(&format!("property {reference} {name}"))
        ))?);
        *item = Expr::List(converted);
    }
    Ok(())
}

/// Turns KiCad 5 arcs (`start` the centre, `end` where the arc starts,
/// `angle` in degrees), which KiCad 6 library files still carry, into the
/// three points KiCad 7 and later write (start, mid, end on the arc). KiCad
/// reads the old form only in a file of that age: a board of today with one
/// does not load (osprey's Tag-Connect footprint: kicad-cli exit 3, the
/// import failed in its pad check).
fn legacy_arcs_to_three_points(items: &mut [Expr]) -> Result<(), String> {
    for item in items.iter_mut() {
        if item.head() != Some("fp_arc") || item.child("mid").is_some() {
            continue;
        }
        let Some(angle) = form_atom(item, "angle", 1).and_then(|angle| angle.parse::<f64>().ok())
        else {
            continue;
        };
        let (Ok(cx), Ok(cy), Ok(sx), Ok(sy)) = (
            form_f64(item, "start", 1),
            form_f64(item, "start", 2),
            form_f64(item, "end", 1),
            form_f64(item, "end", 2),
        ) else {
            continue;
        };
        let radius = (sx - cx).hypot(sy - cy);
        let first = (sy - cy).atan2(sx - cx);
        // KiCad's angle turns from the start towards +y (down) for a
        // positive value: the same sense as atan2 in board coordinates.
        let point = |name: &str, turn: f64| {
            let at = first + turn.to_radians();
            parse(&format!(
                "({name} {} {})",
                round_mm(cx + radius * at.cos()),
                round_mm(cy + radius * at.sin())
            ))
        };
        let replacement = [
            parse(&format!("(start {sx} {sy})"))?,
            point("mid", angle / 2.0)?,
            point("end", angle)?,
        ];
        let Expr::List(parts) = item else { continue };
        parts.retain(|part| !matches!(part.head(), Some("start" | "end" | "angle")));
        parts.splice(1..1, replacement);
    }
    Ok(())
}

fn round_mm(value: f64) -> f64 {
    let rounded = (value * 1e6).round() / 1e6;
    if rounded == 0.0 { 0.0 } else { rounded }
}

/// Sets a footprint property's value, adding it (hidden, on the fab layer,
/// as KiCad does) when the footprint has none of that name.
fn set_property(
    items: &mut Vec<Expr>,
    name: &str,
    value: &str,
    reference: &str,
) -> Result<(), String> {
    for item in items
        .iter_mut()
        .filter(|item| item.head() == Some("property"))
    {
        if let Expr::List(parts) = item
            && parts.get(1).and_then(Expr::atom) == Some(name)
            && parts.len() > 2
        {
            parts[2] = Expr::Atom(requote(value));
            return Ok(());
        }
    }
    let last = items
        .iter()
        .rposition(|item| item.head() == Some("property"))
        .map_or(items.len(), |index| index + 1);
    items.insert(
        last,
        parse(&format!(
            "(property {} {} (at 0 0 0) (unlocked yes) (layer \"F.Fab\") (hide yes) (uuid \"{}\") (effects (font (size 1.27 1.27) (thickness 0.15))))",
            requote(name),
            requote(value),
            uuid_of(&format!("property {reference} {name}"))
        ))?,
    );
    Ok(())
}

/// Gives every UUID inside a footprint instance its own value (library
/// files carry one set for all instances; KiCad makes new ones when it
/// adds a footprint). KiCad 6 and 7 files call them `tstamp`.
fn renew_uuids(item: &mut Expr, owner: &str) {
    let mut pending = vec![item];
    while let Some(item) = pending.pop() {
        let Expr::List(items) = item else {
            continue;
        };
        if matches!(items.first().and_then(Expr::atom), Some("uuid" | "tstamp"))
            && let Some(old) = items.get(1).and_then(Expr::atom).map(str::to_owned)
        {
            items[1] = Expr::Atom(format!("\"{}\"", uuid_of(&format!("{owner} {old}"))));
            continue;
        }
        pending.extend(items.iter_mut());
    }
}

/// The ids a footprint taken from the `--footprints-from` board keeps: the
/// board footprint's own uuid, the first time that footprint is used (a
/// footprint used again, as a template for another part, gets new ones).
/// Its pads and drawings keep theirs too, as KiCad's "Update PCB from
/// Schematic" keeps a footprint's: the project's DRC exclusions name
/// items by these ids (`courtyards_overlap|x|y|<footprint>|<footprint>`,
/// `copper_edge_clearance|x|y|<edge>|<pad>`), and with new ids the
/// designer's excluded findings come back as errors (MokyaLora's
/// R34/R35, urchin's key sockets over the edge).
fn kept_uuid(
    theirs: &crate::board_footprints::BoardFootprint,
    kept: &mut BTreeSet<String>,
) -> Option<String> {
    let uuid = theirs.uuid.as_deref()?.trim_matches('"').to_string();
    (!uuid.is_empty() && kept.insert(uuid.clone())).then_some(uuid)
}

/// A string from the netlist, still escaped as KiCad writes strings.
fn text_of(node: &Expr, head: &str) -> Option<String> {
    form_atom(node, head, 1).map(str::to_owned)
}

/// A string `text_of` read, as an atom again.
fn requote(escaped: &str) -> String {
    format!("\"{escaped}\"")
}

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
    check_board_id(board_id)?;
    let designer = footprints_from
        .map(crate::board_footprints::BoardFootprints::read)
        .transpose()?;
    let project = input.parent().unwrap_or(Path::new(".")).to_path_buf();
    fs::create_dir_all(output)
        .map_err(|error| format!("failed to create {}: {error}", output.display()))?;
    let is_schematic = input
        .extension()
        .is_some_and(|extension| extension == "kicad_sch");
    let netlist_path = if is_schematic {
        let path = output.join(format!("{board_id}.net"));
        let result = Command::new(crate::native_report::kicad_cli())
            .args(["sch", "export", "netlist", "--format", "kicadsexpr", "-o"])
            .arg(&path)
            .arg(input)
            .output()
            .map_err(|error| {
                crate::verdict::input_error(format!(
                    "kicad-cli did not run ({error}): {}",
                    crate::KICAD_REQUIREMENT
                ))
            })?;
        // The schematic KiCad cannot read is the input's fault (exit 2).
        if !result.status.success() {
            return Err(crate::verdict::input_error(format!(
                "kicad-cli sch export netlist failed on {}: {}",
                input.display(),
                String::from_utf8_lossy(&result.stderr)
            )));
        }
        // The project travels with the board: schematic sheets (the root
        // renamed after the board), project settings, library tables.
        let stem = input
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("");
        for entry in fs::read_dir(&project)
            .map_err(|error| error.to_string())?
            .flatten()
        {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("")
                .to_string();
            let target = if name == format!("{stem}.kicad_sch") {
                format!("{board_id}.kicad_sch")
            } else if name == format!("{stem}.kicad_pro") {
                format!("{board_id}.kicad_pro")
            } else if name.ends_with(".kicad_sch")
                || name == "sym-lib-table"
                || name == "fp-lib-table"
            {
                name
            } else {
                continue;
            };
            fs::copy(&path, output.join(target)).map_err(|error| error.to_string())?;
        }
        // Sheets in subfolders (`sch/power.kicad_sch`), where the root
        // expects them.
        let skip = fs::canonicalize(output).ok();
        let mut folders: Vec<(PathBuf, usize)> = fs::read_dir(&project)
            .map_err(|error| error.to_string())?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .map(|path| (path, 1))
            .collect();
        while let Some((folder, depth)) = folders.pop() {
            if fs::canonicalize(&folder).ok() == skip {
                continue;
            }
            let Ok(entries) = fs::read_dir(&folder) else {
                continue;
            };
            for path in entries.flatten().map(|entry| entry.path()) {
                if path.is_dir() {
                    if depth < 3 {
                        folders.push((path, depth + 1));
                    }
                } else if path
                    .extension()
                    .is_some_and(|extension| extension == "kicad_sch")
                    && let Ok(relative) = path.strip_prefix(&project)
                {
                    copy_library_tree(&path, &output.join(relative), &project, output)?;
                }
            }
        }
        // And the project's own libraries its tables name.
        for table in ["fp-lib-table", "sym-lib-table"] {
            let Ok(parsed) = fs::read_to_string(project.join(table))
                .map_err(|error| error.to_string())
                .and_then(|text| parse(&text))
            else {
                continue;
            };
            for lib in parsed
                .children()
                .iter()
                .filter(|item| item.head() == Some("lib"))
            {
                let uri = form_atom(lib, "uri", 1).unwrap_or("");
                let Some(relative) = uri
                    .strip_prefix("${KIPRJMOD}/")
                    .or_else(|| uri.strip_prefix("$(KIPRJMOD)/"))
                else {
                    continue;
                };
                // Only libraries inside the project: `${KIPRJMOD}/../lib`
                // would be copied beside the output directory, not into it
                // (glasgow's footprints two folders up). The board does not
                // need them: its footprints are in it.
                if Path::new(relative).components().any(|part| {
                    !matches!(
                        part,
                        std::path::Component::Normal(_) | std::path::Component::CurDir
                    )
                }) {
                    continue;
                }
                copy_library_tree(
                    &project.join(relative),
                    &output.join(relative),
                    &project,
                    output,
                )?;
            }
        }
        path
    } else {
        input.to_path_buf()
    };
    let not_a_netlist = || {
        crate::verdict::input_error(format!(
            "{} is not a KiCad netlist: give the schematic, or export one with kicad-cli sch export netlist --format kicadsexpr",
            netlist_path.display()
        ))
    };
    let netlist = fs::read_to_string(&netlist_path)
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
    let mut libraries = footprint_libraries(&project);
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

    // Nets by (reference, pin): name, pin function and type.
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

    let mut board = parse(
        &HEADER.replace(
            "{inner}",
            &(1..layers.saturating_sub(1))
                .map(|n| format!("({} \"In{n}.Cu\" signal)", 2 + 2 * n))
                .collect::<Vec<_>>()
                .join(" "),
        ),
    )?;
    let mut missing = Vec::new();
    let mut substituted = Vec::new();
    let mut add_on = Vec::new();
    let mut unmatched_pins = Vec::new();
    // Footprint ids the libraries know under another name, old to new.
    // (Old, new): how many symbols; the copied schematic names a footprint
    // id once for all its symbols, so the most common new name wins.
    let mut renamed: BTreeMap<(String, String), usize> = BTreeMap::new();
    // With `--footprints-from`: footprints taken from the board for want of
    // a library (to write into the project's libraries, by id), the board's
    // versions used, the drifted ones (id: sizes, references).
    let mut local_footprints: BTreeMap<String, Expr> = BTreeMap::new();
    let mut from_board = Vec::new();
    let mut board_versions = 0;
    let mut drift: BTreeMap<(String, String), (String, Vec<String>)> = BTreeMap::new();
    let mut footprints = 0;
    // The board footprints whose ids the import keeps (`kept_uuid`).
    let mut kept_ids: BTreeSet<String> = BTreeSet::new();
    let components = netlist
        .child("components")
        .map(Expr::children)
        .unwrap_or_default();
    // The parts as the key matrix inference sees them.
    let mut matrix_parts = Vec::new();
    // Every symbol's path, so that a board footprint linked to one symbol is
    // not taken for another of its reference.
    let symbol_paths: BTreeSet<String> = components
        .iter()
        .filter(|item| item.head() == Some("comp"))
        .filter_map(|component| {
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
        })
        .collect();
    for component in components.iter().filter(|item| item.head() == Some("comp")) {
        let reference = text_of(component, "ref").unwrap_or_default();
        let value = text_of(component, "value").unwrap_or_default();
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
        matrix_parts.push(crate::key_matrix::MatrixPart {
            reference: reference.clone(),
            footprint: text_of(component, "footprint").unwrap_or_default(),
            symbol: component
                .child("libsource")
                .and_then(|source| text_of(source, "part"))
                .unwrap_or_default(),
            value: value.clone(),
            pins: pins
                .range((reference.clone(), String::new())..)
                .take_while(|((owner, _), _)| *owner == reference)
                .map(|((_, pin), (net, _, kind))| (pin.clone(), net.clone(), kind.clone()))
                .collect(),
        });
        // The symbol the footprint is linked to: `/<sheet uuids>/<symbol uuid>`.
        let sheet_path = component.child("sheetpath");
        let sheet = sheet_path
            .and_then(|sheet| text_of(sheet, "tstamps"))
            .unwrap_or_else(|| "/".into());
        let symbol_path = text_of(component, "tstamps")
            .or_else(|| text_of(component, "tstamp"))
            .map(|stamp| {
                format!(
                    "{}{stamp}",
                    if sheet.ends_with('/') {
                        sheet.clone()
                    } else {
                        format!("{sheet}/")
                    }
                )
            });
        // A symbol without a footprint takes the board's footprint of it,
        // where a board is given and has it.
        let named = text_of(component, "footprint").filter(|id| id.contains(':'));
        let unnamed = named.is_none();
        let Some(footprint_id) = named.or_else(|| {
            let own = designer
                .as_ref()?
                .own(symbol_path.as_deref(), &reference, &symbol_paths)?;
            Some(if own.id.contains(':') {
                own.id.clone()
            } else {
                format!("{FROM_BOARD_LIBRARY}:{}", own.id)
            })
        }) else {
            missing.push(format!("{reference} (no footprint)"));
            continue;
        };
        let library = find_footprint(&libraries, &footprint_id);
        let theirs = designer.as_ref().and_then(|designer| {
            designer.for_symbol(
                symbol_path.as_deref(),
                &reference,
                &footprint_id,
                library.is_none(),
                &symbol_paths,
            )
        });
        let mut kept = None;
        let (mut footprint, found) = match (library, theirs) {
            (library, Some((theirs, how))) => {
                // A library footprint under another id (KiCad 10's name, an
                // add-on's nickname) is no library version of this one: the
                // board's goes into the project's libraries.
                let library = library.filter(|(_, found)| *found == footprint_id);
                // The board's footprint, under the board's name where it
                // differs from the schematic's (the board was changed
                // after the schematic, or the other way round).
                let same_name = theirs.name() == crate::board_footprints::name_of(&footprint_id);
                let wanted = if same_name {
                    footprint_id.clone()
                } else {
                    theirs.id.clone()
                };
                let (wanted_library, wanted_name) = wanted.split_once(':').unwrap_or(("", &wanted));
                let form = crate::board_footprints::library_form(&theirs.expr, wanted_name)?;
                if let Some((file, _)) = &library {
                    // How far the library's version, which the import would
                    // have used, is from the board's.
                    board_versions += 1;
                    let ours = fs::read_to_string(file)
                        .ok()
                        .and_then(|text| parse(&text).ok());
                    if let (Some(a), Some(b)) = (
                        ours.as_ref().and_then(crate::board_footprints::body_size),
                        crate::board_footprints::body_size(&form),
                    ) && crate::board_footprints::drifted(a, b)
                    {
                        let entry = drift.entry((footprint_id.clone(), theirs.id.clone())).or_insert_with(|| {
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
                let resolves =
                    find_footprint(&libraries, &wanted).is_some_and(|(_, found)| found == wanted);
                let id = if (library.is_some() && same_name) || (!same_name && resolves) {
                    wanted.clone()
                } else {
                    // Into a library of the project: under the footprint's
                    // library name where no library of that name exists
                    // here, else `from_board`.
                    let id = if libraries.contains_key(wanted_library) || wanted_library.is_empty()
                    {
                        format!("{FROM_BOARD_LIBRARY}:{wanted_name}")
                    } else {
                        wanted.clone()
                    };
                    local_footprints
                        .entry(id.clone())
                        .or_insert_with(|| form.clone());
                    id
                };
                if id != footprint_id {
                    *renamed
                        .entry((footprint_id.clone(), id.clone()))
                        .or_default() += 1;
                }
                let whose = match how {
                    crate::board_footprints::Found::Own if same_name => {
                        format!("the board's {}", theirs.reference)
                    }
                    crate::board_footprints::Found::Own => format!(
                        "the board's {}, which has another footprint than the schematic's",
                        theirs.reference
                    ),
                    crate::board_footprints::Found::ByName => {
                        format!("the board's {}, of that footprint name", theirs.reference)
                    }
                };
                if unnamed {
                    from_board.push(format!(
                        "{reference}: {id} ({whose}; the schematic names no footprint)"
                    ));
                } else if library.is_none() {
                    from_board.push(format!("{reference}: {id} ({whose})"));
                } else if id != footprint_id {
                    substituted.push(format!("{reference}: {footprint_id} -> {id} ({whose})"));
                }
                if how == crate::board_footprints::Found::Own {
                    kept = kept_uuid(theirs, &mut kept_ids);
                }
                (form, id)
            }
            (Some((file, found)), None) => {
                if file.parent().is_some_and(add_on_folder) {
                    add_on.push(format!(
                        "{reference}: {found} ({})",
                        file.parent()
                            .map_or(String::new(), |folder| folder.display().to_string())
                    ));
                }
                if found != footprint_id {
                    substituted.push(format!(
                        "{reference}: {footprint_id} -> {found}{}",
                        substitution_reason(&footprint_id, &found)
                    ));
                    *renamed
                        .entry((footprint_id.clone(), found.clone()))
                        .or_default() += 1;
                }
                match fs::read_to_string(&file)
                    .map_err(|error| error.to_string())
                    .and_then(|text| parse(&text))
                    .and_then(|footprint| match footprint_form_error(&footprint) {
                        Some(why) => Err(format!("the file holds {why}")),
                        None => Ok(footprint),
                    }) {
                    Ok(footprint) => (footprint, found),
                    Err(error) => {
                        missing.push(format!("{reference} ({found} unreadable: {error})"));
                        continue;
                    }
                }
            }
            (None, None) => {
                // Say where it was looked for, so that the fix is plain.
                let (library, name) = footprint_id.split_once(':').unwrap_or(("", &footprint_id));
                let mut line = match libraries.get(library) {
                    Some(folder) => format!(
                        "{reference} ({footprint_id} not found: {} has no {name}.kicad_mod",
                        folder.display()
                    ),
                    None => format!(
                        "{reference} ({footprint_id} not found: no library {library:?} in the fp-lib-table files, the project's .pretty folders, KiCad's own or its add-on libraries{}",
                        ADD_ON_LIBRARIES
                            .iter()
                            .find(|(name, _)| *name
                                == library.strip_prefix("PCM_").unwrap_or(library))
                            .map_or(String::new(), |(_, source)| format!("; it is {source}"))
                    ),
                };
                if let Some(designer) = &designer {
                    line.push_str(&format!(
                        "; nor has {} this part or a footprint of that name",
                        designer.file.display()
                    ));
                }
                line.push(')');
                missing.push(line);
                continue;
            }
        };
        let Expr::List(items) = &mut footprint else {
            continue;
        };
        items.retain(|item| {
            !matches!(
                item.head(),
                Some("version" | "generator" | "generator_version")
            )
        });
        if kept.is_none() {
            for item in items.iter_mut() {
                renew_uuids(item, &reference);
            }
        }
        // KiCad 5 library files say `module`; KiCad renames it on loading,
        // pcb-maker reads only `footprint` (coco2's 16 footprints from its
        // KiCad 5 library were not placed or routed).
        items[0] = Expr::Atom("footprint".into());
        items[1] = Expr::Atom(requote(&found));
        // Placement fields after the layer.
        let layer_index = items
            .iter()
            .position(|item| item.head() == Some("layer"))
            .map_or(2, |index| index + 1);
        let uuid = kept.unwrap_or_else(|| uuid_of(&format!("footprint {reference}")));
        items.insert(layer_index, parse(&format!("(uuid \"{uuid}\")"))?);
        items.insert(layer_index + 1, parse("(at 0 0)")?);
        // The links to the symbol, where KiCad keeps them: before `attr`
        // and the drawings.
        let mut links = Vec::new();
        if let Some(symbol_path) = &symbol_path {
            links.push(parse(&format!("(path {})", requote(symbol_path)))?);
        }
        if let Some(names) = sheet_path.and_then(|sheet| text_of(sheet, "names")) {
            links.push(parse(&format!("(sheetname {})", requote(&names)))?);
        }
        let sheet_file = component
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
        // The symbol's fields (datasheet, description, LCSC numbers, ...).
        legacy_texts_to_properties(items, &reference)?;
        legacy_arcs_to_three_points(items)?;
        set_property(items, "Reference", &reference, &reference)?;
        set_property(items, "Value", &value, &reference)?;
        for field in component
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
                &reference,
            )?;
        }
        // The symbol says whether the part is fitted and in the BOM (a
        // mounting hole's footprint may say otherwise).
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
        // Pads sharing a number share the pin's net, except a pin nothing
        // connects: KiCad gives each such pad its own net (`_1`, `_2`).
        let mut repeats: BTreeMap<String, usize> = BTreeMap::new();
        for item in items.iter_mut() {
            if let Some("pad") = item.head() {
                let number = item
                    .children()
                    .get(1)
                    .and_then(Expr::atom)
                    .unwrap_or("")
                    .to_string();
                if let Some((net, function, kind)) = pins.get(&(reference.clone(), number.clone()))
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
        // Symbol pins the footprint has no pad for (a library that changed).
        let numbers: BTreeSet<&str> = footprint
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
            .filter_map(|pad| pad.children().get(1).and_then(Expr::atom))
            .collect();
        let lost: Vec<String> = pins
            .range((reference.clone(), String::new())..)
            .take_while(|((owner, _), _)| *owner == reference)
            .filter(|((_, pin), _)| !numbers.contains(pin.as_str()))
            .map(|((_, pin), (net, _, _))| format!("{pin} ({net})"))
            .collect();
        if !lost.is_empty() {
            unmatched_pins.push(format!("{reference}: {}", lost.join(", ")));
        }
        if let Expr::List(board_items) = &mut board {
            board_items.push(footprint);
        }
        footprints += 1;
    }
    // Parts the schematic does not have (`add` in an existing
    // constraints.json).
    let added = add_parts(
        &mut board,
        output,
        &libraries,
        designer.as_ref(),
        &mut missing,
        &mut kept_ids,
    )?;
    if !local_footprints.is_empty() {
        write_local_libraries(
            output,
            &local_footprints,
            &libraries,
            designer.as_ref().map(|designer| designer.file.as_path()),
        )?;
    }
    let board_path = output.join(format!("{board_id}.kicad_pcb"));
    fs::write(&board_path, format!("{}\n", encode(&board)))
        .map_err(|error| format!("failed to write {}: {error}", board_path.display()))?;
    // The copied schematic links the footprints the board has, so the new
    // project checks clean (the original is left as it is).
    let mut most: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for ((old, new), count) in &renamed {
        let entry = most.entry(old.clone()).or_insert((0, String::new()));
        if *count > entry.0 {
            *entry = (*count, new.clone());
        }
    }
    let renamed: BTreeMap<String, String> =
        most.into_iter().map(|(old, (_, new))| (old, new)).collect();
    if is_schematic && !renamed.is_empty() {
        let mut folders = vec![output.to_path_buf()];
        while let Some(folder) = folders.pop() {
            for path in fs::read_dir(&folder)
                .map_err(|error| error.to_string())?
                .flatten()
                .map(|entry| entry.path())
            {
                if path.is_dir() {
                    folders.push(path);
                } else if path
                    .extension()
                    .is_some_and(|extension| extension == "kicad_sch")
                {
                    let text = fs::read_to_string(&path)
                        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
                    let mut changed = text.clone();
                    for (old, new) in &renamed {
                        changed = changed.replace(
                            &format!("(property \"Footprint\" \"{old}\""),
                            &format!("(property \"Footprint\" \"{new}\""),
                        );
                        // KiCad 6 schematics repeat it per instance
                        // (`symbol_instances`), and that one wins: the
                        // parity check found 43 mismatches on 0xcb-static.
                        changed = changed.replace(
                            &format!("(footprint \"{old}\")"),
                            &format!("(footprint \"{new}\")"),
                        );
                    }
                    if changed != text {
                        fs::write(&path, changed).map_err(|error| {
                            format!("failed to write {}: {error}", path.display())
                        })?;
                    }
                }
            }
        }
    }
    // Pads of one footprint closer than the board's clearance (a solder
    // jumper's): no placement or routing changes that. The footprint gets
    // its pads' gap as its own clearance, as designers do.
    let mut clearances = Vec::new();
    let gaps = own_pad_gaps(&board, output, board_id)?;
    if !gaps.is_empty() {
        let Expr::List(items) = &mut board else {
            unreachable!("the board is a list")
        };
        for footprint in items
            .iter_mut()
            .filter(|item| item.head() == Some("footprint"))
        {
            let reference = footprint
                .children()
                .iter()
                .find(|item| {
                    item.head() == Some("property")
                        && item.children().get(1).and_then(Expr::atom) == Some("Reference")
                })
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
            let at = parts
                .iter()
                .position(|item| item.head() == Some("attr"))
                .unwrap_or(parts.len());
            parts.insert(at, parse(&format!("(clearance {clearance})"))?);
            clearances.push(format!(
                "{reference}: {clearance} mm (its pads are {gap} mm apart, the rule is {rule} mm)"
            ));
        }
        fs::write(&board_path, format!("{}\n", encode(&board)))
            .map_err(|error| format!("failed to write {}: {error}", board_path.display()))?;
    }
    // Starter constraints: every part placed, the outline sized from the
    // parts, the plug-in connectors on an edge (facing out when the
    // footprint shows which way it opens).
    let mut edges = Vec::new();
    let mut edge_connectors = Vec::new();
    for footprint in board
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let name = footprint
            .children()
            .get(1)
            .and_then(Expr::atom)
            .unwrap_or("");
        let reference = footprint
            .children()
            .iter()
            .find(|item| {
                item.head() == Some("property")
                    && item.children().get(1).and_then(Expr::atom) == Some("Reference")
            })
            .and_then(|item| item.children().get(2).and_then(Expr::atom))
            .unwrap_or("");
        let wired = footprint
            .children()
            .iter()
            .any(|item| item.head() == Some("pad") && item.child("net").is_some());
        if reference.is_empty() || !wired || !edge_connector(name) {
            continue;
        }
        let mut edge = serde_json::json!({"part": reference, "edge": "any", "flush": true});
        if crate::board_placer::connector_mouth(footprint)?.is_some() {
            edge["opening_outwards"] = true.into();
        }
        edges.push(edge);
        edge_connectors.push(format!("{reference} ({name})"));
    }
    let write_new = |name: &str, value: serde_json::Value| -> Result<Option<PathBuf>, String> {
        let path = output.join(name);
        if path.exists() {
            return Ok(None);
        }
        fs::write(
            &path,
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())? + "\n",
        )
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
        Ok(Some(path))
    };
    let mut starter = serde_json::json!({"version": 1, "move_all": true, "outline": {}});
    if !edges.is_empty() {
        starter["edge"] = edges.into();
    }
    // A key matrix: the starter constraints for it, beside the agent's
    // file (rewritten on every import; constraints.json never is).
    let starter_report = match crate::key_matrix::infer_key_matrix(&matrix_parts) {
        None => None,
        Some(matrix) => {
            let mut bodies = BTreeMap::new();
            for footprint in board
                .children()
                .iter()
                .filter(|item| item.head() == Some("footprint"))
            {
                let Some(reference) = footprint_reference(footprint) else {
                    continue;
                };
                let id = footprint
                    .children()
                    .get(1)
                    .and_then(Expr::atom)
                    .unwrap_or("")
                    .to_string();
                let size = crate::board_footprints::body_size(footprint)
                    .map_or([0.0, 0.0], |(size, _)| size);
                bodies.insert(reference, (id, size));
            }
            let (keys, mut report) =
                crate::key_matrix::starter_constraints(&matrix, &bodies, starter.clone());
            if matrix.groups.is_empty() || report.switches == 0 {
                report.why = Some(format!(
                    "{} switch-like parts, but no two share a row and a column net: no key matrix (keys wired directly to pins, or no keyboard)",
                    matrix.unassigned.len()
                ));
            } else {
                let path = output.join("constraints.starter.json");
                fs::write(&path, crate::key_matrix::constraints_text(&keys))
                    .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
                report.file = Some(path);
            }
            Some(report)
        }
    };
    let constraints = write_new("constraints.json", starter)?;
    let layout = write_new(
        "layout.json",
        serde_json::json!({"placer": {"constraints": "constraints.json"}}),
    )?;
    Ok(KiCadNetlistImport {
        board: board_path,
        footprints,
        nets,
        missing,
        substituted,
        add_on_libraries: add_on,
        unmatched_pins,
        clearances,
        edge_connectors,
        footprints_from_board: designer.is_some().then_some(from_board),
        board_versions: designer.is_some().then_some(board_versions),
        drifted_footprints: designer.is_some().then(|| {
            drift
                .into_iter()
                .map(|((id, _), (sizes, references))| {
                    let shown: Vec<&str> = references.iter().take(6).map(String::as_str).collect();
                    let more = if references.len() > shown.len() {
                        ", ..."
                    } else {
                        ""
                    };
                    format!(
                        "{id} ({} parts: {}{more}): {sizes}",
                        references.len(),
                        shown.join(", ")
                    )
                })
                .collect()
        }),
        added,
        starter: starter_report,
        constraints,
        layout,
    })
}

/// Writes the footprints taken from the board into libraries of the
/// project (`<out>/<library>.pretty`) and names those libraries in the
/// project's `fp-lib-table`, so that the project finds every footprint its
/// schematic names. An entry of that name the libraries could not resolve
/// (a designer's own folder) is replaced.
fn write_local_libraries(
    output: &Path,
    footprints: &BTreeMap<String, Expr>,
    libraries: &BTreeMap<String, PathBuf>,
    board: Option<&Path>,
) -> Result<(), String> {
    let mut nicknames = BTreeSet::new();
    for (id, footprint) in footprints {
        let Some((nickname, name)) = id.split_once(':') else {
            continue;
        };
        // The id comes from the --footprints-from board: a nickname or name
        // that is no file name (`../../x:../../y`) would write outside the
        // output directory.
        if let Some(why) = file_name_error(nickname).or_else(|| file_name_error(name)) {
            return Err(crate::verdict::input_error(format!(
                "the footprint {id:?} of {}: its library nickname or name {why}; the footprints taken from a board are written to <out>/<nickname>.pretty/<name>.kicad_mod",
                board.map_or_else(
                    || "the board".to_string(),
                    |board| board.display().to_string()
                )
            )));
        }
        nicknames.insert(nickname.to_string());
        let folder = output.join(format!("{nickname}.pretty"));
        fs::create_dir_all(&folder)
            .map_err(|error| format!("failed to create {}: {error}", folder.display()))?;
        let mut file = footprint.clone();
        if let Expr::List(items) = &mut file {
            items[1] = Expr::Atom(requote(name));
            items.retain(|item| {
                !matches!(
                    item.head(),
                    Some("version" | "generator" | "generator_version")
                )
            });
            items.splice(
                2..2,
                [
                    parse("(version 20260206)")?,
                    parse("(generator \"pcb-maker\")")?,
                ],
            );
        }
        let path = folder.join(format!("{name}.kicad_mod"));
        fs::write(&path, format!("{}\n", encode(&file)))
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    }
    let table = output.join("fp-lib-table");
    let mut parsed = fs::read_to_string(&table)
        .ok()
        .and_then(|text| parse(&text).ok())
        .filter(|parsed| parsed.head() == Some("fp_lib_table"))
        .unwrap_or_else(|| {
            Expr::List(vec![
                Expr::Atom("fp_lib_table".into()),
                Expr::List(vec![Expr::Atom("version".into()), Expr::Atom("7".into())]),
            ])
        });
    let source = board
        .and_then(|board| board.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Expr::List(items) = &mut parsed {
        for nickname in &nicknames {
            // A library of that name that exists here keeps its entry (the
            // footprints went to `from_board` then).
            if nickname != FROM_BOARD_LIBRARY && libraries.contains_key(nickname) {
                continue;
            }
            items.retain(|item| {
                !(item.head() == Some("lib")
                    && form_atom(item, "name", 1) == Some(nickname.as_str()))
            });
            items.push(parse(&format!(
                "(lib (name {}) (type \"KiCad\") (uri {}) (options \"\") (descr {}))",
                requote(nickname),
                requote(&format!("${{KIPRJMOD}}/{nickname}.pretty")),
                quote(&format!("footprints from the board {source}"))
            ))?);
        }
    }
    fs::write(&table, format!("{}\n", encode(&parsed)))
        .map_err(|error| format!("failed to write {}: {error}", table.display()))
}

/// The parts of `add` in `<out>/constraints.json`, put on the board
/// `board_only`; returns a line per part. A library footprint not found
/// goes to `missing`; an entry that cannot be carried out is an input
/// error.
fn add_parts(
    board: &mut Expr,
    output: &Path,
    libraries: &BTreeMap<String, PathBuf>,
    designer: Option<&crate::board_footprints::BoardFootprints>,
    missing: &mut Vec<String>,
    kept_ids: &mut BTreeSet<String>,
) -> Result<Vec<String>, String> {
    let path = output.join("constraints.json");
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|error| {
        crate::verdict::input_error(format!("invalid constraints {}: {error}", path.display()))
    })?;
    let Some(entries) = value.get("add") else {
        return Ok(Vec::new());
    };
    let additions: Vec<crate::placement_constraints::KiCadAddConstraint> =
        serde_json::from_value(entries.clone()).map_err(|error| {
            crate::verdict::input_error(format!("invalid add in {}: {error}", path.display()))
        })?;
    let invalid = |part: &str, why: String| {
        crate::verdict::input_error(format!("constraints.json add {part}: {why}"))
    };
    let nets_on_board: BTreeSet<String> = board
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .flat_map(|footprint| {
            footprint
                .children()
                .iter()
                .filter(|item| item.head() == Some("pad"))
                .filter_map(|pad| form_atom(pad, "net", 1))
        })
        .map(str::to_owned)
        .collect();
    let mut lines = Vec::new();
    for addition in additions {
        let part = addition.part.as_str();
        if part.is_empty() {
            return Err(invalid(
                part,
                "give the new part's reference in part".into(),
            ));
        }
        let taken = board
            .children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
            .any(|footprint| footprint_reference(footprint).as_deref() == Some(part));
        if taken {
            return Err(invalid(
                part,
                "the board has a part of that reference already".into(),
            ));
        }
        let mut kept = None;
        let side_back = match addition.side.as_deref() {
            None => None,
            Some("front") => Some(false),
            Some("back") => Some(true),
            Some(other) => {
                return Err(invalid(
                    part,
                    format!("side {other:?} is neither front nor back"),
                ));
            }
        };
        if addition.x.is_some() != addition.y.is_some() {
            return Err(invalid(part, "give both x and y, or neither".into()));
        }
        let absolute = match addition.origin.as_deref() {
            None | Some("board") => false,
            Some("absolute") => true,
            Some(other) => {
                return Err(invalid(
                    part,
                    format!("unknown origin {other:?} (board or absolute)"),
                ));
            }
        };
        // The footprint in library form, the board's nets of its pads, its
        // pose there and its name.
        let (form, board_nets, board_pose, id) = match (&addition.footprint, addition.from_board) {
            (Some(_), true) | (None, false) => {
                return Err(invalid(part, "give footprint (a library footprint) or from_board (the --footprints-from board's), one of them".into()));
            }
            (None, true) => {
                let designer =
                    designer.ok_or_else(|| invalid(part, "from_board takes the footprint from the board given with --footprints-from".into()))?;
                let at = match (addition.x, addition.y) {
                    (Some(x), Some(y)) if absolute => Some([x, y]),
                    _ => None,
                };
                let theirs = designer
                    .by_reference(addition.board_reference.as_deref().unwrap_or(part), at)
                    .map_err(|why| invalid(part, why))?;
                let nets: BTreeMap<String, String> = theirs
                    .expr
                    .children()
                    .iter()
                    .filter(|item| item.head() == Some("pad"))
                    .filter_map(|pad| {
                        let number = pad.children().get(1).and_then(Expr::atom)?;
                        let net = pad.child("net")?.children().last()?.atom()?;
                        (!net.is_empty() && !number.is_empty())
                            .then(|| (number.to_string(), net.to_string()))
                    })
                    .collect();
                let at = form_at(&theirs.expr).unwrap_or([0.0; 3]);
                let back = form_atom(&theirs.expr, "layer", 1) == Some("B.Cu");
                kept = kept_uuid(theirs, kept_ids);
                (
                    crate::board_footprints::library_form(
                        &theirs.expr,
                        crate::board_footprints::name_of(&theirs.id),
                    )?,
                    nets,
                    Some((at, back)),
                    theirs.id.clone(),
                )
            }
            (Some(id), false) => {
                let Some((file, found)) = find_footprint(libraries, id) else {
                    missing.push(format!("{part} (add: {id} not found in the libraries)"));
                    continue;
                };
                match fs::read_to_string(&file)
                    .map_err(|error| error.to_string())
                    .and_then(|text| parse(&text))
                    .and_then(|footprint| match footprint_form_error(&footprint) {
                        Some(why) => Err(format!("the file holds {why}")),
                        None => Ok(footprint),
                    }) {
                    Ok(footprint) => (footprint, BTreeMap::new(), None, found),
                    Err(error) => {
                        missing.push(format!("{part} (add: {found} unreadable: {error})"));
                        continue;
                    }
                }
            }
        };
        let nets = addition.nets.clone().unwrap_or(board_nets);
        // Where: x and y as given (at the origin when they are relative to
        // the outline, which the layout draws: its `place` puts it there),
        // else where the board has it (locked), else anywhere.
        let (at, back, locked) = match (addition.x, addition.y, board_pose) {
            (Some(x), Some(y), pose) => {
                let angle = addition.angle.or(pose.map(|(at, _)| at[2])).unwrap_or(0.0);
                let at = if absolute {
                    [x, y, angle]
                } else {
                    [0.0, 0.0, angle]
                };
                (
                    at,
                    side_back.or(pose.map(|(_, back)| back)).unwrap_or(false),
                    false,
                )
            }
            (_, _, Some((at, back))) => (
                [at[0], at[1], addition.angle.unwrap_or(at[2])],
                side_back.unwrap_or(back),
                true,
            ),
            _ => (
                [0.0, 0.0, addition.angle.unwrap_or(0.0)],
                side_back.unwrap_or(false),
                false,
            ),
        };
        let mut footprint = form;
        {
            let Expr::List(items) = &mut footprint else {
                return Err(invalid(part, "the footprint is not a list".into()));
            };
            items.retain(|item| {
                !matches!(
                    item.head(),
                    Some("version" | "generator" | "generator_version" | "at" | "uuid" | "tstamp")
                )
            });
            items[0] = Expr::Atom("footprint".into());
            items[1] = Expr::Atom(requote(&id));
            if kept.is_none() {
                for item in items.iter_mut() {
                    renew_uuids(item, &format!("add {part}"));
                }
            }
            legacy_texts_to_properties(items, part)?;
            legacy_arcs_to_three_points(items)?;
            set_property(items, "Reference", part, part)?;
            // Not in the schematic: KiCad's parity check leaves it out.
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
            if let Expr::List(flags) = &mut items[attr]
                && !flags.iter().any(|flag| flag.atom() == Some("board_only"))
            {
                flags.push(Expr::Atom("board_only".into()));
            }
            let mut unused: BTreeSet<&String> = nets.keys().collect();
            for item in items.iter_mut().filter(|item| item.head() == Some("pad")) {
                let number = item
                    .children()
                    .get(1)
                    .and_then(Expr::atom)
                    .unwrap_or("")
                    .to_string();
                let Expr::List(parts) = item else { continue };
                parts
                    .retain(|part| !matches!(part.head(), Some("net" | "pinfunction" | "pintype")));
                if let Some(net) = nets.get(&number) {
                    unused.remove(&number);
                    parts.push(parse(&format!("(net {})", quote(net)))?);
                }
            }
            if let Some(number) = unused.first() {
                return Err(invalid(part, format!("{id} has no pad {number}")));
            }
        }
        let uuid = kept.unwrap_or_else(|| uuid_of(&format!("add footprint {part}")));
        let mut footprint = crate::board_footprints::posed(
            &footprint,
            if back { [at[0], at[1], -at[2]] } else { at },
        )?;
        if back {
            crate::flip::flip_footprint(&mut footprint)?;
        }
        if let Expr::List(items) = &mut footprint {
            let layer = items
                .iter()
                .position(|item| item.head() == Some("layer"))
                .map_or(2, |index| index + 1);
            items.insert(layer, parse(&format!("(uuid \"{uuid}\")"))?);
            if locked {
                items.insert(layer, parse("(locked yes)")?);
            }
        }
        let mut on: Vec<String> = Vec::new();
        let mut net_names: BTreeSet<&String> = nets.values().collect::<BTreeSet<_>>();
        for (number, net) in &nets {
            on.push(format!(
                "{} {net}",
                if number.is_empty() { "\"\"" } else { number }
            ));
        }
        net_names.retain(|net| !nets_on_board.contains(net.as_str()));
        let mut line = format!("{part}: {id}");
        if !on.is_empty() {
            line.push_str(&format!(", pads on nets ({})", on.join(", ")));
        }
        line.push_str(&match (addition.x, locked) {
            (Some(_), _) => format!(
                ", at {} {} ({})",
                addition.x.unwrap_or(0.0),
                addition.y.unwrap_or(0.0),
                if absolute {
                    "absolute"
                } else {
                    "from the outline's corner, by the layout"
                }
            ),
            (None, true) => format!(", where the board has it ({} {}, locked)", at[0], at[1]),
            (None, false) => ", placed by the layout".into(),
        });
        if back {
            line.push_str(", on the back");
        }
        if !net_names.is_empty() {
            line.push_str(&format!(
                "; nets the schematic does not have: {}",
                net_names
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        lines.push(line);
        if let Expr::List(items) = board {
            items.push(footprint);
        }
    }
    Ok(lines)
}

/// Per footprint, the smallest gap between its own pads that KiCad's DRC
/// finds below the clearance rule, and that rule. The footprints are
/// checked apart from each other, in rows, with the project's rules (on a
/// stack of footprints KiCad stops reporting after 499 clearance errors).
fn own_pad_gaps(
    board: &Expr,
    project: &Path,
    board_id: &str,
) -> Result<BTreeMap<String, (f64, f64)>, String> {
    let mut spread = board.clone();
    let Expr::List(items) = &mut spread else {
        return Ok(BTreeMap::new());
    };
    let (mut x, mut y, mut row) = (0.0f64, 0.0f64, 0.0f64);
    for footprint in items
        .iter_mut()
        .filter(|item| item.head() == Some("footprint"))
    {
        // Pads reach this far from the origin.
        let reach = footprint
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
            .map(|pad| {
                let at = form_at(pad).unwrap_or([0.0; 3]);
                let size = form_f64(pad, "size", 1)
                    .unwrap_or(0.0)
                    .max(form_f64(pad, "size", 2).unwrap_or(0.0));
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
    fs::create_dir_all(&directory)
        .map_err(|error| format!("failed to create {}: {error}", directory.display()))?;
    let settings = project.join(format!("{board_id}.kicad_pro"));
    if settings.exists() {
        fs::copy(&settings, directory.join(format!("{board_id}.kicad_pro")))
            .map_err(|error| error.to_string())?;
    }
    let board = directory.join(format!("{board_id}.kicad_pcb"));
    fs::write(&board, format!("{}\n", encode(&spread)))
        .map_err(|error| format!("failed to write {}: {error}", board.display()))?;
    let report = directory.join("drc.json");
    // Without kicad-cli (a netlist from elsewhere) there is no check.
    // Only clearance findings are read: KiCad's configuration without
    // libraries (`kicad_config`) spares the footprint library load.
    let mut command = Command::new(crate::native_report::kicad_cli());
    command
        .args(["pcb", "drc", "--format", "json", "--severity-error", "-o"])
        .arg(&report)
        .arg(&board);
    crate::kicad_config::configure_drc(&mut command, &crate::native_report::kicad_cli(), &board);
    let Ok(output) = command.output() else {
        let _ = fs::remove_dir_all(&directory);
        return Ok(BTreeMap::new());
    };
    let text = fs::read_to_string(&report);
    let _ = fs::remove_dir_all(&directory);
    if !output.status.success() {
        return Err(format!(
            "kicad-cli pcb drc failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text =
        text.map_err(|error| format!("failed to read the pad check's DRC report: {error}"))?;
    let drc: serde_json::Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    // "Clearance violation (netclass 'Default' clearance 0.2800 mm; actual 0.2496 mm)"
    let number_after = |text: &str, key: &str| -> Option<f64> {
        let rest = &text[text.find(key)? + key.len()..];
        rest.split_whitespace().next()?.parse().ok()
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
        if let ([Some(a), Some(b)], Some(gap), Some(rule)) = (
            owners.as_slice(),
            number_after(description, "actual"),
            number_after(description, "clearance"),
        ) && a == b
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
    fn stock_libraries_are_found_on_macos_and_by_kicads_variables() {
        let environment = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            }
        };
        // KiCad's own variable first, on any platform; the macOS bundle
        // among the standard places.
        let directories = stock_footprint_directories(&environment(&[(
            "KICAD10_FOOTPRINT_DIR",
            "/opt/kicad/footprints",
        )]));
        assert_eq!(directories[0], Path::new("/opt/kicad/footprints"));
        assert!(directories.contains(&PathBuf::from(
            "/Applications/KiCad/KiCad.app/Contents/SharedSupport/footprints"
        )));
        let directories =
            stock_footprint_directories(&environment(&[("KICAD9_FOOTPRINT_DIR", "/opt/kicad9")]));
        assert_eq!(directories[0], Path::new("/opt/kicad9"));
        // The user's table on macOS, KiCad 10's before 9's.
        let tables = user_footprint_tables(&environment(&[("HOME", "/Users/a")]));
        let mac = PathBuf::from("/Users/a/Library/Preferences/kicad/10.0/fp-lib-table");
        let linux = PathBuf::from("/Users/a/.config/kicad/10.0/fp-lib-table");
        let older = PathBuf::from("/Users/a/Library/Preferences/kicad/9.0/fp-lib-table");
        let position = |table: &PathBuf| tables.iter().position(|known| known == table).unwrap();
        assert!(position(&linux) < position(&older) && position(&mac) < position(&older));
        let tables =
            user_footprint_tables(&environment(&[("APPDATA", "C:/Users/a/AppData/Roaming")]));
        assert_eq!(
            tables[0],
            Path::new("C:/Users/a/AppData/Roaming/kicad/10.0/fp-lib-table")
        );
    }

    #[test]
    fn kicad_10_names_and_add_on_nicknames_are_found() {
        // The schematic trial's three: two WSON-8 that KiCad 10 renamed with
        // a vendor prefix, one ESP32 module of Espressif's add-on library.
        let dir = std::env::temp_dir().join(format!("ux3-kicad10-names-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let son = dir.join("Package_SON.pretty");
        let espressif = dir.join("3rdparty/footprints/com_github_espressif/PCM_Espressif.pretty");
        fs::create_dir_all(&son).unwrap();
        fs::create_dir_all(&espressif).unwrap();
        for name in [
            "Texas_DSG0008A_WSON-8-1EP_2x2mm_P0.5mm_EP0.9x1.6mm",
            "Texas_DSG0008A_WSON-8-1EP_2x2mm_P0.5mm_EP0.9x1.6mm_ThermalVias",
            "A_SOIC-8",
            "B_SOIC-8",
        ] {
            fs::write(son.join(format!("{name}.kicad_mod")), "(footprint)").unwrap();
        }
        fs::write(espressif.join("ESP32-S3-MINI-1.kicad_mod"), "(footprint)").unwrap();
        let libraries = BTreeMap::from([
            ("Package_SON".to_string(), son.clone()),
            ("PCM_Espressif".to_string(), espressif.clone()),
        ]);
        let found = |id: &str| find_footprint(&libraries, id).map(|(_, found)| found);
        let wson = "Package_SON:WSON-8-1EP_2x2mm_P0.5mm_EP0.9x1.6mm_ThermalVias";
        let renamed = "Package_SON:Texas_DSG0008A_WSON-8-1EP_2x2mm_P0.5mm_EP0.9x1.6mm_ThermalVias";
        assert_eq!(found(wson).as_deref(), Some(renamed));
        assert_eq!(
            found("Package_SON:WSON-8-1EP_2x2mm_P0.5mm_EP0.9x1.6mm").as_deref(),
            Some("Package_SON:Texas_DSG0008A_WSON-8-1EP_2x2mm_P0.5mm_EP0.9x1.6mm")
        );
        assert!(substitution_reason(wson, renamed).contains("KiCad 10's name"));
        // Two footprints end in the name: neither is taken.
        assert_eq!(found("Package_SON:SOIC-8"), None);
        // The add-on under its Plugin and Content Manager nickname.
        let esp = found("Espressif:ESP32-S3-MINI-1").unwrap();
        assert_eq!(esp, "PCM_Espressif:ESP32-S3-MINI-1");
        assert!(substitution_reason("Espressif:ESP32-S3-MINI-1", &esp).contains("add-on"));
        // KiCad's add-on folder, by its variable and by default.
        let environment = |name: &str| match name {
            "KICAD10_3RD_PARTY" => Some("/opt/3rdparty".to_string()),
            "HOME" => Some("/home/a".to_string()),
            _ => None,
        };
        let folders = third_party_directories(&environment);
        assert_eq!(folders[0], Path::new("/opt/3rdparty"));
        assert!(folders.contains(&PathBuf::from("/home/a/.local/share/kicad/10.0/3rdparty")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn plug_in_connectors_are_told_by_their_footprint_names() {
        for name in [
            "Connector_USB:USB_C_Receptacle_GCT_USB4105",
            "x:USB2.0_TYPE-C",
            "Connector_Coaxial:SMA_Amphenol_901-144_Vertical",
            "Connector_BarrelJack:BarrelJack_Horizontal",
            "Connector_Audio:Jack_3.5mm_CUI_SJ-3523-SMT_Horizontal",
            "Connector_RJ:RJ45_Amphenol_54602-x08_Horizontal",
            "Connector_Dsub:DSUB-9_Socket_Horizontal",
            "TerminalBlock_Phoenix:TerminalBlock_Phoenix_MKDS-1,5-2",
            "Main:PCIEXPRESS-X1",
        ] {
            assert!(edge_connector(name), "{name}");
        }
        for name in [
            "Logos:LordsBoardsLogo_Small_Silk",
            "Connector_Coaxial:U.FL_Hirose_U.FL-R-SMT-1_Vertical",
            "Connector_PinHeader_2.54mm:PinHeader_1x04_P2.54mm_Vertical",
            "Package_SO:SOIC-8_3.9x4.9mm_P1.27mm",
        ] {
            assert!(!edge_connector(name), "{name}");
        }
    }

    #[test]
    fn kicad_5_arcs_become_three_points() {
        // A quarter circle around (2, 0) from (2, 1) to (3, 0), as the
        // Tag-Connect footprint of osprey's library draws its corners.
        let mut footprint = parse(r#"(footprint "x" (fp_arc (start 2 0) (end 2 1) (angle -90) (layer "F.SilkS") (width 0.2)) (fp_arc (start 0 0) (mid 1 1) (end 2 0) (layer "F.SilkS")))"#).unwrap();
        let Expr::List(items) = &mut footprint else {
            unreachable!()
        };
        legacy_arcs_to_three_points(items).unwrap();
        let arc = &footprint.children()[2];
        assert_eq!(encode(arc), encode(&parse(r#"(fp_arc (start 2 1) (mid 2.707107 0.707107) (end 3 0) (layer "F.SilkS") (width 0.2))"#).unwrap()));
        // A KiCad 7 arc stays as it is.
        assert_eq!(
            encode(&footprint.children()[3]),
            encode(
                &parse(r#"(fp_arc (start 0 0) (mid 1 1) (end 2 0) (layer "F.SilkS"))"#).unwrap()
            )
        );
    }

    #[test]
    fn kicad_5_library_footprints_become_footprints_of_today() {
        let directory = std::env::temp_dir().join(format!(
            "pcb-maker-netlist-import-k5-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let library = directory.join("old.pretty");
        fs::create_dir_all(&library).unwrap();
        fs::write(
            library.join("FB.kicad_mod"),
            r#"(module FB (layer F.Cu) (tedit 5A0F774F)
                (fp_text reference REF** (at 0 -2) (layer F.SilkS) (effects (font (size 1 1) (thickness 0.15))))
                (fp_text value FB (at 0 2) (layer F.Fab) (effects (font (size 1 1) (thickness 0.15))))
                (fp_arc (start 0 0) (end 0 1) (angle -90) (layer F.SilkS) (width 0.12))
                (pad 1 thru_hole circle (at -2 0) (size 1.6 1.6) (drill 0.8) (layers *.Cu *.Mask))
                (pad 2 thru_hole circle (at 2 0) (size 1.6 1.6) (drill 0.8) (layers *.Cu *.Mask)))"#,
        )
        .unwrap();
        let netlist = directory.join("test.net");
        fs::write(
            &netlist,
            r#"(export (version "E")
              (components (comp (ref "FB1") (value "FB") (footprint "old:FB") (sheetpath (names "/") (tstamps "/")) (tstamps "1111")))
              (nets (net (code "1") (name "VCC") (node (ref "FB1") (pin "1") (pintype "passive")))))"#,
        )
        .unwrap();
        let report = import_kicad_netlist(&netlist, &directory.join("out"), "test", 2).unwrap();
        assert!(report.missing.is_empty(), "{:?}", report.missing);
        let board = parse(&fs::read_to_string(&report.board).unwrap()).unwrap();
        let footprint = board
            .children()
            .iter()
            .find(|item| item.head() == Some("footprint"))
            .expect("a footprint, not a module");
        let arc = footprint.child("fp_arc").unwrap();
        assert!(arc.child("mid").is_some() && arc.child("angle").is_none());
        assert_eq!(
            form_atom(footprint.child("pad").unwrap(), "net", 1),
            Some("VCC")
        );
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn netlist_becomes_a_board_with_nets_fields_and_symbol_links() {
        let directory = std::env::temp_dir().join(format!(
            "pcb-maker-netlist-import-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
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
        assert_eq!(report.missing, vec!["R2 (Missing:R_Nowhere not found: no library \"Missing\" in the fp-lib-table files, the project's .pretty folders, KiCad's own or its add-on libraries)".to_string()]);
        assert_eq!(
            report.substituted,
            vec!["R3: other:R_Test -> parts:R_Test".to_string()]
        );

        let board = parse(&fs::read_to_string(&report.board).unwrap()).unwrap();
        let layers = board.child("layers").unwrap().children();
        assert!(
            layers
                .iter()
                .any(|layer| layer.children().get(1).and_then(Expr::atom) == Some("In2.Cu"))
        );
        let footprints: Vec<&Expr> = board
            .children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
            .collect();
        let property = |footprint: &Expr, name: &str| {
            footprint
                .children()
                .iter()
                .find(|item| {
                    item.head() == Some("property")
                        && item.children().get(1).and_then(Expr::atom) == Some(name)
                })
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
        assert!(
            r1.child("version").is_none() && r1.child("at").is_some() && r1.child("uuid").is_some()
        );
        let nets: Vec<Option<&str>> = r1
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
            .map(|pad| form_atom(pad, "net", 1))
            .collect();
        assert_eq!(nets, vec![Some("/VIN"), Some("GND")]);
        let r3 = footprints[1];
        assert_eq!(form_atom(r3, "path", 1), Some("/abcd/3333"));
        assert_eq!(form_atom(r3, "sheetname", 1), Some("/sub/"));
        assert!(
            r3.child("attr")
                .unwrap()
                .children()
                .iter()
                .any(|flag| flag.atom() == Some("dnp"))
        );
        let pads: Vec<Option<&str>> = r3
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
            .map(|pad| form_atom(pad, "net", 1))
            .collect();
        assert_eq!(pads, vec![None, Some("GND")]);
        fs::remove_dir_all(&directory).unwrap();
    }
    #[test]
    fn footprints_come_from_the_board_where_no_library_has_them_and_the_board_version_wins() {
        let directory = std::env::temp_dir().join(format!(
            "pcb-maker-netlist-import-board-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        // A library with an old, larger R_Test (drifted: 6 x 2 against 3 x 1).
        let library = directory.join("parts.pretty");
        fs::create_dir_all(&library).unwrap();
        fs::write(
            library.join("R_Test.kicad_mod"),
            r#"(footprint "R_Test" (layer "F.Cu")
                (fp_rect (start -3 -1) (end 3 1) (layer "F.CrtYd") (stroke (width 0.05) (type solid)))
                (pad "1" smd rect (at -2 0) (size 1 1) (layers "F.Cu" "F.Mask"))
                (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu" "F.Mask")))"#,
        )
        .unwrap();
        // The designer's board: R1 (parts:R_Test, smaller, on the back,
        // turned), U1 from a personal library nobody has, H1 a mounting hole
        // the schematic lacks, plated on GND.
        let board = directory.join("designer.kicad_pcb");
        fs::write(
            &board,
            r#"(kicad_pcb (version 20221018) (generator pcbnew)
              (footprint "parts:R_Test" (layer "B.Cu") (tstamp 11111111-0000-4000-8000-000000000001) (at 10 20 90)
                (path "/1111")
                (fp_text reference "R1" (at 0 1.5 90) (layer "B.SilkS") (effects (font (size 1 1) (thickness 0.15)) (justify mirror)))
                (fp_rect (start -1.5 0.5) (end 1.5 -0.5) (layer "B.CrtYd") (stroke (width 0.05) (type solid)))
                (pad "1" smd rect (at -1 0 90) (size 0.6 0.6) (layers "B.Cu" "B.Mask") (net 1 "/VIN"))
                (pad "2" smd rect (at 1 0 90) (size 0.6 0.6) (layers "B.Cu" "B.Mask") (net 2 "GND")))
              (footprint "personal:SOT_Mine" (layer "F.Cu") (tstamp 11111111-0000-4000-8000-000000000002) (at 30 20 180)
                (path "/2222")
                (fp_text reference "U1" (at 0 -2 180) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
                (pad "1" smd rect (at -1 1 180) (size 0.6 0.8) (layers "F.Cu" "F.Mask") (net 2 "GND"))
                (pad "2" smd rect (at 1 1 180) (size 0.6 0.8) (layers "F.Cu" "F.Mask") (net 1 "/VIN")))
              (footprint "MountingHole:Hole_Pad" (layer "F.Cu") (tstamp 11111111-0000-4000-8000-000000000003) (at 3 3)
                (fp_text reference "H1" (at 0 -3) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
                (pad "1" thru_hole circle (at 0 0) (size 5 5) (drill 3) (layers "*.Cu" "*.Mask") (net 2 "GND"))))"#,
        )
        .unwrap();
        let netlist = directory.join("test.net");
        fs::write(
            &netlist,
            r#"(export (version "E")
              (components
                (comp (ref "R1") (value "10k") (footprint "parts:R_Test") (sheetpath (names "/") (tstamps "/")) (tstamps "1111"))
                (comp (ref "U1") (value "X") (footprint "personal:SOT_Mine") (sheetpath (names "/") (tstamps "/")) (tstamps "2222")))
              (nets
                (net (code "1") (name "/VIN") (node (ref "R1") (pin "1") (pintype "passive")) (node (ref "U1") (pin "2") (pintype "passive")))
                (net (code "2") (name "GND") (node (ref "R1") (pin "2") (pintype "passive")) (node (ref "U1") (pin "1") (pintype "passive")))))"#,
        )
        .unwrap();
        let output = directory.join("out");
        // Without the board, U1 is missing.
        let report = import_kicad_netlist(&netlist, &output, "test", 2).unwrap();
        assert_eq!(report.missing.len(), 1, "{:?}", report.missing);
        assert!(report.footprints_from_board.is_none() && report.drifted_footprints.is_none());
        // With it, and with an `add` for the mounting hole (kept where the
        // board has it) and a library footprint placed by the layout.
        fs::write(
            output.join("constraints.json"),
            r#"{"version": 1, "add": [{"part": "H1", "from_board": true},
                {"part": "TP1", "footprint": "parts:R_Test", "nets": {"1": "GND"}, "x": 5, "y": 6}]}"#,
        )
        .unwrap();
        let report = import_kicad_netlist_from(&netlist, &output, "test", 2, Some(&board)).unwrap();
        assert!(report.missing.is_empty(), "{:?}", report.missing);
        assert_eq!(
            report.footprints_from_board.as_deref(),
            Some(&["U1: personal:SOT_Mine (the board's U1)".to_string()][..])
        );
        assert_eq!(report.board_versions, Some(1));
        let drifted = report.drifted_footprints.clone().unwrap();
        assert_eq!(drifted.len(), 1, "{drifted:?}");
        assert!(
            drifted[0].starts_with(
                "parts:R_Test (1 parts: R1): library 6.00 x 2.00 mm, 2 pads; board 3.00 x 1.00 mm"
            ),
            "{drifted:?}"
        );
        assert_eq!(report.added.len(), 2, "{:?}", report.added);
        assert!(report.added[0].starts_with("H1: MountingHole:Hole_Pad, pads on nets (1 GND), where the board has it (3 3, locked)"), "{:?}", report.added);
        // The personal library is in the project now, with its table entry.
        let written =
            parse(&fs::read_to_string(output.join("personal.pretty/SOT_Mine.kicad_mod")).unwrap())
                .unwrap();
        assert_eq!(written.children()[1].atom(), Some("SOT_Mine"));
        assert!(written.child("at").is_none() && written.child("path").is_none());
        let pad = written
            .children()
            .iter()
            .find(|item| item.head() == Some("pad"))
            .unwrap();
        assert_eq!(form_at(pad).unwrap(), [-1.0, 1.0, 0.0]);
        assert!(pad.child("net").is_none());
        assert!(
            fs::read_to_string(output.join("fp-lib-table"))
                .unwrap()
                .contains("${KIPRJMOD}/personal.pretty")
        );

        let imported = parse(&fs::read_to_string(&report.board).unwrap()).unwrap();
        let footprint = |reference: &str| {
            imported
                .children()
                .iter()
                .find(|item| {
                    item.head() == Some("footprint")
                        && footprint_reference(item).as_deref() == Some(reference)
                })
                .unwrap()
                .clone()
        };
        // R1 is the board's version, in library form on the front.
        let r1 = footprint("R1");
        assert_eq!(form_atom(&r1, "layer", 1), Some("F.Cu"));
        let pads: Vec<[f64; 3]> = r1
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
            .map(|pad| form_at(pad).unwrap())
            .collect();
        assert_eq!(pads, vec![[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]]);
        assert_eq!(
            form_atom(
                r1.children()
                    .iter()
                    .find(|item| item.head() == Some("pad"))
                    .unwrap(),
                "net",
                1
            ),
            Some("/VIN")
        );
        // H1 where the board has it, locked, board only, on GND.
        let h1 = footprint("H1");
        assert_eq!(form_at(&h1).unwrap(), [3.0, 3.0, 0.0]);
        assert!(h1.child("locked").is_some());
        assert!(
            h1.child("attr")
                .unwrap()
                .children()
                .iter()
                .any(|flag| flag.atom() == Some("board_only"))
        );
        assert_eq!(form_atom(h1.child("pad").unwrap(), "net", 1), Some("GND"));
        // TP1 from the library, its pad 1 on GND, pad 2 on none; the layout
        // puts it at 5, 6 from the outline's corner.
        let tp1 = footprint("TP1");
        let nets: Vec<Option<&str>> = tp1
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
            .map(|pad| form_atom(pad, "net", 1))
            .collect();
        assert_eq!(nets, vec![Some("GND"), None]);
        let mut constraints = read_placement_constraints(&output.join("constraints.json")).unwrap();
        crate::placement_constraints::expand_additions(&mut constraints, &imported).unwrap();
        assert!(constraints.add.is_empty());
        assert_eq!(constraints.place.len(), 1);
        assert_eq!(
            (
                constraints.place[0].part.as_str(),
                constraints.place[0].x,
                constraints.place[0].y
            ),
            ("TP1", Some(5.0), Some(6.0))
        );
        // The layout refuses an added part the board does not have.
        let mut stale = read_placement_constraints(&output.join("constraints.json")).unwrap();
        let empty = parse("(kicad_pcb)").unwrap();
        assert!(
            crate::placement_constraints::expand_additions(&mut stale, &empty)
                .unwrap_err()
                .contains("import again")
        );

        // An added part may not take a schematic part's reference, and
        // from_board needs the board.
        fs::write(
            output.join("constraints.json"),
            r#"{"version": 1, "add": [{"part": "R1", "footprint": "parts:R_Test"}]}"#,
        )
        .unwrap();
        assert!(
            import_kicad_netlist(&netlist, &output, "test", 2)
                .unwrap_err()
                .contains("has a part of that reference already")
        );
        fs::write(
            output.join("constraints.json"),
            r#"{"version": 1, "add": [{"part": "H1", "from_board": true}]}"#,
        )
        .unwrap();
        assert!(
            import_kicad_netlist(&netlist, &output, "test", 2)
                .unwrap_err()
                .contains("--footprints-from")
        );
        fs::remove_dir_all(&directory).unwrap();
    }
    #[test]
    fn footprints_from_the_board_keep_their_ids_for_the_drc_exclusions() {
        let directory = std::env::temp_dir().join(format!(
            "pcb-maker-netlist-import-ids-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        // A KiCad 6 library footprint: its ids are `tstamp`s, one set for
        // every instance.
        let library = directory.join("parts.pretty");
        fs::create_dir_all(&library).unwrap();
        fs::write(
            library.join("C_Old.kicad_mod"),
            r#"(footprint "C_Old" (layer "F.Cu") (tstamp 22222222-0000-4000-8000-000000000000)
                (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd") (width 0.05) (tstamp 22222222-0000-4000-8000-000000000001))
                (pad "1" smd rect (at -0.5 0) (size 0.5 0.5) (layers "F.Cu" "F.Mask") (tstamp 22222222-0000-4000-8000-000000000002))
                (pad "2" smd rect (at 0.5 0) (size 0.5 0.5) (layers "F.Cu" "F.Mask") (tstamp 22222222-0000-4000-8000-000000000003)))"#,
        )
        .unwrap();
        // The designer's board: U1 (its own footprint, from a library
        // nobody has), H1 (a mounting hole on GND the schematic lacks).
        let board = directory.join("designer.kicad_pcb");
        fs::write(
            &board,
            r#"(kicad_pcb (version 20240108) (generator pcbnew)
              (footprint "personal:SOT_Mine" (layer "F.Cu") (uuid "11111111-0000-4000-8000-000000000002") (at 30 20 180)
                (path "/2222")
                (property "Reference" "U1" (at 0 -2 180) (layer "F.SilkS") (uuid "11111111-0000-4000-8000-0000000000a0") (effects (font (size 1 1) (thickness 0.15))))
                (fp_rect (start -1.5 -1.5) (end 1.5 1.5) (layer "F.CrtYd") (stroke (width 0.05) (type solid)) (uuid "11111111-0000-4000-8000-0000000000a1"))
                (pad "1" smd rect (at -1 1 180) (size 0.6 0.8) (layers "F.Cu" "F.Mask") (net 2 "GND") (uuid "11111111-0000-4000-8000-0000000000a2"))
                (pad "2" smd rect (at 1 1 180) (size 0.6 0.8) (layers "F.Cu" "F.Mask") (net 1 "/VIN") (uuid "11111111-0000-4000-8000-0000000000a3")))
              (footprint "MountingHole:Hole_Pad" (layer "F.Cu") (uuid "11111111-0000-4000-8000-000000000003") (at 3 3)
                (property "Reference" "H1" (at 0 -3 0) (layer "F.SilkS") (uuid "11111111-0000-4000-8000-0000000000b0") (effects (font (size 1 1) (thickness 0.15))))
                (pad "1" thru_hole circle (at 0 0) (size 5 5) (drill 3) (layers "*.Cu" "*.Mask") (net 2 "GND") (uuid "11111111-0000-4000-8000-0000000000b1"))))"#,
        )
        .unwrap();
        // U2 has U1's footprint, which the board has only as U1's: a copy.
        let netlist = directory.join("test.net");
        fs::write(
            &netlist,
            r#"(export (version "E")
              (components
                (comp (ref "U1") (value "X") (footprint "personal:SOT_Mine") (sheetpath (names "/") (tstamps "/")) (tstamps "2222"))
                (comp (ref "U2") (value "X") (footprint "personal:SOT_Mine") (sheetpath (names "/") (tstamps "/")) (tstamps "3333"))
                (comp (ref "C1") (value "1u") (footprint "parts:C_Old") (sheetpath (names "/") (tstamps "/")) (tstamps "4444"))
                (comp (ref "C2") (value "1u") (footprint "parts:C_Old") (sheetpath (names "/") (tstamps "/")) (tstamps "5555")))
              (nets
                (net (code "1") (name "/VIN") (node (ref "U1") (pin "2") (pintype "passive")) (node (ref "U2") (pin "2") (pintype "passive")))
                (net (code "2") (name "GND") (node (ref "U1") (pin "1") (pintype "passive")) (node (ref "C1") (pin "1") (pintype "passive")))))"#,
        )
        .unwrap();
        let output = directory.join("out");
        fs::create_dir_all(&output).unwrap();
        fs::write(
            output.join("constraints.json"),
            r#"{"version": 1, "add": [{"part": "H1", "from_board": true}]}"#,
        )
        .unwrap();
        let report = import_kicad_netlist_from(&netlist, &output, "test", 2, Some(&board)).unwrap();
        assert!(report.missing.is_empty(), "{:?}", report.missing);
        let imported = parse(&fs::read_to_string(&report.board).unwrap()).unwrap();
        let ids = |reference: &str| -> Vec<String> {
            fn collect(item: &Expr, into: &mut Vec<String>) {
                if matches!(item.head(), Some("uuid" | "tstamp")) {
                    into.push(
                        item.children()[1]
                            .atom()
                            .unwrap()
                            .trim_matches('"')
                            .to_string(),
                    );
                }
                for child in item.children() {
                    collect(child, into);
                }
            }
            let footprint = imported
                .children()
                .iter()
                .find(|item| {
                    item.head() == Some("footprint")
                        && footprint_reference(item).as_deref() == Some(reference)
                })
                .unwrap();
            let mut ids = Vec::new();
            collect(footprint, &mut ids);
            ids
        };
        // U1 and H1 keep the board's ids: the footprint's, its pads' and
        // drawings' (the project's DRC exclusions name them).
        let u1 = ids("U1");
        assert_eq!(u1[0], "11111111-0000-4000-8000-000000000002", "{u1:?}");
        for own in [
            "11111111-0000-4000-8000-0000000000a1",
            "11111111-0000-4000-8000-0000000000a2",
            "11111111-0000-4000-8000-0000000000a3",
        ] {
            assert!(u1.iter().any(|id| id == own), "{own} not in {u1:?}");
        }
        let h1 = ids("H1");
        assert!(
            h1.iter()
                .any(|id| id == "11111111-0000-4000-8000-000000000003"),
            "{h1:?}"
        );
        assert!(
            h1.iter()
                .any(|id| id == "11111111-0000-4000-8000-0000000000b1"),
            "{h1:?}"
        );
        // U2, a copy of U1's footprint, and the library's C1 and C2 get ids
        // of their own (the KiCad 6 library's tstamps too): no id twice.
        let mut all: Vec<String> = ["U1", "U2", "C1", "C2", "H1"]
            .iter()
            .flat_map(|reference| ids(reference))
            .collect();
        assert!(
            ids("U2").iter().all(|id| !id.starts_with("11111111")),
            "{:?}",
            ids("U2")
        );
        assert!(
            ids("C1").iter().all(|id| !id.starts_with("22222222")),
            "{:?}",
            ids("C1")
        );
        let count = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), count, "an id twice: {all:?}");
        fs::remove_dir_all(&directory).unwrap();
    }

    fn hostile(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/hostile")
            .join(name)
    }

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "pcb-maker-hostile-import-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn footprint_ids_and_names_never_make_paths_outside_their_folders() {
        let directory = scratch("paths");
        // 1.2: a netlist's `lib:../evil` reads no file beside the library.
        let library = directory.join("lib.pretty");
        fs::create_dir_all(&library).unwrap();
        fs::write(library.join("R.kicad_mod"), "(footprint \"R\")").unwrap();
        fs::write(directory.join("evil.kicad_mod"), "(footprint \"evil\")").unwrap();
        let libraries = BTreeMap::from([("lib".to_string(), library.clone())]);
        assert!(find_footprint(&libraries, "lib:R").is_some());
        assert!(find_footprint(&libraries, "lib:../evil").is_none());
        assert!(find_footprint(&libraries, "lib:..").is_none());
        // 1.1: a --footprints-from board whose footprint id climbs out of
        // the output directory: refused, nothing written outside it.
        let netlist = directory.join("t.net");
        fs::write(
            &netlist,
            r#"(export (version "E")
              (components
                (comp (ref "J1") (value "X") (footprint "../../x:../../y") (sheetpath (names "/") (tstamps "/")) (tstamps "1111")))
              (nets (net (code "1") (name "/TARGET") (node (ref "J1") (pin "1") (pintype "passive")))))"#,
        )
        .unwrap();
        let output = directory.join("a/b/out");
        let error = import_kicad_netlist_from(
            &netlist,
            &output,
            "t",
            2,
            Some(&hostile("climbing-footprint-id.kicad_pcb")),
        )
        .unwrap_err();
        assert!(error.contains("its library nickname or name"), "{error}");
        assert!(crate::verdict::is_input_error(&error));
        assert!(!directory.join("a/x.pretty").exists());
        assert!(!directory.join("x.pretty").exists());
        // A board id that climbs: refused before anything is written.
        let error = import_kicad_netlist(&netlist, &directory.join("c"), "../t", 2).unwrap_err();
        assert!(error.contains("the board id"), "{error}");
        assert!(!directory.join("c").exists());
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_library_footprint_without_its_name_is_reported_missing_not_a_panic() {
        // 2.1: `(footprint)` in a project's .pretty failed the whole import
        // (index out of bounds).
        let directory = scratch("short");
        let library = directory.join("lib.pretty");
        fs::create_dir_all(&library).unwrap();
        fs::copy(
            hostile("short-footprint.kicad_mod"),
            library.join("R.kicad_mod"),
        )
        .unwrap();
        let netlist = directory.join("t.net");
        fs::write(
            &netlist,
            r#"(export (version "E")
              (components
                (comp (ref "R1") (value "1k") (footprint "lib:R") (sheetpath (names "/") (tstamps "/")) (tstamps "1111")))
              (nets (net (code "1") (name "A") (node (ref "R1") (pin "1") (pintype "passive")))))"#,
        )
        .unwrap();
        let report = import_kicad_netlist(&netlist, &directory.join("out"), "t", 2).unwrap();
        assert_eq!(report.missing.len(), 1, "{:?}", report.missing);
        assert!(
            report.missing[0].contains("the file holds a footprint form without its name"),
            "{:?}",
            report.missing
        );
        // From a --footprints-from board: the input's error.
        let error = import_kicad_netlist_from(
            &netlist,
            &directory.join("out2"),
            "t",
            2,
            Some(&hostile("short-footprint.kicad_pcb")),
        )
        .unwrap_err();
        assert!(
            error.contains("a footprint form without its name"),
            "{error}"
        );
        assert!(crate::verdict::is_input_error(&error));
        // And the forms that index past the name refuse it.
        let short = parse("(footprint)").unwrap();
        assert!(crate::board_footprints::library_form(&short, "L:R").is_err());
        assert!(crate::board_footprints::posed(&short, [0.0; 3]).is_err());
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_library_that_holds_the_output_or_links_out_is_not_copied() {
        // 1.3: `${KIPRJMOD}/.` (or a link `lib -> .`) copied the project
        // into its own output, again and again.
        let directory = scratch("copy");
        let project = directory.join("project");
        let output = project.join("out");
        fs::create_dir_all(project.join("lib.pretty")).unwrap();
        fs::create_dir_all(&output).unwrap();
        fs::write(project.join("lib.pretty/R.kicad_mod"), "(footprint \"R\")").unwrap();
        fs::write(directory.join("secret.kicad_mod"), "(footprint \"S\")").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&project, project.join("lib.pretty/loop")).unwrap();
            std::os::unix::fs::symlink(
                directory.join("secret.kicad_mod"),
                project.join("lib.pretty/secret.kicad_mod"),
            )
            .unwrap();
            std::os::unix::fs::symlink(&directory, project.join("outside.pretty")).unwrap();
        }
        // The project itself: not copied.
        copy_library_tree(&project.join("."), &output.join("."), &project, &output).unwrap();
        assert_eq!(fs::read_dir(&output).unwrap().count(), 0);
        // An ordinary library: its files, not its links.
        copy_library_tree(
            &project.join("lib.pretty"),
            &output.join("lib.pretty"),
            &project,
            &output,
        )
        .unwrap();
        assert!(output.join("lib.pretty/R.kicad_mod").is_file());
        assert!(!output.join("lib.pretty/loop").exists());
        assert!(!output.join("lib.pretty/secret.kicad_mod").exists());
        // A link out of the project: not copied.
        #[cfg(unix)]
        {
            copy_library_tree(
                &project.join("outside.pretty"),
                &output.join("outside.pretty"),
                &project,
                &output,
            )
            .unwrap();
            assert!(!output.join("outside.pretty").exists());
        }
        fs::remove_dir_all(&directory).unwrap();
    }
}
