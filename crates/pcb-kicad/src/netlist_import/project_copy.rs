// Copyright (C) 2026 Toit contributors.

//! The schematic's side of an import: the netlist kicad-cli exports, the
//! project files and libraries copied next to the board, and the footprint
//! names the copied sheets are brought to.

use super::*;

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
    copy_project_files(input, project, output, board_id)?;
    copy_nested_sheets(project, output)?;
    copy_table_libraries(project, output)?;
    Ok(path)
}

/// The project travels with the board: schematic sheets (the root
/// renamed after the board), project settings, library tables.
fn copy_project_files(
    input: &Path,
    project: &Path,
    output: &Path,
    board_id: &str,
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
        fs::copy(&path, output.join(target)).map_err(|error| error.to_string())?;
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
            copy_library_tree(
                &project.join(relative),
                &output.join(relative),
                project,
                output,
            )?;
        }
    }
    Ok(())
}

/// For each footprint id the libraries know under another name, the new name
/// most symbols got: the copied schematic names a footprint id once for all
/// its symbols.
pub(super) fn most_common_renames(
    renamed: &BTreeMap<(String, String), usize>,
) -> BTreeMap<String, String> {
    let mut most: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for ((old, new), count) in renamed {
        let entry = most.entry(old.clone()).or_insert((0, String::new()));
        if *count > entry.0 {
            *entry = (*count, new.clone());
        }
    }
    most.into_iter().map(|(old, (_, new))| (old, new)).collect()
}

/// The copied schematic links the footprints the board has, so the new
/// project checks clean (the original is left as it is).
pub(super) fn rename_footprints_in_sheets(
    output: &Path,
    renamed: &BTreeMap<String, String>,
) -> Result<(), String> {
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
                for (old, new) in renamed {
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
                    fs::write(&path, changed)
                        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
                }
            }
        }
    }
    Ok(())
}
