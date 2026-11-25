// Copyright (C) 2026 Florian Loitsch.

//! Native evidence for keeping an already connected route or plane.

use super::*;
use pcb_kicad_check::native_report::{check_kept_route_input, run_kicad_report};
use pcb_kicad_file::board_stats::{inspect_kicad_routable_connections, is_rule_area, saved_copper};
use serde_json::Value;

const CHECK_WALL: std::time::Duration = std::time::Duration::from_secs(20);

/// Existing copper KiCad checked using the input's project and rules.
#[derive(Clone, Debug, Serialize)]
pub struct KiCadInputConnectivity {
    pub status: &'static str,
    pub prior_refilled_open_items: usize,
    pub refilled_open_items: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub protected_planes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial_rejection: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial_refilled_open_items: Option<usize>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub trial_new_findings: BTreeMap<String, usize>,
    /// Second raw KiCad check of the original footprint-library warnings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_library_sample: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejected_drc: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored_open_items: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored_errors: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub retry_argv: Vec<String>,
}

pub(super) struct NativeInput {
    stage: PathBuf,
    board_id: String,
    report: Value,
    parity_requested: bool,
    open_nets: BTreeMap<String, usize>,
    pub protected_planes: BTreeSet<String>,
    pub check_seconds: f64,
}

pub(super) struct TrialRejection {
    pub reason: String,
    opens: Option<usize>,
    new_findings: BTreeMap<String, usize>,
}

impl From<String> for TrialRejection {
    fn from(reason: String) -> Self {
        Self {
            reason,
            opens: None,
            new_findings: BTreeMap::new(),
        }
    }
}

impl From<&str> for TrialRejection {
    fn from(reason: &str) -> Self {
        reason.to_owned().into()
    }
}

impl Drop for NativeInput {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.stage);
    }
}

fn deadline(config: &KiCadBoardRouterConfig) -> std::time::Instant {
    let limit = std::time::Instant::now() + CHECK_WALL;
    config.deadline.map_or(limit, |wall| limit.min(wall))
}

pub(super) fn probe_input(
    pcb: &Expr,
    source_bytes: &str,
    source: &Path,
    board_id: &str,
    output: &Path,
    config: &KiCadBoardRouterConfig,
    checks: &mut usize,
) -> Option<NativeInput> {
    if config.skip_native_verification
        || config.kicad_kept.is_some()
        || config.route_reuse_clearance_unverified
        || config.route_reuse_preserve_input_layers
        || config.pours == KiCadPourMode::Tracks
        || count_tracks(pcb) == [0, 0, 0]
        || !pcb
            .children()
            .iter()
            .any(|item| item.head() == Some("zone") && !is_rule_area(item))
    {
        return None;
    }
    let project: Value = read_typed_json(&source.join(format!("{board_id}.kicad_pro"))).ok()?;
    if !connectivity_enabled(&project) {
        return None;
    }

    let deadline = deadline(config);
    let expired = || std::time::Instant::now() >= deadline;
    if expired() {
        return None;
    }
    let started = std::time::Instant::now();
    let stage = temporary_next_to(output);
    let checked: Result<Option<NativeInput>, String> = (|| {
        copy_directory_tree(source, &stage)?;
        pcb_kicad_check::native_report::remove_stale_board_run_reports(&stage)?;
        let board = stage.join(format!("{board_id}.kicad_pcb"));
        let parity = board.with_extension("kicad_sch").is_file();
        let before = companions(&board);
        *checks += 1;
        let report =
            check_kept_route_input(&board, &stage.join("drc.json"), parity, true, expired)?;
        if fs::read_to_string(&board).map_err(|error| error.to_string())? != source_bytes
            || companions(&board) != before
        {
            return Err("native verification changed the input project".into());
        }
        if !clean_findings(&report) {
            return Ok(None);
        }
        let Some(open_nets) = open_nets(pcb, &report) else {
            return Ok(None);
        };
        let protected_planes = if open_nets.is_empty() {
            BTreeSet::new()
        } else {
            if outline_pieces::Pieces::of_board(pcb)?.is_some() {
                return Ok(None);
            }
            let layers = LayerTable::from_pcb(pcb)?;
            pours(pcb, &layers)?
                .into_iter()
                .map(|pour| pour.net)
                .filter(|name| {
                    !open_nets.contains_key(name)
                        && (!config.diff_pairs_on()
                            || crate::project_rules::diff_pair_of(name).is_none())
                })
                .collect()
        };
        if !open_nets.is_empty() && protected_planes.is_empty() {
            return Ok(None);
        }
        Ok(Some(NativeInput {
            stage: stage.clone(),
            board_id: board_id.into(),
            report,
            parity_requested: parity,
            open_nets,
            protected_planes,
            check_seconds: started.elapsed().as_secs_f64(),
        }))
    })();
    match checked {
        Ok(Some(input)) => Some(input),
        Ok(None) => {
            let _ = fs::remove_dir_all(stage);
            None
        }
        Err(error) => {
            let _ = fs::remove_dir_all(stage);
            eprintln!("existing copper: input connectivity preflight unavailable ({error})");
            None
        }
    }
}

