// Copyright (C) 2026 Toit contributors.

//! Native report execution and the minimum structure required for admission.
//! Unknown metadata is retained in the original JSON. Required fields follow
//! https://schemas.kicad.org/erc.v1.json and https://schemas.kicad.org/drc.v1.json.

use serde::Deserialize;
use serde_json::Value;
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

/// Whether a validated DRC finding is tracked as library metadata rather than
/// a board design violation. Keep this policy shared by absolute verification
/// and source/result admission; other warnings and mismatch errors still count.
pub fn is_library_metadata_warning(finding: &Value) -> bool {
    // `lib_footprint_issues`: the footprint's library is not installed, as
    // for boards shared without their libraries.
    matches!(
        finding["type"].as_str(),
        Some("lib_footprint_mismatch" | "lib_footprint_issues")
    ) && finding["severity"] == "warning"
}

/// Whether a validated ERC finding is library metadata: a warning about a
/// symbol's library (`lib_symbol_issues`: not in the table, or not found;
/// `lib_symbol_mismatch`: differs from the library) or a symbol's
/// footprint link (`footprint_link_issues`). These depend on the libraries
/// installed where KiCad runs, not on the design, and pcb-maker runs ERC
/// without the global libraries (`kicad_config`), so they are kept out of
/// `erc_violations` and counted in `erc_library_warnings`.
pub fn is_erc_library_warning(finding: &Value) -> bool {
    matches!(
        finding["type"].as_str(),
        Some("lib_symbol_issues" | "lib_symbol_mismatch" | "footprint_link_issues")
    ) && finding["severity"] == "warning"
}

/// Design findings of an already validated ERC report: every sheet's
/// violations but the library warnings.
pub fn erc_design_issues(report: &Value) -> Vec<&Value> {
    report["sheets"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|sheet| sheet["violations"].as_array().into_iter().flatten())
        .filter(|finding| !is_erc_library_warning(finding))
        .collect()
}

#[derive(Clone, Copy, Debug)]
pub(super) enum ReportKind {
    Erc,
    Drc,
}

impl ReportKind {
    fn schema(self) -> &'static str {
        match self {
            Self::Erc => "https://schemas.kicad.org/erc.v1.json",
            Self::Drc => "https://schemas.kicad.org/drc.v1.json",
        }
    }
}

#[derive(Deserialize)]
struct Metadata<'a> {
    #[serde(rename = "$schema", borrow)]
    schema: Option<&'a str>,
    source: &'a str,
    date: &'a str,
    kicad_version: &'a str,
    coordinate_units: Option<&'a str>,
    included_severities: Option<Vec<&'a str>>,
}

/// Required finding data must not disappear into a default empty collection.
#[derive(Deserialize)]
struct Finding<'a> {
    #[serde(rename = "type", borrow)]
    kind: &'a str,
    description: &'a str,
    severity: &'a str,
    items: Vec<Item<'a>>,
}

#[derive(Deserialize)]
struct Item<'a> {
    uuid: &'a str,
    description: &'a str,
    pos: Position,
}

#[derive(Deserialize)]
struct Position {
    x: f64,
    y: f64,
}

#[derive(Deserialize)]
struct Sheet<'a> {
    path: &'a str,
    uuid_path: &'a str,
    #[serde(borrow)]
    violations: Vec<Finding<'a>>,
}

fn required_array<'a>(value: &'a Value, field: &str) -> Result<&'a Vec<Value>, String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing or non-array {field}"))
}

