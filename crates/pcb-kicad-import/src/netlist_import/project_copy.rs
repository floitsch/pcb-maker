// Copyright (C) 2026 Toit contributors.

//! Export the schematic netlist and copy project files and libraries next to
//! the imported board.

use super::*;
use std::io::{ErrorKind, Write};
use std::path::Component;

/// Exports the netlist of the schematic `input` to `<output>/<board_id>.net`
/// with kicad-cli and copies the project next to it.
pub(super) fn export_schematic(
    input: &Path,
    output: &Path,
    board_id: &str,
    project: &Path,
) -> Result<PathBuf, String> {
    let path = output.join(format!("{board_id}.net"));
    let mut command = Command::new(crate::native_report::kicad_cli());
    command
        .args(["sch", "export", "netlist", "--format", "kicadsexpr", "-o"])
        .arg(&path)
        .arg(input);
    let result = crate::ledger::kicad_check(|| command.output()).map_err(|error| {
        crate::exit::input_error(format!(
            "kicad-cli did not run ({error}): {}",
            crate::KICAD_REQUIREMENT
        ))
    })?;
    // The schematic KiCad cannot read is the input's fault (exit 2).
    if !result.status.success() {
        return Err(crate::exit::input_error(format!(
            "kicad-cli sch export netlist failed on {}: {}",
            input.display(),
            String::from_utf8_lossy(&result.stderr)
        )));
    }
    let same_project = fs::canonicalize(project)
        .ok()
        .is_some_and(|source| fs::canonicalize(output).is_ok_and(|target| source == target));
    copy_project_files(input, project, output, board_id, same_project)?;
    if !same_project {
        copy_nested_sheets(project, output)?;
        copy_table_libraries(project, output)?;
        package_external_footprint_libraries(project, output)?;
    }
    Ok(path)
}

/// Package authored parent-relative native libraries as local model inventories.
/// A distinct library already available at the rebased URI belongs to the
/// output project. Only the copied source entry may be repointed.
fn package_external_footprint_libraries(project: &Path, output: &Path) -> Result<(), String> {
    let (Ok(project), Ok(output)) = (fs::canonicalize(project), fs::canonicalize(output)) else {
        return Ok(());
    };
    let table = output.join("fp-lib-table");
    if fs::symlink_metadata(&table).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Ok(());
    }
    let Some(original) = read_optional_table(&table)? else {
        return Ok(());
    };
    let Ok(mut copied @ Expr::List(_)) = parse(&original) else {
        return Ok(());
    };
    if copied.head() != Some("fp_lib_table") {
        return Ok(());
    }
    let Ok(source) = fs::read_to_string(project.join("fp-lib-table"))
        .map_err(|error| error.to_string())
        .and_then(|text| parse(&text))
    else {
        return Ok(());
    };
    if source.head() != Some("fp_lib_table") {
        return Ok(());
    }
    let mut names = BTreeMap::new();
    for entry in copied.children() {
        if let Some(name) = form_atom(entry, "name", 1).filter(|_| entry.head() == Some("lib")) {
            *names.entry(name.to_owned()).or_insert(0) += 1;
        }
    }
    let Expr::List(items) = &mut copied else {
        unreachable!()
    };
    let mut changed = false;
    for entry in items.iter_mut().filter(|entry| entry.head() == Some("lib")) {
        let (Some(nickname), Some(uri)) = (form_atom(entry, "name", 1), form_atom(entry, "uri", 1))
        else {
            continue;
        };
        if names[nickname] != 1
            || file_name_error(nickname).is_some()
            || form_atom(entry, "type", 1) != Some("KiCad")
        {
            continue;
        }
        let Some(relative) = uri
            .strip_prefix("${KIPRJMOD}/")
            .or_else(|| uri.strip_prefix("$(KIPRJMOD)/"))
        else {
            continue;
        };
        if relative.contains('$') {
            continue;
        }
        let relative = Path::new(relative);
        if relative.is_absolute()
            || !relative
                .components()
                .any(|part| part == Component::ParentDir)
            || relative.extension().is_none_or(|ext| ext != "pretty")
        {
            continue;
        }
        let mut authored = source.children().iter().filter(|item| {
            item.head() == Some("lib") && form_atom(item, "name", 1) == Some(nickname)
        });
        if authored.next() != Some(entry) || authored.next().is_some() {
            continue;
        }
        let authored_folder = project.join(relative);
        let Ok(folder) = fs::canonicalize(&authored_folder) else {
            continue;
        };
        if folder.starts_with(&project)
            || folder.starts_with(&output)
            || output.starts_with(&folder)
            || !fs::symlink_metadata(&authored_folder).is_ok_and(|meta| meta.is_dir())
        {
            continue;
        }
        match fs::canonicalize(output.join(relative)) {
            Ok(rebased) if rebased == folder => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            _ => continue,
        }
        let Some(models) = native_model_inventory(&folder) else {
            continue;
        };
        let destination = new_local_library(&output, nickname)?;
        for (file, bytes) in models {
            write_new_model(&destination.join(file), &bytes, |existing| {
                existing == bytes
            })?;
        }
        let folder_name = destination.file_name().expect("local library folder");
        let uri = quote(&format!("${{KIPRJMOD}}/{}", folder_name.to_string_lossy()));
        let Expr::List(parts) = entry else {
            unreachable!()
        };
        parts.retain(|item| item.head() != Some("uri"));
        parts.push(parse(&format!("(uri {uri})"))?);
        changed = true;
    }
    if changed {
        if read_optional_table(&table)?.as_deref() != Some(&original) {
            return Err(crate::exit::input_error(format!(
                "{} changed during import. Keep its current entries and retry",
                table.display()
            )));
        }
        write_atomically(&table, format!("{}\n", encode(&copied)).as_bytes())?;
    }
    Ok(())
}