impl NativeInput {
    pub fn complete(&self) -> bool {
        self.open_nets.is_empty()
    }

    pub fn original_result(
        &self,
        pcb: &Expr,
        output: &Path,
        config: &KiCadBoardRouterConfig,
        checks: &mut usize,
        rejection: Option<TrialRejection>,
    ) -> Result<KiCadBoardRouterResult, String> {
        let started = std::time::Instant::now();
        let board = self.stage.join(format!("{}.kicad_pcb", self.board_id));
        let pcb_bytes = fs::read(&board).map_err(|error| error.to_string())?;
        let project = companions(&board);
        let connectivity = self.report_connectivity(
            &board,
            output,
            config,
            checks,
            opens(&self.report)?,
            rejection,
        );
        let erc_path = self.stage.join("erc.json");
        let schematic = board.with_extension("kicad_sch");
        if schematic.is_file() {
            run_kicad_report(
                &[
                    "sch",
                    "erc",
                    "--format",
                    "json",
                    "--severity-all",
                    "--output",
                ],
                &erc_path,
                &schematic,
            )?;
        } else {
            write_atomically(
                &erc_path,
                &serde_json::to_vec(&serde_json::json!({
                    "source": format!("{}.kicad_sch", self.board_id),
                    "date": self.report["date"],
                    "kicad_version": self.report["kicad_version"],
                    "sheets": [],
                }))
                .map_err(|error| error.to_string())?,
            )?;
        }
        let native = verification::record_verified_additive_board(
            &self.stage,
            &self.board_id,
            self.parity_requested,
        )?;
        if fs::read(&board).map_err(|error| error.to_string())? != pcb_bytes
            || companions(&board) != project
        {
            return Err("native verification changed the original project".into());
        }
        let copper = saved_copper(pcb)?;
        let connections = inspect_kicad_routable_connections(&board)?;
        let board = lower(pcb, config, true)?.board;
        let names = NetNames::of(pcb);
        let codes = kicad_nets::net_codes(pcb);
        let kept = KiCadKeptCopper {
            segments: copper.segments,
            arcs: copper.arcs,
            vias: copper.vias,
            connections_made: connections
                .iter()
                .filter(|net| !self.open_nets.contains_key(&net.connection))
                .map(|net| {
                    net.electrical_terminal_count
                        .unwrap_or(net.distinct_pad_centers)
                        .saturating_sub(1)
                })
                .sum(),
            nets: super::route_reuse::track_items(pcb)
                .filter_map(|item| kicad_nets::item_net(item, &codes))
                .map(|name| names.normalize(name).to_owned())
                .filter(|name| !name.is_empty())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            nets_complete: connections
                .iter()
                .filter(|net| !self.open_nets.contains_key(&net.connection))
                .map(|net| net.connection.clone())
                .collect(),
            retained_pad_tips: (!config.retained_pad_tips.is_empty()).then(|| {
                super::kept::RetainedPadTips {
                    used: Vec::new(),
                    ignored: config
                        .retained_pad_tips
                        .iter()
                        .map(|pad| {
                            (
                                pad.clone(),
                                "the selected PCB is the unchanged input".into(),
                            )
                        })
                        .collect(),
                }
            }),
            ..Default::default()
        };
        let nets = connections
            .into_iter()
            .map(|net| {
                let open = self.open_nets.get(&net.connection).copied().unwrap_or(0);
                KiCadBoardRouterNet {
                    connection: net.connection,
                    terminals: net
                        .electrical_terminal_count
                        .unwrap_or(net.distinct_pad_centers),
                    status: if open > 0 { "partial" } else { "routed" }.into(),
                    unconnected_terminals: open,
                    vias: 0,
                    length_mm: 0.0,
                    // KiCad's connectivity of the input: no router facts.
                    open: Vec::new(),
                    repair: None,
                    open_repair: None,
                    copy: None,
                    pads: Vec::new(),
                }
            })
            .collect::<Vec<_>>();
        let work = *checks as f64 * config.kicad_check_work.unwrap_or(0.0);
        let result = KiCadBoardRouterResult {
            board_id: self.board_id.clone(),
            routable_connections: nets.len(),
            routed_connections: kept.nets_complete.len(),
            unconnected_terminals: connectivity.refilled_open_items,
            saved_copper: Some(copper),
            input_connectivity: Some(connectivity),
            native: Some(native),
            nets,
            native_verification_seconds: self.check_seconds + started.elapsed().as_secs_f64(),
            kicad_checks_charged: *checks,
            kicad_check_work_seconds: work,
            work_seconds: work,
            pours: "existing".into(),
            kept_copper: Some(kept),
            rules: KiCadRulesSummary::of(&board, config),
            ..Default::default()
        };
        Ok(result)
    }

