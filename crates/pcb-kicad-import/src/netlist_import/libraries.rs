// Copyright (C) 2026 Toit contributors.

//! Where the footprints are: the footprint library tables and folders, and
//! the libraries the project gets for footprints taken from a board.

use super::legacy_libraries::LegacyLibraries;
use super::*;
use crate::board_footprints::same_library_model;
use std::io::{ErrorKind, Write};

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
    pub(super) legacy: LegacyLibraries,
    table_uses_kicad: BTreeMap<String, bool>,
    output: Option<PathBuf>,
}

impl LibraryNotes {
    pub fn supports_native_additions(&self, nickname: &str, path: &Path) -> bool {
        self.table_uses_kicad
            .get(nickname)
            .copied()
            .unwrap_or_else(|| {
                path.extension()
                    .is_some_and(|extension| extension == "pretty")
            })
    }

    pub fn is_staged_model(&self, path: &Path) -> bool {
        self.output
            .as_ref()
            .is_some_and(|output| fs::canonicalize(path).is_ok_and(|path| path.starts_with(output)))
    }
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
    let mut notes = LibraryNotes {
        output: output.and_then(|path| fs::canonicalize(path).ok()),
        ..LibraryNotes::default()
    };
    let variable = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let candidates = stock_footprint_directories(&variable);
    let system = candidates
        .iter()
        .find(|directory| directory.is_dir())
        .or(candidates.first())
        .map(|directory| directory.display().to_string())
        .unwrap_or_default();
    if let Some(output) = output {
        notes.legacy = LegacyLibraries::read(project, output, &system);
    }
    let project_table = project.join("fp-lib-table");
    let mut tables = vec![project_table.clone()];
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
            if form_atom(lib, "type", 1) == Some("Legacy") {
                if table != project_table
                    && !libraries.contains_key(name)
                    && !notes.legacy.staged.contains_key(name)
                {
                    notes.legacy.protected.insert(name.to_string());
                    notes.legacy.replacements.remove(name);
                }
                if !path.is_file() {
                    notes
                        .absent
                        .entry(name.to_string())
                        .or_insert_with(|| path.display().to_string());
                }
                continue;
            }
            let uses_kicad = || form_atom(lib, "type", 1) == Some("KiCad");
            if path.is_dir() {
                libraries.entry(name.to_string()).or_insert_with(|| {
                    notes
                        .table_uses_kicad
                        .insert(name.to_string(), uses_kicad());
                    path
                });
            } else if !libraries.contains_key(name) {
                match relocated_library(&path, project) {
                    Some(found) => {
                        notes.relocated.push(format!(
                            "{name}: the fp-lib-table's {} is not here; used {}",
                            path.display(),
                            found.display()
                        ));
                        notes.absent.remove(name);
                        notes
                            .table_uses_kicad
                            .insert(name.to_string(), uses_kicad());
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
    for (name, path) in &notes.legacy.staged {
        libraries.insert(name.clone(), path.clone());
        notes.table_uses_kicad.insert(name.clone(), true);
    }
    libraries.retain(|name, _| !notes.legacy.protected.contains(name));
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

/// Publish donor models in the project. A recovered model under an existing
/// nickname joins the staged library or a copy of that library's full model
/// inventory, so KiCad can still resolve the other names under the nickname.
pub(super) fn write_local_libraries(
    output: &Path,
    footprints: &BTreeMap<String, Expr>,
    libraries: &BTreeMap<String, PathBuf>,
    notes: &LibraryNotes,
    board: Option<&Path>,
) -> Result<(), String> {
    let mut additions = BTreeMap::<&str, Vec<(&str, &Expr)>>::new();
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
        additions
            .entry(nickname)
            .or_default()
            .push((name, footprint));
    }
    let output = fs::canonicalize(output).map_err(|error| error.to_string())?;
    let table = output.join("fp-lib-table");
    if fs::symlink_metadata(&table).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(crate::exit::input_error(format!(
            "cannot write donor footprints while preserving {}: import into a separate output project with its own footprint table",
            table.display()
        )));
    }
    let original = read_optional_table(&table)?;
    let mut parsed = parse(original.as_deref().unwrap_or("(fp_lib_table (version 7))"))?;
    if parsed.head() != Some("fp_lib_table") {
        return Err(crate::exit::input_error(format!(
            "{} is not a footprint library table",
            table.display()
        )));
    }
    let donor = board
        .and_then(|board| board.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Expr::List(items) = &mut parsed else {
        unreachable!()
    };
    let mut changed = false;
    for (nickname, models) in additions {
        let is_entry = |item: &Expr| {
            item.head() == Some("lib") && form_atom(item, "name", 1) == Some(nickname)
        };
        let mut entries = items.iter_mut().filter(|item| is_entry(item));
        let existing = entries.next();
        let replace_legacy = board.is_some()
            && notes
                .legacy
                .replacements
                .get(nickname)
                .is_some_and(|legacy| legacy.still_missing(nickname, existing.as_deref()));
        if entries.next().is_some()
            || existing.as_ref().is_some_and(|entry| {
                form_atom(entry, "type", 1) != Some("KiCad") && !replace_legacy
            })
            || notes.legacy.replacements.contains_key(nickname) && !replace_legacy
        {
            return Err(crate::exit::input_error(format!(
                "cannot safely extend library {nickname:?}: {} must name one KiCad library with that nickname",
                table.display()
            )));
        }
        let source = libraries.get(nickname).map(PathBuf::as_path);
        let staged = existing
            .as_ref()
            .filter(|_| !replace_legacy)
            .and_then(|entry| form_atom(entry, "uri", 1))
            .and_then(|uri| {
                uri.strip_prefix("${KIPRJMOD}/")
                    .or_else(|| uri.strip_prefix("$(KIPRJMOD)/"))
            })
            .and_then(|uri| staged_library(&output.join(uri), &output));
        let folder = staged
            .as_ref()
            .cloned()
            .or_else(|| {
                source
                    .filter(|_| !replace_legacy)
                    .and_then(|source| staged_library(source, &output))
            })
            .map_or_else(|| new_local_library(&output, nickname), Ok)?;
        if let Some(source) = source {
            copy_missing_models(source, &folder)?;
        }
        for (name, model) in models {
            write_donor_model(&folder, name, model)?;
        }
        if staged.is_some() {
            continue;
        }
        changed = true;
        let relative = folder.strip_prefix(&output).expect("output library folder");
        let relative: Vec<_> = relative.iter().map(|part| part.to_string_lossy()).collect();
        let uri = quote(&format!("${{KIPRJMOD}}/{}", relative.join("/")));
        let uri = parse(&format!("(uri {uri})"))?;
        if let Some(Expr::List(entry)) = existing {
            entry.retain(|item| {
                item.head() != Some("uri") && !(replace_legacy && item.head() == Some("type"))
            });
            if replace_legacy {
                entry.push(parse(r#"(type "KiCad")"#)?);
            }
            entry.push(uri);
        } else {
            let mut entry = parse(&format!(
                "(lib (name {}) (type \"KiCad\") (options \"\") (descr {}))",
                requote(nickname),
                quote(&format!("footprints from the board {donor}"))
            ))?;
            if let Expr::List(parts) = &mut entry {
                parts.push(uri);
            }
            items.push(entry);
        }
    }
    if !changed {
        return Ok(());
    }
    if read_optional_table(&table)? != original {
        return Err(crate::exit::input_error(format!(
            "{} changed during import; keep its current entries and retry",
            table.display()
        )));
    }
    write_atomically(&table, format!("{}\n", encode(&parsed)).as_bytes())
}

pub(super) fn read_optional_table(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("failed to read {}: {error}", path.display())),
    }
}

pub(super) fn staged_library(path: &Path, output: &Path) -> Option<PathBuf> {
    let path = fs::canonicalize(path).ok()?;
    (path != output && path.starts_with(output) && path.is_dir()).then_some(path)
}

pub(super) fn new_local_library(output: &Path, nickname: &str) -> Result<PathBuf, String> {
    for index in 1.. {
        let stem = if index == 1 {
            nickname.into()
        } else {
            format!("{nickname}-{index}")
        };
        let path = output.join(format!("{stem}.pretty"));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => (),
            Err(error) => return Err(format!("failed to create {}: {error}", path.display())),
        }
    }
    unreachable!()
}

fn copy_missing_models(source: &Path, folder: &Path) -> Result<(), String> {
    if fs::canonicalize(source).is_ok_and(|path| path == folder) {
        return Ok(());
    }
    for entry in fs::read_dir(source).map_err(|error| format!("{}: {error}", source.display()))? {
        let entry = entry.map_err(|error| format!("{}: {error}", source.display()))?;
        let path = entry.path();
        if !path.extension().is_some_and(|ext| ext == "kicad_mod") || !path.is_file() {
            continue;
        }
        let contents = fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let target = folder.join(entry.file_name());
        write_new_model(&target, &contents, |existing| existing == contents)?;
    }
    Ok(())
}

fn write_donor_model(folder: &Path, name: &str, footprint: &Expr) -> Result<(), String> {
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
    write_new_model(
        &path,
        format!("{}\n", encode(&file)).as_bytes(),
        |existing| {
            std::str::from_utf8(existing)
                .ok()
                .and_then(|text| parse(text).ok())
                .is_some_and(|model| same_library_model(&model, &file))
        },
    )
}

pub(super) fn write_new_model(
    path: &Path,
    contents: &[u8],
    matches: impl FnOnce(&[u8]) -> bool,
) -> Result<(), String> {
    let mut file = match fs::File::options().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            if fs::read(path).is_ok_and(|existing| matches(&existing)) {
                return Ok(());
            }
            return Err(crate::exit::input_error(format!(
                "a different model already exists at {}",
                path.display()
            )));
        }
        Err(error) => return Err(format!("failed to create {}: {error}", path.display())),
    };
    if let Err(error) = file.write_all(contents) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(format!("failed to write {}: {error}", path.display()));
    }
    Ok(())
}