fn validate_findings(findings: &[Finding<'_>]) -> Result<(), String> {
    for finding in findings {
        if finding.kind.is_empty()
            || finding.description.is_empty()
            || !matches!(finding.severity, "error" | "warning" | "exclusion")
        {
            return Err("finding has an invalid type, description, or severity".into());
        }
        for item in &finding.items {
            if item.uuid.is_empty()
                || item.description.is_empty()
                || !item.pos.x.is_finite()
                || !item.pos.y.is_finite()
            {
                return Err("finding has an invalid affected item".into());
            }
        }
    }
    Ok(())
}

pub(super) fn validate_report(
    report: &Value,
    kind: ReportKind,
    input: &Path,
) -> Result<(), String> {
    let validate = || -> Result<(), String> {
        let metadata = Metadata::deserialize(report).map_err(|error| error.to_string())?;
        if metadata
            .schema
            .is_some_and(|schema| schema != kind.schema())
        {
            return Err("unsupported report schema".into());
        }
        if metadata.source.is_empty()
            || metadata.date.is_empty()
            || metadata.kicad_version.is_empty()
        {
            return Err("source, date, and kicad_version must be non-empty".into());
        }
        if Path::new(metadata.source).file_name() != input.file_name() {
            return Err(format!(
                "report source {:?} does not match {}",
                metadata.source,
                input.display()
            ));
        }
        if metadata
            .coordinate_units
            .is_some_and(|units| !matches!(units, "mm" | "mils" | "in"))
            || matches!(kind, ReportKind::Drc) && metadata.coordinate_units.is_none()
        {
            return Err("missing or unsupported coordinate_units".into());
        }
        // Older KiCad reports omit this metadata. When present, it must agree
        // with our --severity-all invocation rather than hide a finding class.
        if let Some(severities) = &metadata.included_severities
            && !["error", "warning", "exclusion"]
                .iter()
                .all(|kind| severities.contains(kind))
        {
            return Err("report does not include every requested severity".into());
        }
        match kind {
            ReportKind::Erc => {
                for (index, sheet) in required_array(report, "sheets")?.iter().enumerate() {
                    let sheet = Sheet::deserialize(sheet)
                        .map_err(|error| format!("sheets[{index}]: {error}"))?;
                    if sheet.path.is_empty() || sheet.uuid_path.is_empty() {
                        return Err(format!("sheets[{index}] has an empty identity"));
                    }
                    validate_findings(&sheet.violations)?;
                }
            }
            ReportKind::Drc => {
                for field in ["violations", "unconnected_items", "schematic_parity"] {
                    required_array(report, field)?;
                    let findings = Vec::<Finding<'_>>::deserialize(&report[field])
                        .map_err(|error| format!("{field}: {error}"))?;
                    validate_findings(&findings)?;
                }
            }
        }
        Ok(())
    };
    validate().map_err(|error| format!("invalid {kind:?} report for {}: {error}", input.display()))
}

pub(super) fn remove_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to invalidate {}: {error}", path.display())),
    }
}

struct ReportDirectory(PathBuf);