    pub fn publish_original(
        &self,
        output: &Path,
        result: &mut KiCadBoardRouterResult,
        rejected: Option<&Path>,
    ) -> Result<(), String> {
        if let Some(trial) = rejected {
            const WITNESS: &str = "drc-rejected-protected-planes.json";
            let report = trial.join("drc-protected-planes.json");
            if report.is_file() {
                fs::copy(report, self.stage.join(WITNESS)).map_err(|error| error.to_string())?;
                if let Some(evidence) = result.input_connectivity.as_mut() {
                    evidence.rejected_drc = Some(WITNESS);
                }
            }
        }
        write_atomically(
            &self.stage.join("board-router.json"),
            &serde_json::to_vec_pretty(&result).map_err(|error| error.to_string())?,
        )?;
        super::route::publish_new_route(&self.stage, output)
    }

    pub fn confirm_trial(
        &self,
        original: &Expr,
        trial: &Path,
        output: &Path,
        config: &KiCadBoardRouterConfig,
        checks: &mut usize,
    ) -> Result<KiCadInputConnectivity, TrialRejection> {
        let board = trial.join(format!("{}.kicad_pcb", self.board_id));
        let text = fs::read_to_string(&board).map_err(|error| error.to_string())?;
        let project = companions(&board);
        if project != companions(&self.stage.join(format!("{}.kicad_pcb", self.board_id))) {
            return Err("trial changed the native project inputs".into());
        }
        let pcb = parse(&text)?;
        let net_names = NetNames::of(original);
        let before = super::route_reuse::copper_census(original);
        let after = super::route_reuse::copper_census(&pcb);
        if before
            .iter()
            .any(|(item, count)| after.get(item).is_none_or(|kept| kept < count))
        {
            return Err("trial changed existing copper".into());
        }
        if after.iter().any(|(item, count)| {
            self.protected_planes.contains(net_names.normalize(&item.0))
                && before.get(item).is_none_or(|known| known < count)
        }) {
            return Err("trial added copper to a protected plane net".into());
        }
        let limit = deadline(config);
        let expired = || std::time::Instant::now() >= limit;
        if expired() {
            return Err("the wall guard expired before validating the protected planes".into());
        }
        let path = trial.join("drc-protected-planes.json");
        *checks += 1;
        let report = check_kept_route_input(
            &board,
            &path,
            board.with_extension("kicad_sch").is_file(),
            true,
            expired,
        )?;
        let after_opens = opens(&report)?;
        if !crate::open_repair::same_check_policy(&self.report, &report) {
            return Err("the native check policy changed between the input and trial".into());
        }
        let (mut new_findings, first_error) = new_findings(&self.report, &report)?;
        let mut source_library_sample = None;
        if after_opens < opens(&self.report)?
            && clean_findings(&report)
            && !new_findings.is_empty()
            && new_findings.keys().all(|kind| {
                matches!(
                    kind.as_str(),
                    "lib_footprint_issues" | "lib_footprint_mismatch"
                ) && [&self.report, &report].into_iter().all(|r| {
                    crate::open_repair::finding_types_at_cap(r, "violations")
                        .contains(&kind.as_str())
                })
            })
            && same_footprints(original, &pcb)
            && same_library_table(&self.stage, trial)
            && !expired()
        {
            const SAMPLE: &str = "drc-original-library-sample.json";
            *checks += 1;
            let source = self.stage.join(format!("{}.kicad_pcb", self.board_id));
            if let Ok(sample) = pcb_kicad_check::native_report::resample_footprint_warnings(
                &source,
                &trial.join(SAMPLE),
                expired,
            ) && crate::open_repair::same_check_policy(&self.report, &sample)
                && clean_findings(&sample)
                && opens(&sample)? == opens(&self.report)?
                && library_witnesses_explained(&self.report, &sample, &report)?
            {
                fs::copy(
                    self.stage.join("drc.json"),
                    trial.join("drc-original-before-trial.json"),
                )
                .map_err(|e| e.to_string())?;
                new_findings.clear();
                source_library_sample = Some(SAMPLE);
            }
        }
        let reason = if !new_findings.is_empty() {
            let findings: Vec<_> = new_findings
                .iter()
                .map(|(kind, count)| format!("{kind} ({count})"))
                .collect();
            let mut reason = format!(
                "KiCad reported findings absent from the input report: {}",
                findings.join(", ")
            );
            if let Some(error) = first_error {
                reason.push_str(". ");
                reason.push_str(&error);
            }
            Some(reason)
        } else if !clean_findings(&report) {
            Some("the trial's native finding census could not be certified".into())
        } else if let Some(after_nets) = open_nets(&pcb, &report) {
            if let Some((net, count)) = after_nets.iter().find(|(net, count)| {
                **count > self.open_nets.get(*net).copied().unwrap_or_default()
            }) {
                Some(format!("trial increased opens on {net} to {count}"))
            } else if after_opens >= opens(&self.report)? {
                Some("trial did not reduce native opens".into())
            } else {
                None
            }
        } else {
            Some("the trial's native opens could not be attributed".into())
        };
        if let Some(reason) = reason {
            return Err(TrialRejection {
                reason,
                opens: Some(after_opens),
                new_findings,
            });
        }
        let mut evidence =
            self.report_connectivity(&board, output, config, checks, after_opens, None);
        evidence.source_library_sample = source_library_sample;
        if fs::read_to_string(&board).map_err(|error| error.to_string())? != text
            || companions(&board) != project
        {
            return Err("the candidate changed during native recheck".into());
        }
        Ok(evidence)
    }

