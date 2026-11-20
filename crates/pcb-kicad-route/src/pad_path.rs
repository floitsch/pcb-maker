// Copyright (C) 2026 Florian Loitsch.

//! Replace an explicit pad pair's detour without discarding a shared tee.

use super::*;
use crate::board_router::LayerTable;
use crate::open_repair::{
    KICAD_FINDING_TYPE_FLOOR, KICAD_UNCONNECTED_CAP, finding_counts, finding_types_at_cap,
    in_repair_copy, preserved, report_capped, reported_missing_libraries,
};
use serde_json::Value;

mod decoupling;
pub use decoupling::schematic_decoupling_pair;
mod feed;
mod planner;
pub use decoupling::DecouplingShortcut;
pub use feed::{ParallelFeed, ParallelFeedFill, ParallelFeedPublisher, add_kicad_parallel_feed};

/// What an explicit pad-pair shortening kept and its native verification.
#[derive(Default, Serialize)]
pub struct PadPathShortening {
    pub pads: [String; 2],
    pub net: String,
    pub status: &'static str,
    pub reason: String,
    pub trace_span_before_mm: Option<f64>,
    pub trace_span_after_mm: Option<f64>,
    /// Measured from track centrelines and the modeled pad outlines.
    pub span_measurement: &'static str,
    pub track_length_change_mm: f64,
    pub removed_tracks: usize,
    pub layer: Option<String>,
    pub kept_shared_tee: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_checks_directory: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predecessor_report: Option<String>,
    pub baseline_checks: usize,
    pub trial_checks: usize,
    pub rejected: Vec<PadPathRejection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoupling_shortcut: Option<DecouplingShortcut>,
}

#[derive(Serialize)]
pub struct PadPathRejection {
    pub layer: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rule_types: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_report: Option<String>,
}

fn reserve_directory(project: &Path, stem: &str) -> Result<String, String> {
    for number in 1.. {
        let name = if number == 1 {
            stem.to_owned()
        } else {
            format!("{stem}-{number}")
        };
        let path = project.join(&name);
        match fs::create_dir(&path) {
            Ok(()) => return Ok(name),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("cannot create {}: {error}", path.display())),
        }
    }
    unreachable!("a project could not contain every evidence directory")
}

/// Keep the previous manifest's report references valid relative to the
/// copied project, even if the new request never reaches native checks.
fn archive_predecessor(project: &Path) -> Result<Option<String>, String> {
    let manifest = project.join("pad-path.json");
    if !manifest.exists() {
        return Ok(None);
    }
    let directory = reserve_directory(project, "pad-path-predecessor")?;
    let previous = format!("{directory}/pad-path.json");
    fs::rename(manifest, project.join(&previous)).map_err(|error| error.to_string())?;
    Ok(Some(previous))
}

fn record_kept_checks(
    project: &Path,
    reports: &Path,
    trial: usize,
    saved_fill_report: Option<&Path>,
) -> Result<(), String> {
    let originals = reports.join("input-evidence");
    fs::create_dir(&originals).map_err(|error| error.to_string())?;
    for name in [
        "board-layout.json",
        "board-router.json",
        "drc.json",
        "drc-saved-fill.json",
        "drc-refilled.json",
        "fill-handoff.json",
        "verification.json",
        "verdict.json",
        "preview.json",
        "preview.svg",
    ] {
        let input = project.join(name);
        if input.is_file() {
            fs::rename(input, originals.join(name)).map_err(|error| error.to_string())?;
        }
    }
    let stored = reports.join(format!("trial-{trial}-stored.json"));
    fs::copy(
        saved_fill_report.unwrap_or(&stored),
        project.join("drc.json"),
    )
    .map_err(|error| error.to_string())?;
    fs::copy(
        reports.join(format!("trial-{trial}-refilled.json")),
        project.join("drc-refilled.json"),
    )
    .map_err(|error| error.to_string())?;
    if let Some(saved) = saved_fill_report {
        fs::copy(saved, project.join("drc-saved-fill.json")).map_err(|error| error.to_string())?;
    }
    Ok(())
}

const MODES: [&str; 2] = ["stored", "refilled"];

