// Copyright (C) 2026 Toit contributors.

//! A KiCad configuration without libraries, for kicad-cli's DRC.
//!
//! KiCad 10's `pcb drc` loads every footprint library of the global
//! `fp-lib-table` before it starts (`PCBNEW_JOBS_HANDLER::JobExportDrc`:
//! `AsyncLoad`, `BlockUntilLoaded`), for the library parity test alone:
//! about 2.7 s, 9 CPU s and 2 GB per call on any board (docs/handover.md,
//! "KiCad check cost"). Its `lib_footprint_*` findings are warnings that
//! pcb-maker never counts. The DRC therefore runs with `KICAD_CONFIG_HOME`
//! pointing at a pcb-maker-owned directory whose `fp-lib-table` and
//! `sym-lib-table` are empty: `<cache root>/kicad-config/<major.minor>/`,
//! the cache root as for the DRC cache (`PCB_MAKER_CACHE_DIR`, else
//! `$XDG_CACHE_HOME/pcb-maker`, else `~/.cache/pcb-maker`; also when the
//! cache itself is off). Every other finding is the same per type and
//! severity.
//!
//! The user's configuration is kept when `PCB_MAKER_KICAD_USER_LIBS=1`, when
//! the board's project sets a `lib_footprint_*` severity to `error` (its
//! library findings would then count), when the project file cannot be
//! read as JSON, and when the directory cannot be prepared. ERC keeps the
//! user's configuration: with empty tables `lib_symbol_issues`,
//! `lib_symbol_mismatch` and `footprint_link_issues` change (warnings, but
//! `erc_violations` counts every ERC finding; docs/handover.md, "KiCad
//! check cost").

use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

/// Set to anything but `0` to run kicad-cli on the user's own libraries.
pub(crate) const USER_LIBS_ENV: &str = "PCB_MAKER_KICAD_USER_LIBS";

const FOOTPRINT_TABLE: &str = "(fp_lib_table\n\t(version 7)\n)\n";
const SYMBOL_TABLE: &str = "(sym_lib_table\n\t(version 7)\n)\n";

fn variable(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name).filter(|value| !value.is_empty())
}