    fn report_connectivity(
        &self,
        board: &Path,
        output: &Path,
        config: &KiCadBoardRouterConfig,
        checks: &mut usize,
        refilled_open_items: usize,
        trial: Option<TrialRejection>,
    ) -> KiCadInputConnectivity {
        let limit = deadline(config);
        let expired = || std::time::Instant::now() >= limit;
        let stored = if expired() {
            None
        } else {
            *checks += 1;
            check_kept_route_input(
                board,
                &board.with_file_name("drc-stored-input.json"),
                board.with_extension("kicad_sch").is_file(),
                false,
                expired,
            )
            .ok()
        };
        let stored_open_items = stored.as_ref().and_then(|report| opens(report).ok());
        let stored_errors = stored.as_ref().and_then(error_count);
        let retry_argv = if refilled_open_items == 0
            && (stored_open_items != Some(0) || stored_errors != Some(0))
        {
            vec![
                "pcb-maker".into(),
                "drc-kicad-board".into(),
                output.display().to_string(),
                self.board_id.clone(),
                output
                    .with_extension("filled-drc.json")
                    .display()
                    .to_string(),
                "--save-filled".into(),
                output.with_extension("filled").display().to_string(),
            ]
        } else {
            Vec::new()
        };
        let (trial_rejection, trial_refilled_open_items, trial_new_findings) = match trial {
            Some(trial) => (Some(trial.reason), trial.opens, trial.new_findings),
            None => (None, None, BTreeMap::new()),
        };
        KiCadInputConnectivity {
            status: if trial_rejection.is_some() {
                "trial_rejected"
            } else if self.complete() {
                "unchanged"
            } else {
                "planes_preserved"
            },
            prior_refilled_open_items: opens(&self.report).unwrap_or_default(),
            refilled_open_items,
            stored_open_items,
            stored_errors,
            retry_argv,
            trial_rejection,
            trial_refilled_open_items,
            trial_new_findings,
            source_library_sample: None,
            rejected_drc: None,
            protected_planes: self.protected_planes.iter().cloned().collect(),
        }
    }
}