fn checked(board: &Path, reports: &Path, label: &str, refill: bool) -> Result<Value, String> {
    native_report::run_route_reuse_drc(board, &reports.join(format!("{label}.json")), refill)
        .map_err(|error| at_native_check(reports, label, &error))
}

fn at_native_check(reports: &Path, label: &str, reason: &str) -> String {
    let reason = reason.trim_end_matches('.');
    let filename = format!("{label}.json");
    let mode = if label.ends_with("refilled") {
        "refilled"
    } else {
        "stored"
    };
    let stage = if label.starts_with("before-") {
        "baseline"
    } else {
        "trial"
    };
    if reports.join(&filename).is_file() {
        let directory = reports.file_name().unwrap_or_default().to_string_lossy();
        format!("{reason}. See the {mode} native {stage} at {directory}/{filename}.")
    } else {
        format!("The {mode} native {stage} could not be verified ({reason}).")
    }
}

fn check_baseline(reports: &Path, mode: &str, report: &Value) -> Result<(), String> {
    check_complete(report)
        .map_err(|reason| at_native_check(reports, &format!("before-{mode}"), &reason))
}

fn check_complete(report: &Value) -> Result<(), String> {
    let mut reasons = Vec::new();
    let missing: Vec<_> = ["violations", "unconnected_items", "schematic_parity"]
        .into_iter()
        .filter(|field| !report[*field].is_array())
        .collect();
    if !missing.is_empty() {
        reasons.push(format!(
            "The native report needs arrays for {}. Rerun with the current schematic and a supported KiCad CLI that emits the required sections.",
            missing.join(", ")
        ));
    }

    let capped_kinds = finding_types_at_cap(report, "violations");
    let mut caps: Vec<_> = capped_kinds
        .iter()
        .map(|kind| {
            let count = report["violations"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|finding| finding["type"].as_str().unwrap_or_default() == *kind)
                .count();
            let kind = if kind.is_empty() {
                "unknown type"
            } else {
                *kind
            };
            format!("violations/{kind} ({count} reported)")
        })
        .collect();
    if let Some(parity) = report["schematic_parity"].as_array()
        && parity.len() >= KICAD_FINDING_TYPE_FLOOR
    {
        caps.push(format!("schematic_parity ({} reported)", parity.len()));
    }
    if let Some(opens) = report["unconnected_items"].as_array()
        && opens.len() >= KICAD_UNCONNECTED_CAP
    {
        caps.push(format!("unconnected_items ({} reported)", opens.len()));
    }
    if caps.is_empty() && report_capped(report) {
        caps.push("native report marked capped".into());
    }
    if !caps.is_empty() {
        reasons.push(format!(
            "KiCad may have reached its report limit for {}. Complete native finding evidence is required.",
            caps.join(", ")
        ));
        if capped_kinds.contains(&"lib_footprint_issues")
            && !reported_missing_libraries(report).is_empty()
        {
            reasons.push("Use PCB_MAKER_KICAD_USER_LIBS=1 if the matching libraries are already configured in KiCad. Restore any missing project footprint libraries and fp-lib-table entries, then retry in a fresh output directory.".into());
        } else {
            reasons.push(
                "Resolve the reported findings and retry with a complete native report.".into(),
            );
        }
    }

    if let Some(opens) = report["unconnected_items"].as_array()
        && !opens.is_empty()
    {
        let excluded = opens
            .iter()
            .filter(|item| item["excluded"] == true || item["severity"] == "exclusion")
            .count();
        reasons.push(format!(
            "KiCad reports an unconnected_items count of {} ({} active, {excluded} excluded). The operation requires this section to be empty.",
            opens.len(),
            opens.len() - excluded
        ));
    }
    let disabled: BTreeSet<_> = report["ignored_checks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|check| check["key"].as_str())
        .filter(|check| {
            matches!(
                *check,
                "clearance"
                    | "shorting_items"
                    | "copper_edge_clearance"
                    | "track_width"
                    | "items_not_allowed"
            )
        })
        .collect();
    if !disabled.is_empty() {
        let disabled = disabled.into_iter().collect::<Vec<_>>().join(", ");
        reasons.push(format!(
            "The project disables required native copper checks ({disabled}). Enable these checks in the verified project before retrying."
        ));
    }
    if !reasons.is_empty() {
        return Err(reasons.join(" "));
    }
    Ok(())
}

fn parity(report: &Value) -> Vec<String> {
    let mut items: Vec<_> = report["schematic_parity"]
        .as_array()
        .into_iter()
        .flatten()
        .map(Value::to_string)
        .collect();
    items.sort();
    items
}

fn violation_types(before: &Value, after: &Value) -> Vec<String> {
    let before = finding_counts(before, "violations").unwrap_or_default();
    finding_counts(after, "violations")
        .unwrap_or_default()
        .into_iter()
        .filter(|(finding, count)| *count > before.get(finding).copied().unwrap_or(0))
        .filter_map(|(finding, _)| {
            let value: Value = serde_json::from_str(&finding).ok()?;
            value["type"].as_str().map(str::to_owned)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn compare_trial(before: &Value, after: &Value) -> Result<(), String> {
    check_complete(after)?;
    if parity(before) != parity(after) {
        return Err("KiCad changed schematic parity".into());
    }
    if !preserved(before, after)? {
        return Err("KiCad added or changed a finding".into());
    }
    Ok(())
}

fn governed_path(file: &Path, board: &Expr, net: &str) -> Result<bool, String> {
    let rules = resolve_project_rules(&file.with_extension("kicad_pro"), file)?;
    let names = note_net_names(board);
    let normalized = names.normalize(net);
    Ok(rules.diff_pair_rules.contains_key(normalized)
        || rules
            .length_rules
            .iter()
            .any(|rule| rule.nets.iter().any(|member| member.net == normalized)))
}

fn shorten_in_copy(
    project: &Path,
    board_id: &str,
    pair: [&str; 2],
) -> Result<PadPathShortening, String> {
    let file = project.join(format!("{board_id}.kicad_pcb"));
    let original = fs::read_to_string(&file).map_err(|error| error.to_string())?;
    let board = parse(&original)?;
    let mut report = PadPathShortening {
        pads: pair.map(str::to_owned),
        status: "unchanged",
        span_measurement: "track centerline outside modeled pad copper",
        ..Default::default()
    };
    let codes = net_codes(&board);
    let [source_net, target_net] = pair.map(|pad| planner::pad_net(&board, &codes, pad));
    let Some(net) =
        source_net.filter(|name| target_net.as_ref() == Some(name) && pair[0] != pair[1])
    else {
        report.reason = "choose two distinct unique pads on the same named net".into();
        return Ok(report);
    };
    report.net = net.clone();
    if governed_path(&file, &board, &net)? {
        report.reason = "the selected net has a differential or length requirement".into();
        return Ok(report);
    }
    let layers = LayerTable::from_pcb(&board)?;
    let candidates = match planner::plan(&board, &codes, &net, &layers, pair) {
        Ok(candidates) => candidates,
        Err(reason) => {
            report.reason = reason;
            return Ok(report);
        }
    };
    report.trace_span_before_mm = Some(candidates[0].before_mm);
    report.trace_span_after_mm = report.trace_span_before_mm;
    let report_directory = reserve_directory(project, "pad-path-checks")?;
    let reports = project.join(&report_directory);
    report.native_checks_directory = Some(report_directory.clone());
    let before: Vec<_> = MODES
        .into_iter()
        .enumerate()
        .map(|(mode, label)| checked(&file, &reports, &format!("before-{label}"), mode != 0))
        .collect::<Result<_, _>>()?;
    report.baseline_checks = before.len();
    if let Err(error) = MODES
        .into_iter()
        .zip(&before)
        .try_for_each(|(mode, native)| check_baseline(&reports, mode, native))
    {
        report.reason = error;
        return Ok(report);
    }
    for (i, candidate) in candidates.into_iter().enumerate() {
        let mut chord = supplemental_segment_expr(&candidate.chord);
        let id = supplemental_segment_id(&candidate.chord);
        if original.to_ascii_lowercase().contains(&id) {
            report.rejected.push(PadPathRejection {
                layer: candidate.chord.layer,
                reason: "replacement track identity already exists".into(),
                rule_types: Vec::new(),
                native_report: None,
            });
            continue;
        }
        let net_field = board.children()[*candidate.remove.first().expect("nonempty leaf path")]
            .child("net")
            .expect("track net checked")
            .clone();
        if let Expr::List(fields) = &mut chord {
            for field in fields
                .iter_mut()
                .filter(|field| field.head() == Some("net"))
            {
                *field = net_field.clone();
            }
        }
        let mut changed = board
            .children()
            .iter()
            .enumerate()
            .filter(|(index, _)| !candidate.remove.contains(index))
            .map(|(_, form)| form.clone())
            .collect::<Vec<_>>();
        changed.push(chord);
        let trial = encode(&Expr::List(changed));
        fs::write(&file, &trial).map_err(|error| error.to_string())?;
        let gate =
            MODES
                .into_iter()
                .zip(&before)
                .enumerate()
                .try_for_each(|(mode, (label, before))| {
                    report.trial_checks += 1;
                    let trial_name = format!("trial-{i}-{label}");
                    let rejection = |reason, rule_types| PadPathRejection {
                        layer: candidate.chord.layer.clone(),
                        reason,
                        rule_types,
                        native_report: reports
                            .join(format!("{trial_name}.json"))
                            .is_file()
                            .then(|| format!("{report_directory}/{trial_name}.json")),
                    };
                    let after = checked(&file, &reports, &trial_name, mode != 0)
                        .map_err(|error| rejection(error, Vec::new()))?;
                    compare_trial(before, &after).map_err(|reason| {
                        rejection(
                            at_native_check(&reports, &trial_name, &reason),
                            violation_types(before, &after),
                        )
                    })
                });
        if let Err(rejection) = gate {
            report.rejected.push(rejection);
            fs::write(&file, &original).map_err(|error| error.to_string())?;
            continue;
        }
        if fs::read_to_string(&file).map_err(|error| error.to_string())? != trial {
            return Err("native check modified the trial board".into());
        }
        record_kept_checks(project, &reports, i, None)?;
        report.status = "shortened";
        report.trace_span_before_mm = Some(candidate.before_mm);
        report.trace_span_after_mm = Some(candidate.after_mm);
        report.track_length_change_mm = candidate.delta_mm;
        report.removed_tracks = candidate.remove.len();
        report.layer = Some(candidate.chord.layer);
        report.kept_shared_tee = candidate.kept_tee;
        report.reason = "stored and refilled native checks preserve all connections".into();
        return Ok(report);
    }
    report.reason = "native checks rejected the available shorter copper".into();
    Ok(report)
}

/// Tries a single direct chord on a current routed board, published only
/// after both fresh native checks. The source project is immutable.
pub fn shorten_kicad_pad_path(
    project: &Path,
    board_id: &str,
    output: &Path,
    pads: [&str; 2],
) -> Result<PadPathShortening, String> {
    check_board_id(board_id).map_err(exit::input_error)?;
    if !project.join(format!("{board_id}.kicad_sch")).is_file() {
        return Err(exit::input_error(
            "pad path shortening requires the current KiCad schematic",
        ));
    }
    in_repair_copy(project, output, |copy| {
        let mut report = shorten_in_copy(copy, board_id, pads)?;
        report.predecessor_report = archive_predecessor(copy)?;
        write_typed_json(&copy.join("pad-path.json"), &report)?;
        Ok(report)
    })
}

/// Add one local pad-to-pad shortcut for a schematic-verified decoupler.
pub fn shorten_kicad_decoupling_path(
    project: &Path,
    board_id: &str,
    output: &Path,
    pads: [&str; 2],
    publish: ParallelFeedPublisher,
) -> Result<PadPathShortening, String> {
    check_board_id(board_id).map_err(exit::input_error)?;
    if !project.join(format!("{board_id}.kicad_sch")).is_file() {
        return Err(exit::input_error(
            "a decoupling shortcut requires the current KiCad schematic",
        ));
    }
    in_repair_copy(project, output, |copy| {
        let inputs = project_companion_hashes(copy)?;
        let mut report = decoupling::shorten(copy, board_id, pads, publish)?;
        if project_companion_hashes(copy)? != inputs {
            return Err("native checks changed the project rules, schematic or libraries".into());
        }
        report.predecessor_report = archive_predecessor(copy)?;
        write_typed_json(&copy.join("pad-path.json"), &report)?;
        Ok(report)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_native_report() -> Value {
        serde_json::json!({"violations": [], "unconnected_items": [], "schematic_parity": []})
    }

    #[test]
    fn library_cap_reports_the_count_and_matching_library_remedy() {
        let mut native = complete_native_report();
        let missing = serde_json::json!({
            "type": "lib_footprint_issues",
            "severity": "warning",
            "description": "The current configuration does not include the footprint library 'Resistor_SMD'"
        });
        native["violations"] = serde_json::json!(vec![missing; KICAD_FINDING_TYPE_FLOOR]);
        let reason = check_complete(&native).unwrap_err();
        assert!(reason.contains("violations/lib_footprint_issues (199 reported)"));
        assert!(reason.contains("PCB_MAKER_KICAD_USER_LIBS=1"));
        assert!(!reason.contains("unconnected"));
        let findings = native["violations"].as_array_mut().unwrap();
        findings.pop();
        assert!(check_complete(&native).is_ok());
    }

    #[test]
    fn missing_sections_are_distinguished_from_the_parity_cap_and_opens() {
        let mut native = complete_native_report();
        native["schematic_parity"] = Value::Null;
        let reason = check_complete(&native).unwrap_err();
        assert!(reason.contains("needs arrays for schematic_parity."));
        assert!(!reason.contains("report limit"));
        assert!(!reason.contains("unconnected"));

        native["schematic_parity"] = serde_json::json!(vec![Value::Null; KICAD_FINDING_TYPE_FLOOR]);
        let reason = check_complete(&native).unwrap_err();
        assert!(reason.contains("report limit for schematic_parity (199 reported)"));
        assert!(!reason.contains("needs arrays"));
    }

    #[test]
    fn names_disabled_copper_checks_and_counts_actual_native_opens() {
        let mut native = complete_native_report();
        native["ignored_checks"] = serde_json::json!([
            {"key": "missing_courtyard"},
            {"key": "copper_edge_clearance"}
        ]);
        let reason = check_complete(&native).unwrap_err();
        assert!(reason.contains("checks (copper_edge_clearance)."));
        assert!(!reason.contains("missing_courtyard"));
        assert!(!reason.contains("unconnected"));

        native["unconnected_items"] = serde_json::json!([
            {"severity": "error"}, {"severity": "error", "excluded": true}
        ]);
        let reason = check_complete(&native).unwrap_err();
        assert!(reason.contains("unconnected_items count of 2 (1 active, 1 excluded)"));
        assert!(!reason.contains("report limit"));
    }

    #[test]
    fn a_different_finding_of_an_existing_type_is_reported() {
        let reported = |location| {
            serde_json::json!({
                "violations": [{"type": "clearance", "items": [{"uuid": location}]}]
            })
        };
        assert_eq!(
            violation_types(&reported("old"), &reported("new")),
            ["clearance"]
        );
        assert!(violation_types(&reported("old"), &reported("old")).is_empty());
    }

    #[test]
    fn another_pair_reserves_fresh_checks_and_keeps_the_predecessor_manifest() {
        let directory = temporary_next_to(&std::env::temp_dir().join("pcb-maker-pad-path"));
        fs::create_dir(&directory).unwrap();
        let first = reserve_directory(&directory, "pad-path-checks").unwrap();
        let first_check = directory.join(&first).join("before-stored.json");
        fs::write(&first_check, "first pair evidence").unwrap();
        let reason = check_baseline(&directory.join(&first), "stored", &Value::Null).unwrap_err();
        assert!(reason.ends_with("stored native baseline at pad-path-checks/before-stored.json."));
        let manifest = directory.join("pad-path.json");
        fs::write(
            &manifest,
            format!("{{\"native_checks_directory\":\"{first}\"}}"),
        )
        .unwrap();

        let next = reserve_directory(&directory, "pad-path-checks").unwrap();
        let predecessor = archive_predecessor(&directory).unwrap().unwrap();
        fs::write(
            directory.join(&next).join("before-stored.json"),
            "second pair evidence",
        )
        .unwrap();
        assert_eq!(next, "pad-path-checks-2");
        assert!(!manifest.exists());
        let archived: Value = read_typed_json(&directory.join(predecessor)).unwrap();
        assert_eq!(archived["native_checks_directory"], first);
        assert_eq!(
            fs::read_to_string(first_check).unwrap(),
            "first pair evidence"
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
