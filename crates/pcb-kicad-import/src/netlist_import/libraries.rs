// Copyright (C) 2026 Toit contributors.

//! Where the footprints are: the footprint library tables and folders, and
//! the libraries the project gets for footprints taken from a board.

use super::*;

/// Expands `${VAR}` and `$(VAR)`: the project directory, KiCad's footprint
/// directory under any version's name, else the environment.
pub(super) fn expand(uri: &str, project: &Path, system: &str) -> String {
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
pub(super) fn stock_footprint_directories(
    variable: &dyn Fn(&str) -> Option<String>,
) -> Vec<PathBuf> {
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
pub(super) fn third_party_directories(variable: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
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

/// The user's global footprint tables, KiCad 10's before 9's: under
/// `KICAD_CONFIG_HOME`, else `$XDG_CONFIG_HOME/kicad` or `~/.config/kicad`
/// (Linux), `~/Library/Preferences/kicad` (macOS), `%APPDATA%\kicad`
/// (Windows). Missing ones are skipped when read.
pub(super) fn user_footprint_tables(variable: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
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

/// What the library search found of the tables' entries besides the
/// libraries: entries whose folder is not where the table says but was
/// found under the project's folders (`relocated`), and entries found
/// nowhere, by name, with the folder the table gives (`absent`).
#[derive(Default)]
pub(super) struct LibraryNotes {
    pub(super) relocated: Vec<String>,
    pub(super) absent: BTreeMap<String, String>,
}

/// A table's folder that is not where the table says, such as a path on the
/// designer's machine (eurorack-pmod's
/// `/home/seb/dev/eurorack-pmod/hardware/libs/custom/AudioJacksFixed.pretty`,
/// fresh-agent trial 6): the same trailing folders under the project's
/// folder or one of its three parents, the nearest folder and the longest
/// tail first (`<checkout>/hardware/libs/custom/AudioJacksFixed.pretty`).
pub(super) fn relocated_library(folder: &Path, project: &Path) -> Option<PathBuf> {
    let parts: Vec<&std::ffi::OsStr> = folder
        .components()
        .filter_map(|part| match part {
            std::path::Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect();
    if !parts.last()?.to_str()?.ends_with(".pretty") {
        return None;
    }
    let project = project
        .canonicalize()
        .unwrap_or_else(|_| project.to_path_buf());
    for ancestor in project.ancestors().take(4) {
        for start in 0..parts.len() {
            let candidate = parts[start..]
                .iter()
                .fold(ancestor.to_path_buf(), |path, part| path.join(part));
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Footprint libraries by name: from fp-lib-tables (the project's first,
/// then the user's), then `.pretty` folders in the project, then KiCad's
/// own libraries. A table entry whose folder does not exist here falls back
/// to the others.
pub(super) fn footprint_libraries(project: &Path) -> BTreeMap<String, PathBuf> {
    footprint_library_search(project, None).0
}

/// `footprint_libraries`, with what became of the tables' entries whose
/// folder is not there (`LibraryNotes`), and the output directory's
/// `.pretty` folders (where an agent puts a library the import asks for,
/// and where an import with `--footprints-from` wrote the board's) after
/// the project's own.
pub(super) fn footprint_library_search(
    project: &Path,
    output: Option<&Path>,
) -> (BTreeMap<String, PathBuf>, LibraryNotes) {
    let mut notes = LibraryNotes::default();
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
            } else if !libraries.contains_key(name) {
                match relocated_library(&path, project) {
                    Some(found) => {
                        notes.relocated.push(format!(
                            "{name}: the fp-lib-table's {} is not here; used {}",
                            path.display(),
                            found.display()
                        ));
                        notes.absent.remove(name);
                        libraries.insert(name.to_string(), found);
                    }
                    None => {
                        notes
                            .absent
                            .entry(name.to_string())
                            .or_insert_with(|| path.display().to_string());
                    }
                }
            }
        }
    }
    // Libraries shipped in the project without a table entry, then in the
    // output directory.
    let mut folders = vec![(project.to_path_buf(), 0)];
    folders.extend(output.map(|output| (output.to_path_buf(), 0)));
    folders.reverse();
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
    notes.absent.retain(|name, _| !libraries.contains_key(name));
    (libraries, notes)
}

/// Whether a library folder is one of KiCad's add-on libraries (under a
/// `third_party_directories` folder).
pub(super) fn add_on_folder(path: &Path) -> bool {
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
pub(super) fn find_footprint(
    libraries: &BTreeMap<String, PathBuf>,
    id: &str,
) -> Option<(PathBuf, String)> {
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
pub(super) fn substitution_reason(asked: &str, found: &str) -> &'static str {
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

/// Writes the footprints taken from the board into libraries of the
/// project (`<out>/<library>.pretty`) and names those libraries in the
/// project's `fp-lib-table`, so that the project finds every footprint its
/// schematic names. An entry of that name the libraries could not resolve
/// (a designer's own folder) is replaced.
pub(super) fn write_local_libraries(
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
            return Err(crate::exit::input_error(format!(
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

/// Add-on libraries a schematic may name that KiCad does not ship, with
/// where to get them (docs/agents.md, "From a schematic").
pub(super) const ADD_ON_LIBRARIES: [(&str, &str); 1] = [(
    "Espressif",
    "Espressif's library: KiCad's Plugin and Content Manager, \"Espressif KiCad Library\" (installed as PCM_Espressif), or github.com/espressif/kicad-libraries",
)];
