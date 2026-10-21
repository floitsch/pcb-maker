// Copyright (C) 2026 Toit contributors.

//! Files and directories: hashes, JSON, tree copies.

use super::*;

pub(crate) fn file_sha256(path: &Path) -> Result<String, String> {
    let contents =
        fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(contents)))
}

pub(crate) fn write_typed_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    fs::write(path, format!("{serialized}\n"))
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

pub(crate) fn read_typed_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

/// Why `part` cannot be one file name inside a directory, if it cannot:
/// empty, `.` or `..`, a path separator or NUL in it, or a leading `~` or
/// `$` (a home directory or a variable to a shell or to KiCad's paths).
/// Names from files and arguments (a board id, a footprint library's
/// nickname, a footprint's name) pass this before they are joined to a
/// directory, so that a hostile board cannot make a path outside it.
pub(crate) fn file_name_error(part: &str) -> Option<&'static str> {
    if part.is_empty() {
        Some("is empty")
    } else if part == "." || part == ".." {
        Some("names a directory")
    } else if part.contains(['/', '\\', '\0']) {
        Some("holds a path separator")
    } else if part.starts_with(['~', '$']) {
        Some("starts with ~ or $")
    } else {
        None
    }
}

/// The input's error when `board_id` is not a file stem: every command
/// joins `<board-id>.kicad_pcb` (and `.kicad_sch`, `.kicad_pro`) to a
/// directory it was given, and a board id of `../../x` would read and
/// write beside it.
pub fn check_board_id(board_id: &str) -> Result<(), String> {
    match file_name_error(board_id) {
        None => Ok(()),
        Some(why) => Err(crate::verdict::input_error(format!(
            "the board id {board_id:?} {why}: give the board file's stem (<board-id>.kicad_pcb in the project directory)"
        ))),
    }
}

/// How deep `copy_library_tree` goes: a library is a folder of files
/// (`.pretty`, `.3dshapes`) or one file.
const LIBRARY_DEPTH: usize = 8;

/// Copies a project's library (`from`, a folder or a file) to `to`, for
/// an import's output project (`output`). As `copy_directory_tree` does,
/// a directory that holds the output is not copied (the project into its
/// own `out`), and links are not followed: a link in a library (`lib ->
/// .`) is left out, with a note. `from` itself (a library the project's
/// fp-lib-table names) must lie in `project`, links resolved, and must not
/// hold `output` (`${KIPRJMOD}/.`): otherwise it is not copied, with a
/// note (the board does not need it: its footprints are in it). Until
/// 2026-10-21 such a table copied the project into its own output, again
/// and again, until the path grew too long.
pub(crate) fn copy_library_tree(
    from: &Path,
    to: &Path,
    project: &Path,
    output: &Path,
) -> Result<(), String> {
    let (Ok(source), Ok(project), Ok(output)) = (
        fs::canonicalize(from),
        fs::canonicalize(project),
        fs::canonicalize(output),
    ) else {
        // Not there (a table's entry for a library never made): nothing
        // to copy, as before.
        return Ok(());
    };
    if !source.starts_with(&project) {
        eprintln!(
            "import: the library {} lies outside the project {} (a link?): not copied",
            from.display(),
            project.display()
        );
        return Ok(());
    }
    if output.starts_with(&source) {
        eprintln!(
            "import: the library {} holds the output directory {}: not copied into itself",
            from.display(),
            output.display()
        );
        return Ok(());
    }
    copy_library_entries(&source, to, &output, 0)
}

fn copy_library_entries(from: &Path, to: &Path, output: &Path, depth: usize) -> Result<(), String> {
    let kind = fs::symlink_metadata(from)
        .map_err(|error| format!("failed to read {}: {error}", from.display()))?
        .file_type();
    if kind.is_file() {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        fs::copy(from, to)
            .map_err(|error| format!("failed to copy {}: {error}", from.display()))?;
        return Ok(());
    }
    if !kind.is_dir() {
        eprintln!(
            "import: {} is not a file or a folder (a link?): not copied",
            from.display()
        );
        return Ok(());
    }
    if depth >= LIBRARY_DEPTH {
        eprintln!(
            "import: {} is more than {LIBRARY_DEPTH} folders deep in its library: not copied",
            from.display()
        );
        return Ok(());
    }
    if fs::canonicalize(from).is_ok_and(|path| output.starts_with(path)) {
        return Ok(());
    }
    fs::create_dir_all(to)
        .map_err(|error| format!("failed to create {}: {error}", to.display()))?;
    for entry in fs::read_dir(from)
        .map_err(|error| format!("failed to read {}: {error}", from.display()))?
        .flatten()
    {
        copy_library_entries(
            &entry.path(),
            &to.join(entry.file_name()),
            output,
            depth + 1,
        )?;
    }
    Ok(())
}

pub(crate) fn copy_directory_tree(source: &Path, destination: &Path) -> Result<(), String> {
    if destination.exists() {
        return Err(format!(
            "refusing to overwrite KiCad action artifact {}",
            destination.display()
        ));
    }
    fs::create_dir(destination).map_err(|error| {
        format!(
            "failed to create KiCad action artifact {}: {error}",
            destination.display()
        )
    })?;
    // An output directory inside the project is not copied into itself.
    let root = fs::canonicalize(destination).unwrap_or_else(|_| destination.to_path_buf());
    copy_directory_entries(source, destination, &root)
}

fn copy_directory_entries(source: &Path, destination: &Path, root: &Path) -> Result<(), String> {
    for entry in fs::read_dir(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "failed to inspect KiCad artifact {}: {error}",
                entry.path().display()
            )
        })?;
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            if fs::canonicalize(entry.path()).is_ok_and(|path| root.starts_with(path)) {
                continue;
            }
            fs::create_dir(&target).map_err(|error| {
                format!(
                    "failed to create KiCad action artifact {}: {error}",
                    target.display()
                )
            })?;
            copy_directory_entries(&entry.path(), &target, root)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target).map_err(|error| {
                format!(
                    "failed to copy KiCad artifact {} to {}: {error}",
                    entry.path().display(),
                    target.display()
                )
            })?;
        } else {
            return Err(format!(
                "KiCad reference-action source contains unsupported non-file artifact {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

pub(crate) fn read_json(path: &Path) -> Result<serde_json::Value, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_joined_to_a_directory_stays_in_it() {
        for name in ["board", "my board.v2", "a..b", "R_0603"] {
            assert_eq!(file_name_error(name), None, "{name}");
            assert!(check_board_id(name).is_ok());
        }
        for name in ["", ".", "..", "../x", "a/b", "a\\b", "~x", "$HOME", "x\0y"] {
            assert!(file_name_error(name).is_some(), "{name:?}");
            let error = check_board_id(name).unwrap_err();
            assert!(crate::verdict::is_input_error(&error), "{error}");
        }
    }
}
