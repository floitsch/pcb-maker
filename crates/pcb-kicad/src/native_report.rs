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
    finding["type"] == "lib_footprint_mismatch" && finding["severity"] == "warning"
}

/// Design findings from an already validated native DRC report. Metadata
/// warnings remain available in the original report and verification counts.
pub fn drc_design_issues(report: &Value) -> Vec<&Value> {
    report["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|finding| !is_library_metadata_warning(finding))
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
        if let Some(severities) = &metadata.included_severities {
            if !["error", "warning", "exclusion"]
                .iter()
                .all(|kind| severities.contains(kind))
            {
                return Err("report does not include every requested severity".into());
            }
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

pub(super) fn run_kicad_report(
    arguments: &[&str],
    output: &Path,
    input: &Path,
) -> Result<(), String> {
    run_report_command(OsStr::new("kicad-cli"), arguments, output, input)
}

fn run_report_command(
    program: &OsStr,
    arguments: &[&str],
    output: &Path,
    input: &Path,
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
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = ReportDirectory::new(parent)?;
    let fresh = temporary.0.join("report.json");
    let result = Command::new(program)
        .args(arguments)
        .arg(&fresh)
        .arg(input)
        .output()
        .map_err(|error| format!("failed to run kicad-cli: {error}"))?;
    if !result.status.success() {
        return Err(format!(
            "kicad-cli failed with {}: {}{}",
            result.status,
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    let report = super::read_json(&fresh)?;
    validate_report(&report, kind, input)?;
    fs::rename(&fresh, output).map_err(|error| {
        format!(
            "failed to publish native report {}: {error}",
            output.display()
        )
    })
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
    fn library_warnings_remain_visible_without_masking_design_errors() {
        let erc = report(ReportKind::Erc);
        let mut drc = report(ReportKind::Drc);
        drc["violations"] = json!([{
            "type": "lib_footprint_mismatch", "severity": "warning",
            "description": "Footprint differs from library", "items": []
        }]);
        let assess =
            |drc: &Value| super::super::assess_verification_reports("board", &erc, drc).unwrap();
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
        let assess =
            |drc: &Value| super::super::write_verification_report(&directory.0, "board", &erc, drc);
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
