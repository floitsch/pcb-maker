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