impl ReportDirectory {
    fn new(parent: &Path) -> Result<Self, String> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let directory = parent.join(format!(
            ".kicad-report-{}-{nonce}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory)
            .map_err(|error| format!("failed to create {}: {error}", directory.display()))?;
        Ok(Self(directory))
    }
}

impl Drop for ReportDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The oldest KiCad major version pcb-maker is checked against: it writes
/// KiCad 10's file format, and every benchmark ran on KiCad 10.0.7.
const KICAD_MAJOR: u32 = 10;

/// The warning for a kicad-cli of `version` (`kicad-cli version`'s output)
/// older than [`KICAD_MAJOR`]; `None` for a current or unreadable one.
fn old_kicad_warning(version: &str) -> Option<String> {
    let major: u32 = version
        .trim()
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()?;
    (major < KICAD_MAJOR).then(|| {
        format!(
            "warning: kicad-cli {} is older than KiCad {KICAD_MAJOR}: pcb-maker writes KiCad {KICAD_MAJOR} boards and is tested with KiCad 10.0.7 only; its checks may fail or differ",
            version.trim()
        )
    })
}

/// Warns once per process, before the first KiCad check, when `program`
/// is older than KiCad 10 (its version is asked once and cached).
fn warn_if_old_kicad(program: &OsStr) {
    static CHECKED: std::sync::Once = std::sync::Once::new();
    CHECKED.call_once(|| {
        if let Some(warning) =
            crate::drc_cache::tool_version(program).and_then(|version| old_kicad_warning(&version))
        {
            eprintln!("{warning}");
        }
    });
}

static KICAD_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static KICAD_NANOS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// kicad-cli's reports and saves in this process (the DRC cache's answers
/// among them) and their wall seconds, summed over threads.
pub(crate) fn kicad_totals() -> (u64, f64) {
    use std::sync::atomic::Ordering::Relaxed;
    (
        KICAD_CALLS.load(Relaxed),
        KICAD_NANOS.load(Relaxed) as f64 / 1.0e9,
    )
}

/// Counts a kicad-cli run that started at `started` (`kicad_totals`).
fn note_kicad(started: std::time::Instant) {
    use std::sync::atomic::Ordering::Relaxed;
    KICAD_CALLS.fetch_add(1, Relaxed);
    KICAD_NANOS.fetch_add(started.elapsed().as_nanos() as u64, Relaxed);
}

pub(super) fn run_kicad_report(
    arguments: &[&str],
    output: &Path,
    input: &Path,
) -> Result<(), String> {
    let started = std::time::Instant::now();
    let result = run_kicad_report_untimed(arguments, output, input);
    note_kicad(started);
    result
}

fn run_kicad_report_untimed(arguments: &[&str], output: &Path, input: &Path) -> Result<(), String> {
    let program = OsStr::new("kicad-cli");
    warn_if_old_kicad(program);
    // ERC too: its library warnings are not counted (`is_erc_library_warning`).
    let config_home = match arguments.get(..2) {
        Some(["pcb", "drc"]) => crate::kicad_config::drc_config_home(program, input),
        Some(["sch", "erc"]) => crate::kicad_config::erc_config_home(program, input),
        _ => None,
    };
    run_report_command_cached(
        program,
        arguments,
        output,
        input,
        crate::drc_cache::directory_from_env().as_deref(),
        config_home.as_deref(),
    )
}

#[cfg(test)]
fn run_report_command(
    program: &OsStr,
    arguments: &[&str],
    output: &Path,
    input: &Path,
) -> Result<(), String> {
    run_report_command_cached(program, arguments, output, input, None, None)
}

/// Runs a native report, a DRC through the cache in `cache` (see
/// `drc_cache`): a hit is validated and published exactly as a fresh
/// report is. `config_home`, when given, is kicad-cli's `KICAD_CONFIG_HOME`
/// (see `kicad_config`) and part of the cache key.
fn run_report_command_cached(
    program: &OsStr,
    arguments: &[&str],
    output: &Path,
    input: &Path,
    cache: Option<&Path>,
    config_home: Option<&Path>,
) -> Result<(), String> {
    let kind = match arguments.get(..2) {
        Some(["sch", "erc"]) => ReportKind::Erc,
        Some(["pcb", "drc"]) => ReportKind::Drc,
        _ => return Err("unsupported native report command".into()),
    };
    // Findings are interpreted from JSON. Nonzero exits mean execution failed;
    // do not ask KiCad to also encode ordinary findings in its exit status.
    if arguments.contains(&"--exit-code-violations") || arguments.last() != Some(&"--output") {
        return Err(
            "native report command requires --output last and no --exit-code-violations".into(),
        );
    }
    remove_if_present(output)?;
    let cache = match kind {
        ReportKind::Drc => {
            crate::drc_cache::count_call();
            // The key is taken before KiCad runs: the inputs as given.
            cache.and_then(|directory| {
                crate::drc_cache::DrcCache::for_run(
                    directory.to_path_buf(),
                    program,
                    arguments,
                    input,
                    config_home,
                )
            })
        }
        ReportKind::Erc => None,
    };
    if let Some(cache) = &cache
        && let Some(bytes) = cache.lookup()
    {
        let valid = serde_json::from_slice::<Value>(&bytes)
            .map_err(|error| error.to_string())
            .and_then(|report| validate_report(&report, kind, input));
        match valid {
            Ok(()) => {
                crate::drc_cache::write_atomically(output, &bytes).map_err(|error| {
                    format!(
                        "failed to publish native report {}: {error}",
                        output.display()
                    )
                })?;
                cache.count_hit();
                return Ok(());
            }
            Err(_) => cache.discard(),
        }
    }
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = ReportDirectory::new(parent)?;
    let fresh = temporary.0.join("report.json");
    let mut command = Command::new(program);
    command.args(arguments).arg(&fresh).arg(input);
    if let Some(home) = config_home {
        command.env("KICAD_CONFIG_HOME", home);
    }
    let (status, transcript) = run_bounded(&mut command, &temporary.0)?;
    if !status.success() {
        return Err(format!("kicad-cli failed with {status}: {transcript}"));
    }
    let report = super::read_json(&fresh)?;
    validate_report(&report, kind, input)?;
    if let Some(cache) = &cache {
        match fs::read(&fresh) {
            Ok(bytes) => cache.store(&bytes),
            Err(error) => eprintln!("DRC cache: failed to read {}: {error}", fresh.display()),
        }
    }
    fs::rename(&fresh, output).map_err(|error| {
        format!(
            "failed to publish native report {}: {error}",
            output.display()
        )
    })
}

/// Refills the zones of `board` with kicad-cli's DRC (which also rebuilds
/// teardrops) and saves the board in place: KiCad's own fill, for checks
/// that need its geometry (`necks`). Not cached: the cache keeps reports,
/// not boards. The report goes to `report`.
pub(super) fn run_kicad_save_board(board: &Path, report: &Path) -> Result<(), String> {
    let started = std::time::Instant::now();
    let result = run_kicad_save_board_untimed(board, report);
    note_kicad(started);
    result
}

fn run_kicad_save_board_untimed(board: &Path, report: &Path) -> Result<(), String> {
    let scratch = board
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let program = OsStr::new("kicad-cli");
    warn_if_old_kicad(program);
    let mut command = Command::new(program);
    command
        .args([
            "pcb",
            "drc",
            "--refill-zones",
            "--save-board",
            "--severity-error",
            "--format",
            "json",
            "--output",
        ])
        .arg(report)
        .arg(board);
    crate::kicad_config::configure_drc(&mut command, program, board);
    let (status, transcript) = run_bounded(&mut command, scratch)?;
    if !status.success() {
        return Err(format!("kicad-cli failed with {status}: {transcript}"));
    }
    Ok(())
}

/// Seconds a kicad-cli report may take (`PCB_KICAD_TIMEOUT`, default 900):
/// a zone refill on 0xCB's panel never finished, and the hung process
/// outlived the router by hours.
fn report_timeout() -> std::time::Duration {
    let seconds = std::env::var("PCB_KICAD_TIMEOUT")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| *value > 0.0)
        .unwrap_or(900.0);
    std::time::Duration::from_secs_f64(seconds)
}