fn native_model_inventory(folder: &Path) -> Option<Vec<(std::ffi::OsString, Vec<u8>)>> {
    let mut models = Vec::new();
    for item in fs::read_dir(folder).ok()? {
        let item = item.ok()?;
        let path = item.path();
        if path.extension().is_none_or(|ext| ext != "kicad_mod") {
            continue;
        }
        if !item.file_type().ok()?.is_file() {
            return None;
        }
        models.push((item.file_name(), fs::read(path).ok()?));
    }
    (!models.is_empty()).then_some(models)
}

/// Keep rules already supplied for the output board. Otherwise take the
/// input's companion file, or the originating schematic's rules for a
/// netlist exported elsewhere. KiCad selects the file by the board stem.
pub(super) fn copy_custom_rules(
    input: &Path,
    output: &Path,
    board_id: &str,
    project: &Path,
    netlist: Option<&Expr>,
) -> Result<(), String> {
    let target = output.join(format!("{board_id}.kicad_dru"));
    let source_schematic = netlist
        .and_then(|netlist| netlist.child("design"))
        .and_then(|design| text_of(design, "source"))
        .map(PathBuf::from)
        .filter(|source| {
            source
                .extension()
                .is_some_and(|extension| extension == "kicad_sch" || extension == "sch")
        });
    let companions = std::iter::once(input.with_extension("kicad_dru"))
        .chain(
            source_schematic
                .iter()
                .map(|source| project.join(source).with_extension("kicad_dru")),
        )
        .chain(
            source_schematic
                .as_deref()
                .and_then(Path::file_name)
                .map(|name| project.join(name).with_extension("kicad_dru")),
        );
    copy_first_companion(companions, &target)
}

/// Use a netlist's explicit sibling project before measuring footprint
/// clearances. Its embedded schematic path does not select the project.
pub(super) fn copy_netlist_project(
    input: &Path,
    output: &Path,
    board_id: &str,
) -> Result<(), String> {
    copy_first_companion(
        std::iter::once(input.with_extension("kicad_pro")),
        &output.join(format!("{board_id}.kicad_pro")),
    )
}

fn copy_first_companion(
    sources: impl IntoIterator<Item = PathBuf>,
    target: &Path,
) -> Result<(), String> {
    if target.symlink_metadata().is_ok() {
        return Ok(());
    }
    for source in sources {
        let contents = match fs::read(&source) {
            Ok(contents) => contents,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("failed to read {}: {error}", source.display())),
        };
        let mut file = match fs::File::options()
            .write(true)
            .create_new(true)
            .open(target)
        {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => return Ok(()),
            Err(error) => return Err(format!("failed to create {}: {error}", target.display())),
        };
        if let Err(error) = file.write_all(&contents) {
            drop(file);
            let _ = fs::remove_file(target);
            return Err(format!("failed to write {}: {error}", target.display()));
        }
        break;
    }
    Ok(())
}

/// The project travels with the board: schematic sheets (the root
/// renamed after the board), project settings, library tables.
fn copy_project_files(
    input: &Path,
    project: &Path,
    output: &Path,
    board_id: &str,
    same_project: bool,
) -> Result<(), String> {
    let stem = input
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("");
    for entry in fs::read_dir(project)
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
        } else if name.ends_with(".kicad_sch") || name == "sym-lib-table" || name == "fp-lib-table"
        {
            name
        } else {
            continue;
        };
        if !same_project || std::ffi::OsStr::new(&target) != path.file_name().unwrap_or_default() {
            if target == "fp-lib-table" {
                copy_first_companion(std::iter::once(path), &output.join(target))?;
            } else {
                fs::copy(&path, output.join(target)).map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(())
}

/// Sheets in subfolders (`sch/power.kicad_sch`), where the root expects
/// them.
fn copy_nested_sheets(project: &Path, output: &Path) -> Result<(), String> {
    let skip = fs::canonicalize(output).ok();
    let mut folders: Vec<(PathBuf, usize)> = fs::read_dir(project)
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
                && let Ok(relative) = path.strip_prefix(project)
            {
                copy_library_tree(&path, &output.join(relative), project, output)?;
            }
        }
    }
    Ok(())
}