fn companions(board: &Path) -> [Option<Vec<u8>>; 3] {
    ["kicad_pro", "kicad_dru", "kicad_sch"].map(|ext| fs::read(board.with_extension(ext)).ok())
}

fn connectivity_enabled(project: &Value) -> bool {
    let severity = &project["board"]["design_settings"]["rule_severities"]["unconnected_items"];
    severity.is_null() || matches!(severity.as_str(), Some("error" | "warning"))
}

fn error_count(report: &Value) -> Option<usize> {
    Some(
        report["violations"]
            .as_array()?
            .iter()
            .filter(|issue| issue["severity"] == "error" && issue["excluded"] != true)
            .count(),
    )
}

fn clean_findings(report: &Value) -> bool {
    error_count(report) == Some(0)
        && !report["ignored_checks"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|check| check == "unconnected_items" || check["key"] == "unconnected_items")
        && report["schematic_parity"]
            .as_array()
            .is_none_or(Vec::is_empty)
        && crate::open_repair::finding_types_at_cap(report, "violations")
            .iter()
            .all(|kind| matches!(*kind, "lib_footprint_issues" | "lib_footprint_mismatch"))
}

fn same_library_table(source: &Path, trial: &Path) -> bool {
    match (
        fs::read(source.join("fp-lib-table")),
        fs::read(trial.join("fp-lib-table")),
    ) {
        (Ok(a), Ok(b)) => a == b,
        (Err(a), Err(b)) => [a, b]
            .iter()
            .all(|e| e.kind() == std::io::ErrorKind::NotFound),
        _ => false,
    }
}

fn same_footprints(original: &Expr, trial: &Expr) -> bool {
    let forms = |pcb: &Expr| {
        let mut items: Vec<_> = pcb
            .children()
            .iter()
            .filter(|f| f.head() == Some("footprint"))
            .map(pcb_kicad_file::kicad_sexpr::encode)
            .collect();
        items.sort_unstable();
        items
    };
    forms(original) == forms(trial)
}