/// Runs `command` with its output in `scratch`, killed after
/// `report_timeout()` and, on Linux, when this process dies.
fn run_bounded(
    command: &mut Command,
    scratch: &Path,
) -> Result<(std::process::ExitStatus, String), String> {
    let log = scratch.join("kicad-cli.log");
    let file = fs::File::create(&log)
        .map_err(|error| format!("failed to create {}: {error}", log.display()))?;
    let error_file = file.try_clone().map_err(|error| error.to_string())?;
    command.stdout(file).stderr(error_file);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: prctl is async-signal-safe and touches no memory.
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to run kicad-cli: {error}"))?;
    let timeout = report_timeout();
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "kicad-cli timed out after {:.0} s",
                timeout.as_secs_f64()
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    Ok((status, fs::read_to_string(&log).unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report(kind: ReportKind) -> Value {
        let mut report = json!({
            "$schema": kind.schema(),
            "source": match kind { ReportKind::Erc => "board.kicad_sch", ReportKind::Drc => "board.kicad_pcb" },
            "date": "2026-09-07T12:00:00", "kicad_version": "10.0.6",
            "coordinate_units": "mm", "included_severities": ["error", "warning", "exclusion"]
        });
        match kind {
            ReportKind::Erc => {
                report["sheets"] = json!([{"path": "/", "uuid_path": "/root", "violations": []}])
            }
            ReportKind::Drc => {
                for field in ["violations", "unconnected_items", "schematic_parity"] {
                    report[field] = json!([]);
                }
            }
        }
        report
    }

    #[test]
    fn kicad_older_than_10_is_warned_about() {
        assert!(old_kicad_warning("9.0.1\n").is_some_and(|text| text.contains("kicad-cli 9.0.1")));
        assert!(old_kicad_warning("8.0.9-rc1").is_some());
        assert_eq!(old_kicad_warning("10.0.7"), None);
        assert_eq!(old_kicad_warning("11.0.0"), None);
        assert_eq!(old_kicad_warning("unknown"), None);
    }

    #[test]
    fn library_warnings_remain_visible_without_masking_design_errors() {
        let erc = report(ReportKind::Erc);
        let mut drc = report(ReportKind::Drc);
        drc["violations"] = json!([{
            "type": "lib_footprint_mismatch", "severity": "warning",
            "description": "Footprint differs from library", "items": []
        }]);
        let assess = |drc: &Value| {
            crate::verification::assess_verification_reports("board", &erc, drc).unwrap()
        };
        let result = assess(&drc);
        assert!(result.complete);
        assert_eq!(result.library_metadata_warnings, 1);
        assert_eq!(result.drc_design_violations, 0);
        for (kind, severity) in [
            ("lib_footprint_mismatch", "error"),
            ("clearance", "warning"),
            ("new_kicad_rule", "warning"),
        ] {
            drc["violations"][0]["type"] = json!(kind);
            drc["violations"][0]["severity"] = json!(severity);
            let result = assess(&drc);
            assert!(!result.complete);
            assert_eq!(result.library_metadata_warnings, 0);
            assert_eq!(result.drc_design_violations, 1);
        }
    }

    #[test]
    fn rejects_missing_or_malformed_required_report_data() {
        let pcb = Path::new("board.kicad_pcb");
        for invalid in [json!({}), json!(null), json!([])] {
            assert!(validate_report(&invalid, ReportKind::Drc, pcb).is_err());
        }
        for field in [
            "violations",
            "unconnected_items",
            "schematic_parity",
            "source",
            "date",
            "kicad_version",
            "coordinate_units",
        ] {
            let mut invalid = report(ReportKind::Drc);
            invalid.as_object_mut().unwrap().remove(field);
            assert!(
                validate_report(&invalid, ReportKind::Drc, pcb).is_err(),
                "missing {field}"
            );
        }
        for invalid_findings in [json!(null), json!({}), json!([null]), json!([{}])] {
            let mut invalid = report(ReportKind::Drc);
            invalid["violations"] = invalid_findings;
            assert!(validate_report(&invalid, ReportKind::Drc, pcb).is_err());
        }
        let mut erc = report(ReportKind::Erc);
        erc["sheets"][0]
            .as_object_mut()
            .unwrap()
            .remove("violations");
        assert!(validate_report(&erc, ReportKind::Erc, Path::new("board.kicad_sch")).is_err());
    }

    #[test]
    fn rejects_wrong_source_kind_and_filtered_findings() {
        let pcb = Path::new("board.kicad_pcb");
        let valid = report(ReportKind::Drc);
        assert!(validate_report(&valid, ReportKind::Drc, pcb).is_ok());
        assert!(validate_report(&valid, ReportKind::Erc, pcb).is_err());
        assert!(
            validate_report(&valid, ReportKind::Drc, Path::new("different.kicad_pcb")).is_err()
        );
        let mut filtered = valid.clone();
        filtered["included_severities"] = json!(["warning"]);
        assert!(validate_report(&filtered, ReportKind::Drc, pcb).is_err());
        let mut older = valid;
        older.as_object_mut().unwrap().remove("included_severities");
        older.as_object_mut().unwrap().remove("$schema");
        assert!(validate_report(&older, ReportKind::Drc, pcb).is_ok());
    }

    fn finding() -> Value {
        json!({"type": "clearance", "description": "Copper too close", "severity": "error", "items": []})
    }

    #[test]
    fn valid_reports_with_findings_are_incomplete_and_malformed_reports_invalidate_old_verdicts() {
        let directory = ReportDirectory::new(&std::env::temp_dir()).unwrap();
        let verdict = directory.0.join("verification.json");
        let erc = report(ReportKind::Erc);
        let mut drc = report(ReportKind::Drc);
        let assess = |drc: &Value| {
            crate::verification::write_verification_report(&directory.0, "board", &erc, drc)
        };
        assert!(assess(&drc).unwrap().complete);
        drc["violations"] = json!([finding()]);
        let result = assess(&drc).unwrap();
        assert!(!result.complete);
        assert_eq!(result.drc_design_violations, 1);
        assert!(assess(&json!({})).is_err());
        assert!(!verdict.exists());
    }

    #[cfg(unix)]
    fn fake_cli(directory: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let program = directory.join("fake-kicad-cli");
        fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        program
    }

    #[cfg(unix)]
    fn write_report_script(contents: &Value, exit: u8) -> String {
        format!(
            "while [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = \"--output\" ]; then\n    shift\n    cat > \"$1\" <<'REPORT'\n{contents}\nREPORT\n    exit {exit}\n  fi\n  shift\ndone\nexit 99"
        )
    }

    #[test]
    #[cfg(unix)]
    fn failed_missing_and_malformed_fresh_reports_never_reuse_old_output() {
        let directory = ReportDirectory::new(&std::env::temp_dir()).unwrap();
        let output = directory.0.join("drc.json");
        let valid = report(ReportKind::Drc);
        for body in [
            "exit 1".into(),
            "exit 0".into(),
            write_report_script(&valid, 1),
            write_report_script(&json!({}), 0),
        ] {
            fs::write(&output, valid.to_string()).unwrap();
            let cli = fake_cli(&directory.0, &body);
            let result = run_report_command(
                cli.as_os_str(),
                &["pcb", "drc", "--output"],
                &output,
                Path::new("board.kicad_pcb"),
            );
            assert!(result.is_err(), "accepted script: {body}");
            assert!(!output.exists(), "retained an unverified report");
            assert!(
                fs::read_dir(&directory.0).unwrap().all(|entry| !entry
                    .unwrap()
                    .file_type()
                    .unwrap()
                    .is_dir())
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn two_drc_runs_on_the_same_board_make_one_cache_entry_and_one_kicad_run() {
        let directory = ReportDirectory::new(&std::env::temp_dir()).unwrap();
        let project = directory.0.join("project");
        let cache = directory.0.join("cache");
        fs::create_dir(&project).unwrap();
        let board = project.join("board.kicad_pcb");
        fs::write(&board, "(kicad_pcb (version 20240108))\n").unwrap();
        fs::write(project.join("board.kicad_pro"), "{}\n").unwrap();
        let runs = directory.0.join("runs");
        let mut valid = report(ReportKind::Drc);
        valid["violations"] = json!([finding()]);
        let body = format!(
            "if [ \"$1\" = version ]; then echo 10.0.7; exit 0; fi\necho run >> {}\n{}",
            runs.display(),
            write_report_script(&valid, 0)
        );
        let cli = fake_cli(&directory.0, &body);
        let arguments = [
            "pcb",
            "drc",
            "--format",
            "json",
            "--severity-all",
            "--output",
        ];
        let run = |output: &Path| {
            run_report_command_cached(
                cli.as_os_str(),
                &arguments,
                output,
                &board,
                Some(&cache),
                None,
            )
            .unwrap();
            fs::read(output).unwrap()
        };
        let kicad_runs = || fs::read_to_string(&runs).map_or(0, |text| text.lines().count());
        let entries = || fs::read_dir(&cache).map_or(0, |entries| entries.count());
        let counts = crate::drc_cache::drc_cache_counts();
        let first = run(&project.join("drc.json"));
        assert_eq!((kicad_runs(), entries()), (1, 1));
        let second = run(&directory.0.join("second.json"));
        assert_eq!((kicad_runs(), entries()), (1, 1), "the second run is a hit");
        assert_eq!(first, second, "a hit gives the fresh report's bytes");
        assert_eq!(
            super::super::read_json(&directory.0.join("second.json")).unwrap(),
            valid
        );
        let after = crate::drc_cache::drc_cache_counts();
        assert!(after.calls >= counts.calls + 2 && after.hits > counts.hits);
        // One byte more in the board, or other arguments: a miss and a new entry.
        fs::write(&board, "(kicad_pcb (version 20240108)) \n").unwrap();
        run(&project.join("drc.json"));
        assert_eq!((kicad_runs(), entries()), (2, 2));
        let refill = [
            "pcb",
            "drc",
            "--format",
            "json",
            "--severity-all",
            "--refill-zones",
            "--output",
        ];
        run_report_command_cached(
            cli.as_os_str(),
            &refill,
            &project.join("drc.json"),
            &board,
            Some(&cache),
            None,
        )
        .unwrap();
        assert_eq!((kicad_runs(), entries()), (3, 3));
        // A corrupt entry is dropped and the DRC runs again.
        for entry in fs::read_dir(&cache).unwrap() {
            fs::write(entry.unwrap().path(), "{").unwrap();
        }
        run(&project.join("drc.json"));
        assert_eq!(kicad_runs(), 4);
        assert_eq!(
            super::super::read_json(&project.join("drc.json")).unwrap(),
            valid
        );
        // No temporary files are left next to the outputs or in the cache.
        for place in [&project, &cache, &directory.0] {
            for entry in fs::read_dir(place).unwrap() {
                let name = entry.unwrap().file_name().into_string().unwrap();
                assert!(
                    !name.ends_with(".tmp"),
                    "left {name} in {}",
                    place.display()
                );
            }
        }
    }

    #[test]
    fn erc_library_warnings_are_not_design_issues() {
        let finding = |kind: &str, severity: &str| json!({"type": kind, "severity": severity, "description": "", "items": []});
        let erc = json!({"sheets": [
            {"violations": [finding("lib_symbol_issues", "warning"), finding("pin_not_connected", "error")]},
            {"violations": [finding("footprint_link_issues", "warning"), finding("lib_symbol_mismatch", "warning"),
                            finding("lib_symbol_issues", "error")]},
        ]});
        let kinds: Vec<_> = erc_design_issues(&erc)
            .iter()
            .map(|finding| finding["type"].as_str().unwrap())
            .collect();
        assert_eq!(
            kinds,
            ["pin_not_connected", "lib_symbol_issues"],
            "a library rule rated an error counts"
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_drc_without_libraries_runs_with_the_empty_tables_and_its_own_cache_key() {
        let directory = ReportDirectory::new(&std::env::temp_dir()).unwrap();
        let project = directory.0.join("project");
        let cache = directory.0.join("cache");
        fs::create_dir(&project).unwrap();
        let board = project.join("board.kicad_pcb");
        fs::write(&board, "(kicad_pcb (version 20240108))\n").unwrap();
        fs::write(project.join("board.kicad_pro"), "{}\n").unwrap();
        let home =
            crate::kicad_config::prepare(&directory.0.join("kicad-config"), "10.0.7").unwrap();
        let runs = directory.0.join("runs");
        let body = format!(
            "if [ \"$1\" = version ]; then echo 10.0.7; exit 0; fi\n\
             if [ -f \"$KICAD_CONFIG_HOME/10.0/fp-lib-table\" ] && [ -f \"$KICAD_CONFIG_HOME/10.0/sym-lib-table\" ] \
             && ! grep -q '(lib ' \"$KICAD_CONFIG_HOME/10.0/fp-lib-table\"; then\n  echo \"empty $KICAD_CONFIG_HOME\" >> {runs}\n\
             else\n  echo \"user ${{KICAD_CONFIG_HOME:-}}\" >> {runs}\nfi\n{}",
            write_report_script(&report(ReportKind::Drc), 0),
            runs = runs.display(),
        );
        let cli = fake_cli(&directory.0, &body);
        let arguments = [
            "pcb",
            "drc",
            "--format",
            "json",
            "--severity-all",
            "--output",
        ];
        let output = project.join("drc.json");
        let lines = || {
            fs::read_to_string(&runs)
                .unwrap_or_default()
                .lines()
                .map(String::from)
                .collect::<Vec<_>>()
        };
        let entries = || fs::read_dir(&cache).map_or(0, |entries| entries.count());
        run_report_command_cached(
            cli.as_os_str(),
            &arguments,
            &output,
            &board,
            Some(&cache),
            Some(&home),
        )
        .unwrap();
        assert_eq!(lines(), [format!("empty {}", home.display())]);
        assert_eq!(entries(), 1);
        run_report_command_cached(
            cli.as_os_str(),
            &arguments,
            &output,
            &board,
            Some(&cache),
            Some(&home),
        )
        .unwrap();
        assert_eq!(
            (lines().len(), entries()),
            (1, 1),
            "the same configuration hits"
        );
        // The user's configuration is another key: a miss, a KiCad run without the override.
        run_report_command_cached(
            cli.as_os_str(),
            &arguments,
            &output,
            &board,
            Some(&cache),
            None,
        )
        .unwrap();
        let lines = lines();
        assert_eq!((lines.len(), entries()), (2, 2));
        assert!(
            !lines[1].contains(&home.display().to_string()),
            "{}",
            lines[1]
        );
    }

    #[test]
    #[cfg(unix)]
    fn successful_fresh_report_preserves_findings_and_cleans_temporary_files() {
        let directory = ReportDirectory::new(&std::env::temp_dir()).unwrap();
        let output = directory.0.join("drc.json");
        let mut valid = report(ReportKind::Drc);
        valid["violations"] = json!([finding()]);
        let cli = fake_cli(&directory.0, &write_report_script(&valid, 0));
        run_report_command(
            cli.as_os_str(),
            &["pcb", "drc", "--output"],
            &output,
            Path::new("board.kicad_pcb"),
        )
        .unwrap();
        assert_eq!(super::super::read_json(&output).unwrap(), valid);
        assert!(
            fs::read_dir(&directory.0).unwrap().all(|entry| !entry
                .unwrap()
                .file_type()
                .unwrap()
                .is_dir())
        );
    }
}