/// And the project's own libraries its tables name.
fn copy_table_libraries(project: &Path, output: &Path) -> Result<(), String> {
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
            let destination = output.join(relative);
            if table == "fp-lib-table" && form_atom(lib, "type", 1) == Some("Legacy") {
                copy_legacy_library(&project.join(relative), &destination, project, output)?;
            } else {
                copy_library_tree(&project.join(relative), &destination, project, output)?;
            }
        }
    }
    Ok(())
}

fn copy_legacy_library(
    source: &Path,
    target: &Path,
    project: &Path,
    output: &Path,
) -> Result<(), String> {
    let (Ok(source), Ok(project), Ok(output)) = (
        fs::canonicalize(source),
        fs::canonicalize(project),
        fs::canonicalize(output),
    ) else {
        return Ok(());
    };
    let Some(parent) = target.parent() else {
        return Ok(());
    };
    if !source.starts_with(project)
        || !source.is_file()
        || !parent
            .ancestors()
            .find_map(|ancestor| fs::canonicalize(ancestor).ok())
            .is_some_and(|ancestor| ancestor.starts_with(output))
    {
        return Ok(());
    }
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    copy_first_companion(std::iter::once(source), target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_relative_pretty_packages_the_inventory_and_keeps_existing_output() {
        let root = temporary_next_to(&std::env::temp_dir().join("portable-footprints"));
        let project = root.join("original/project");
        let library = root.join("original/library.pretty");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir(&library).unwrap();
        let model = r#"(footprint "F" (layer "F.Cu"))"#;
        for file in ["Model.kicad_mod", "Spare.kicad_mod"] {
            fs::write(library.join(file), model).unwrap();
        }
        let table = r#"(fp_lib_table (version 7) (lib (name "Sensor") (type "KiCad")
            (uri "${KIPRJMOD}/../library.pretty") (options "KiCad") (descr "source inventory")))"#;
        fs::write(project.join("fp-lib-table"), table).unwrap();
        let output = root.join("packaged/project");
        let existing = output.join("Sensor.pretty");
        fs::create_dir_all(&existing).unwrap();
        fs::write(existing.join("Model.kicad_mod"), "user model").unwrap();
        fs::write(output.join("fp-lib-table"), table).unwrap();
        package_external_footprint_libraries(&project, &output).unwrap();
        let portable = fs::read_to_string(output.join("fp-lib-table")).unwrap();
        assert_eq!(portable.matches(r#"(name "Sensor")"#).count(), 1);
        assert!(portable.contains("${KIPRJMOD}/Sensor-2.pretty"));
        assert!(portable.contains(r#"(descr "source inventory")"#));
        assert!(portable.contains(r#"(options "KiCad")"#));
        let copied = output.join("Sensor-2.pretty");
        for file in ["Model.kicad_mod", "Spare.kicad_mod"] {
            assert_eq!(fs::read_to_string(copied.join(file)).unwrap(), model);
        }
        assert_eq!(
            fs::read_to_string(existing.join("Model.kicad_mod")).unwrap(),
            "user model"
        );
        assert_eq!(
            fs::read_to_string(project.join("fp-lib-table")).unwrap(),
            table
        );
        package_external_footprint_libraries(&project, &output).unwrap();
        assert_eq!(
            fs::read_to_string(output.join("fp-lib-table")).unwrap(),
            portable
        );
        assert!(!output.join("Sensor-3.pretty").exists());

        let collision = root.join("conflict/project");
        let caller_library = root.join("conflict/library.pretty");
        fs::create_dir_all(&collision).unwrap();
        fs::create_dir(&caller_library).unwrap();
        fs::write(caller_library.join("Model.kicad_mod"), "other inventory").unwrap();
        fs::write(collision.join("fp-lib-table"), table).unwrap();
        package_external_footprint_libraries(&project, &collision).unwrap();
        assert_eq!(
            fs::read_to_string(collision.join("fp-lib-table")).unwrap(),
            table
        );
        assert_eq!(
            fs::read_to_string(caller_library.join("Model.kicad_mod")).unwrap(),
            "other inventory"
        );
        assert!(!collision.join("Sensor.pretty").exists());

        #[cfg(unix)]
        {
            let output = root.join("linked/project");
            fs::create_dir_all(&output).unwrap();
            fs::write(output.join("fp-lib-table"), table).unwrap();
            std::os::unix::fs::symlink(
                caller_library.join("Model.kicad_mod"),
                library.join("Forbidden.kicad_mod"),
            )
            .unwrap();
            package_external_footprint_libraries(&project, &output).unwrap();
            assert_eq!(
                fs::read_to_string(output.join("fp-lib-table")).unwrap(),
                table
            );
            assert!(!output.join("Sensor.pretty").exists());
        }
        fs::remove_dir_all(root).unwrap();
    }
}