/// A changed witness must be present, including its full multiplicity, in
/// an actual source check. Two partial samples cannot be added together.
fn library_witnesses_explained(
    original: &Value,
    sample: &Value,
    trial: &Value,
) -> Result<bool, String> {
    let counts = |report| crate::open_repair::finding_counts(report, "violations");
    let original = counts(original)?;
    let sample = counts(sample)?;
    for (key, count) in counts(trial)? {
        if count <= original.get(&key).copied().unwrap_or_default() {
            continue;
        }
        let finding = serde_json::from_str(&key).map_err(|e| e.to_string())?;
        if !pcb_kicad_check::native_report::is_library_metadata_warning(&finding)
            || count > sample.get(&key).copied().unwrap_or_default()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn new_findings(
    before: &Value,
    after: &Value,
) -> Result<(BTreeMap<String, usize>, Option<String>), String> {
    let mut types = BTreeMap::new();
    let mut first_error = None;
    for field in ["violations", "schematic_parity"] {
        let prior = crate::open_repair::finding_counts(before, field)?;
        for (key, count) in crate::open_repair::finding_counts(after, field)? {
            let added = count.saturating_sub(prior.get(&key).copied().unwrap_or_default());
            if added > 0 {
                let finding: Value =
                    serde_json::from_str(&key).map_err(|error| error.to_string())?;
                let kind = finding["type"].as_str().unwrap_or("unknown");
                *types.entry(kind.to_owned()).or_default() += added;
                if first_error.is_none()
                    && finding["severity"] == "error"
                    && let Some(description) = finding["description"].as_str()
                {
                    let mut witness = description.to_owned();
                    for item in finding["items"].as_array().into_iter().flatten().take(2) {
                        if let Some(item) = item["description"].as_str() {
                            witness.push_str(" — ");
                            witness.push_str(item);
                        }
                    }
                    first_error = Some(witness);
                }
            }
        }
    }
    Ok((types, first_error))
}

fn opens(report: &Value) -> Result<usize, String> {
    let opens = report["unconnected_items"]
        .as_array()
        .ok_or("missing native opens")?
        .len();
    let cap = if report["kicad_version"]
        .as_str()
        .is_some_and(|v| v.starts_with("10.0."))
    {
        499
    } else {
        199
    };
    if opens >= cap {
        return Err("native unconnected report is capped".into());
    }
    Ok(opens)
}

/// KiCad's unconnected items of `report` on `pcb` per net (its connection
/// name); None where an item names no single net.
pub(super) fn open_nets(pcb: &Expr, report: &Value) -> Option<BTreeMap<String, usize>> {
    let count = opens(report).ok()?;
    if count == 0 {
        return Some(BTreeMap::new());
    }
    let names = NetNames::of(pcb);
    let codes = kicad_nets::net_codes(pcb);
    let observations = crate::open_repair::native_opens(pcb, report, &codes);
    if observations.len() != count {
        return None;
    }
    let mut counts = BTreeMap::new();
    for (name, _) in observations {
        *counts.entry(names.normalize(name).to_owned()).or_default() += 1;
    }
    Some(counts)
}

/// Keep original pad and copper obstacles while omitting redundant searches.
pub(super) fn protect_planes(board: &mut pcb_router::Board, config: &KiCadBoardRouterConfig) {
    if config.native_closed_planes.is_empty() {
        return;
    }
    for net in &mut board.nets {
        if config.native_closed_planes.contains(&net.name) {
            net.terminals.clear();
        }
    }
    let unprotected = |net: pcb_router::NetId| {
        !config
            .native_closed_planes
            .contains(&board.nets[net as usize].name)
    };
    board.merged_pads.retain(|pad| unprotected(pad.net));
    board
        .terminal_copper
        .retain(|copper| unprotected(copper.net));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_second_source_sample_can_only_confirm_identical_library_warnings() {
        let warn =
            json!({"type":"lib_footprint_issues","severity":"warning","items":[{"uuid":"a"}]});
        let other =
            json!({"type":"lib_footprint_issues","severity":"warning","items":[{"uuid":"b"}]});
        let error = json!({"type":"clearance","severity":"error","items":[{"uuid":"b"}]});
        let a = json!({"violations":[warn]});
        let b = json!({"violations":[other]});
        assert!(library_witnesses_explained(&a, &b, &json!({"violations":[warn,other]})).unwrap());
        assert!(!library_witnesses_explained(&a, &a, &b).unwrap());
        assert!(
            !library_witnesses_explained(&a, &b, &json!({"violations":[other,other]})).unwrap()
        );
        let nonlibrary = json!({"violations":[error]});
        assert!(!library_witnesses_explained(&a, &nonlibrary, &nonlibrary).unwrap());
        let old = parse("(kicad_pcb (footprint X (at 1 2)) (footprint Y))").unwrap();
        let moved = parse("(kicad_pcb (footprint Y) (footprint X (at 1 3)))").unwrap();
        assert!(!same_footprints(&old, &moved));
    }

    #[test]
    fn trial_feedback_names_the_new_error_and_its_items() {
        let prior = json!({"type":"clearance","severity":"error","description":"existing"});
        let warning = json!({"type":"library","severity":"warning","description":"new warning"});
        let error = json!({"type":"diff_pair_uncoupled_length_too_long","severity":"error",
            "description":"maximum 10 mm, actual 45 mm",
            "items":[{"description":"Track [TX_P]"},{"description":"Track [TX_N]"}]});
        let input = json!({"violations":[prior]});
        let trial = json!({"violations":[prior,warning,error]});
        let (counts, witness) = new_findings(&input, &trial).unwrap();
        assert_eq!(counts.len(), 2);
        let witness = witness.unwrap();
        for detail in ["maximum 10 mm, actual 45 mm", "[TX_P]", "[TX_N]"] {
            assert!(witness.contains(detail));
        }
        assert_eq!(new_findings(&trial, &trial).unwrap(), Default::default());
    }

    #[test]
    fn native_input_requires_enabled_connectivity_and_no_refilled_hard_findings() {
        let project = |severity: Value| json!({"board":{"design_settings":{"rule_severities":{"unconnected_items":severity}}}});
        assert!(connectivity_enabled(&project(json!(null))));
        assert!(connectivity_enabled(&project(json!("warning"))));
        assert!(!connectivity_enabled(&project(json!("ignore"))));
        assert!(!connectivity_enabled(&project(json!(17))));

        let mut report = json!({"kicad_version":"10.0.8", "violations":[], "unconnected_items":[]});
        assert!(clean_findings(&report));
        report["ignored_checks"] = json!([{"key":"unconnected_items"}]);
        assert!(!clean_findings(&report));
        report["ignored_checks"] = json!([]);
        report["violations"] = json!([{"type":"shorting_items","severity":"error"}]);
        assert!(!clean_findings(&report));
        report.as_object_mut().unwrap().remove("unconnected_items");
        assert!(opens(&report).is_err());
    }

    #[test]
    fn an_open_must_bind_both_witnesses_to_one_native_net() {
        let pcb = parse(
            r#"(kicad_pcb
            (footprint (pad "1" (net "SIGNAL") (uuid "AAA"))
                       (pad "2" (net "SIGNAL") (uuid "BBB"))))"#,
        )
        .unwrap();
        let mut checked = json!({"unconnected_items":[{"items":[{"uuid":"aaa"},{"uuid":"bbb"}]}]});
        assert_eq!(
            open_nets(&pcb, &checked).unwrap(),
            [("SIGNAL".into(), 1)].into()
        );
        checked["unconnected_items"][0]["items"][1]["uuid"] = json!("not-present");
        assert!(open_nets(&pcb, &checked).is_none());
    }
}