/// pcb-maker's cache root, whether or not the DRC cache is on.
pub(crate) fn cache_root_from_env() -> Option<PathBuf> {
    variable("PCB_MAKER_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| variable("XDG_CACHE_HOME").map(|base| PathBuf::from(base).join("pcb-maker")))
        .or_else(|| variable("HOME").map(|home| PathBuf::from(home).join(".cache/pcb-maker")))
}

fn warn(message: &str) {
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !WARNED.swap(true, Ordering::Relaxed) {
        eprintln!("KiCad configuration without libraries skipped: {message}");
    }
}

/// `KICAD_CONFIG_HOME` for a DRC of `input` by `program`, or `None` to keep
/// the user's configuration.
pub(crate) fn drc_config_home(program: &OsStr, input: &Path) -> Option<PathBuf> {
    if variable(USER_LIBS_ENV).is_some_and(|value| value != "0") {
        return None;
    }
    if project_counts_library_findings(input) {
        return None;
    }
    let root = cache_root_from_env()?;
    let Some(version) = crate::drc_cache::tool_version(program) else {
        warn("kicad-cli version unknown");
        return None;
    };
    match prepare(&root.join("kicad-config"), &version) {
        Ok(home) => Some(home),
        Err(error) => {
            warn(&error);
            None
        }
    }
}

/// Points `command`, a kicad-cli DRC of `input`, at the configuration
/// without libraries when [`drc_config_home`] gives one.
pub(crate) fn configure_drc(command: &mut std::process::Command, program: &OsStr, input: &Path) {
    if let Some(home) = drc_config_home(program, input) {
        command.env("KICAD_CONFIG_HOME", home);
    }
}

/// Whether the project next to `input` sets a `lib_footprint_*` severity
/// to `error`, or cannot be read (both keep the user's libraries). No
/// project file is KiCad's defaults: warnings.
fn project_counts_library_findings(input: &Path) -> bool {
    let project = input.with_extension("kicad_pro");
    let Ok(text) = fs::read_to_string(&project) else {
        return project.exists();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return true;
    };
    value["board"]["design_settings"]["rule_severities"]
        .as_object()
        .is_some_and(|severities| {
            severities.iter().any(|(name, severity)| name.starts_with("lib_footprint") && severity == "error")
        })
}

/// KiCad's settings directory name for `version` (`10.0.7` -> `10.0`).
fn series(version: &str) -> Option<String> {
    let mut parts = version.trim().split(|c: char| !c.is_ascii_digit());
    let major = parts.next().filter(|part| !part.is_empty())?;
    let minor = parts.next().filter(|part| !part.is_empty())?;
    Some(format!("{major}.{minor}"))
}

/// Creates `home/<major.minor>/` with empty library tables (rewritten when
/// they differ) and returns `home`. Safe against concurrent processes:
/// tables are written to a temporary file and renamed.
pub(crate) fn prepare(home: &Path, version: &str) -> Result<PathBuf, String> {
    let series = series(version).ok_or_else(|| format!("unexpected kicad-cli version {version:?}"))?;
    let directory = home.join(series);
    fs::create_dir_all(&directory).map_err(|error| format!("failed to create {}: {error}", directory.display()))?;
    for (name, contents) in [("fp-lib-table", FOOTPRINT_TABLE), ("sym-lib-table", SYMBOL_TABLE)] {
        let path = directory.join(name);
        if fs::read(&path).ok().as_deref() != Some(contents.as_bytes()) {
            crate::drc_cache::write_atomically(&path, contents.as_bytes())?;
        }
    }
    Ok(home.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_is_major_and_minor() {
        assert_eq!(series("10.0.7").as_deref(), Some("10.0"));
        assert_eq!(series("9.0.1-rc2\n").as_deref(), Some("9.0"));
        assert_eq!(series("10").as_deref(), None);
        assert_eq!(series("unknown").as_deref(), None);
    }

    #[test]
    fn prepare_writes_empty_tables_and_repairs_them() {
        let home = std::env::temp_dir().join(format!("pcb-maker-kicad-config-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        assert_eq!(prepare(&home, "10.0.7").unwrap(), home);
        let table = home.join("10.0/fp-lib-table");
        assert_eq!(fs::read_to_string(&table).unwrap(), FOOTPRINT_TABLE);
        assert_eq!(fs::read_to_string(home.join("10.0/sym-lib-table")).unwrap(), SYMBOL_TABLE);
        fs::write(&table, "(fp_lib_table (lib (name x)))").unwrap();
        prepare(&home, "10.0.7").unwrap();
        assert_eq!(fs::read_to_string(&table).unwrap(), FOOTPRINT_TABLE);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn library_errors_in_the_project_keep_the_user_libraries() {
        let directory = std::env::temp_dir().join(format!("pcb-maker-kicad-project-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let board = directory.join("board.kicad_pcb");
        assert!(!project_counts_library_findings(&board), "no project: KiCad's defaults");
        let project = directory.join("board.kicad_pro");
        let with = |severity: &str| {
            format!(
                "{{\"board\": {{\"design_settings\": {{\"rule_severities\": {{\"clearance\": \"error\", \"lib_footprint_mismatch\": \"{severity}\"}}}}}}}}"
            )
        };
        fs::write(&project, with("warning")).unwrap();
        assert!(!project_counts_library_findings(&board));
        fs::write(&project, with("ignore")).unwrap();
        assert!(!project_counts_library_findings(&board));
        fs::write(&project, with("error")).unwrap();
        assert!(project_counts_library_findings(&board));
        fs::write(&project, "{").unwrap();
        assert!(project_counts_library_findings(&board), "unreadable: keep the user's libraries");
        let _ = fs::remove_dir_all(&directory);
    }
}