/// Add-on libraries a schematic may name that KiCad does not ship, with
/// where to get them (docs/agents.md, "From a schematic").
pub(super) const ADD_ON_LIBRARIES: [(&str, &str); 1] = [(
    "Espressif",
    "Espressif's library: KiCad's Plugin and Content Manager, \"Espressif KiCad Library\" (installed as PCM_Espressif), or github.com/espressif/kicad-libraries",
)];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_model_reuses_the_staged_library_or_copies_its_full_inventory() {
        let root = temporary_next_to(&std::env::temp_dir().join("footprint-library-overlay"));
        let source = root.join("input/Stock.pretty");
        fs::create_dir_all(&source).unwrap();
        let existing = r#"(footprint "Current" (layer "F.Cu"))"#;
        fs::write(source.join("Current.kicad_mod"), existing).unwrap();
        let libraries = BTreeMap::from([("Stock".into(), source.clone())]);
        let models = BTreeMap::from([(
            "Stock:Historic".into(),
            parse(r#"(footprint "Historic" (layer "F.Cu"))"#).unwrap(),
        )]);
        for (case, relative) in [
            ("external", "Stock.pretty"),
            ("project", "libs/Stock.pretty"),
        ] {
            let output = root.join(case);
            let staged = output.join(relative);
            let uri = if case == "project" {
                fs::create_dir_all(&staged).unwrap();
                fs::write(staged.join("Current.kicad_mod"), existing).unwrap();
                format!("${{KIPRJMOD}}/{relative}")
            } else {
                fs::create_dir_all(&output).unwrap();
                source.display().to_string()
            };
            let table = output.join("fp-lib-table");
            fs::write(
                &table,
                format!(
                    r#"(fp_lib_table (version 7)
                (lib (name "Stock") (type "KiCad") (uri "{uri}") (options "") (descr "existing")))"#
                ),
            )
            .unwrap();
            write_local_libraries(&output, &models, &libraries, &LibraryNotes::default(), None)
                .unwrap();
            write_local_libraries(&output, &models, &libraries, &LibraryNotes::default(), None)
                .unwrap();
            let table = fs::read_to_string(table).unwrap();
            assert!(table.contains(&format!("${{KIPRJMOD}}/{relative}")));
            assert!(table.contains("(descr \"existing\")"));
            assert_eq!(table.matches("(name \"Stock\")").count(), 1);
            assert_eq!(
                fs::read_to_string(staged.join("Current.kicad_mod")).unwrap(),
                existing
            );
            assert!(staged.join("Historic.kicad_mod").is_file());
        }
        assert!(!source.join("Historic.kicad_mod").exists());
        assert_eq!(
            fs::read_to_string(source.join("Current.kicad_mod")).unwrap(),
            existing
        );
        fs::remove_dir_all(root).unwrap();
    }
}
