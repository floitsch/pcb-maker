// Copyright (C) 2026 Florian Loitsch.

//! Compare requested pours by running the ordinary layout on the same brief.

use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, process::Command};

const USAGE: &str = "usage: pcb-maker compare-kicad-pour-policies <project-dir> <board-id> <out-dir> <policies.json> [router-config.json|auto] [layout-config.json] [--reference <project-dir>|none] [--placement-seed N]";

pub(super) const HELP: &str = "usage: pcb-maker compare-kicad-pour-policies <project-dir> <board-id> <out-dir> <policies.json> [router-config.json|auto] [layout-config.json] [--reference <project-dir>|none] [--placement-seed N]\n\nEach named entry in policies.json may replace add_pours. An entry with only a name keeps the shared router settings. Every policy gets the full layout budget on the same project and constraints. The output retains each layout and writes pour-policies.json. Eligible results pass the ordinary native layout verdict without new schematic parity findings relative to an available unchanged policy. Fewer parity findings win, then shorter physical track centreline length, then fewer vias. Exit 1 means no policy qualified. See docs/agents.md.";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    name: String,
    add_pours: Option<Vec<Value>>,
}

#[derive(Default, Serialize)]
struct Candidate {
    name: String,
    add_pours: Option<Vec<Value>>,
    log: String,
    exit_code: Option<i32>,
    layout: String,
    verdict: Option<String>,
    result: Option<String>,
    board: Option<String>,
    reason: Option<String>,
    status: Option<String>,
    eligible: bool,
    raw_errors: Option<BTreeMap<String, usize>>,
    new_errors: Value,
    inherited_at_same_place: Value,
    inherited_footprint: Value,
    inherited_input: Value,
    raw_opens: Option<usize>,
    judged_opens: Option<u64>,
    cross_sub_board_opens: Option<usize>,
    parity: Option<usize>,
    adds_parity_findings: Option<bool>,
    #[serde(skip)]
    parity_identities: Option<BTreeMap<String, usize>>,
    constraints_missed: Value,
    unplaced: Value,
    placement_digest: Option<String>,
    final_placement_sha256: Option<String>,
    length_mm: Option<f64>,
    vias: Option<usize>,
    work_seconds: Option<f64>,
}

fn read_report(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn raw_errors(report: &Value) -> Option<BTreeMap<String, usize>> {
    let mut counts = BTreeMap::new();
    for finding in report["violations"].as_array()? {
        if finding["severity"] == "error" {
            *counts
                .entry(finding["type"].as_str()?.to_owned())
                .or_default() += 1;
        }
    }
    Some(counts)
}

fn empty_array(value: &Value) -> bool {
    value.as_array().is_some_and(Vec::is_empty)
}

fn parity_identities(report: &Value) -> Option<BTreeMap<String, usize>> {
    let mut counts = BTreeMap::new();
    for finding in report["schematic_parity"].as_array()? {
        let mut items = finding["items"]
            .as_array()?
            .iter()
            .map(|item| (&item["uuid"], &item["description"]))
            .collect::<Vec<_>>();
        items.sort_by_cached_key(|item| json!(item).to_string());
        // Movable footprints can change pose. Compare electrical identities
        // without treating those coordinate changes as new parity findings.
        let key = json!([
            finding["type"],
            finding["severity"],
            finding["description"],
            items
        ]);
        *counts.entry(key.to_string()).or_default() += 1;
    }
    Some(counts)
}

impl Candidate {
    fn inspect(&mut self, directory: &Path, board_id: &str, has_schematic: bool) {
        let verdict_path = directory.join("verdict.json");
        let Some(verdict) = read_report(&verdict_path) else {
            self.reason = Some("layout did not write a readable verdict".into());
            return;
        };
        self.verdict = Some(path_text(&verdict_path));
        self.status = verdict["status"].as_str().map(str::to_owned);
        self.reason = verdict["reason"].as_str().map(str::to_owned);
        self.new_errors = verdict["new_findings_strict"].clone();
        self.inherited_at_same_place = verdict["preexisting_findings"].clone();
        self.inherited_footprint = verdict["footprint_findings"].clone();
        self.inherited_input = verdict["input_findings"].clone();
        self.judged_opens = verdict["kicad_unconnected_items"].as_u64();
        self.constraints_missed = verdict["constraints_missed"].clone();
        self.unplaced = verdict["unplaced"].clone();
        self.work_seconds = verdict["work_seconds"].as_f64();
        self.placement_digest = read_report(&directory.join("board-layout.json"))
            .and_then(|layout| layout["placement_digest"].as_str().map(str::to_owned));
        let result = directory.join("result");
        let board = result.join(format!("{board_id}.kicad_pcb"));
        if board.is_file() {
            self.board = Some(path_text(&board));
            self.result = Some(path_text(&result));
            match pcb_kicad::inspect_kicad_board(&board) {
                Ok(stats) => {
                    self.length_mm = Some(stats.physical_copper.physical_centerline_length_mm);
                    self.vias = Some(stats.vias);
                    self.final_placement_sha256 = Some(stats.component_placement_sha256);
                }
                Err(error) => {
                    self.reason = Some(format!("cannot measure the result board: {error}"))
                }
            }
        }
        if let Some(drc) = read_report(&result.join("drc.json")) {
            self.raw_errors = raw_errors(&drc);
            self.cross_sub_board_opens = drc["unconnected_across_sub_boards"]
                .as_array()
                .map(Vec::len);
            self.raw_opens = drc["unconnected_items"]
                .as_array()
                .map(|opens| opens.len() + self.cross_sub_board_opens.unwrap_or(0));
            if has_schematic {
                self.parity_identities = parity_identities(&drc);
                self.parity = self
                    .parity_identities
                    .as_ref()
                    .map(|counts| counts.values().sum());
            }
        }
        self.eligible = self.exit_code == Some(0)
            && verdict["status"] == "complete"
            && verdict["kicad_checked"] == true
            && self.judged_opens == Some(0)
            && empty_array(&self.constraints_missed)
            && empty_array(&self.unplaced)
            && empty_array(&verdict["cut"])
            && self
                .new_errors
                .as_object()
                .is_some_and(|errors| errors.is_empty())
            && self.raw_errors.is_some()
            && self.raw_opens.is_some()
            && (!has_schematic || self.parity.is_some())
            && self.length_mm.is_some()
            && self.vias.is_some();
    }
}

fn mark_parity_changes(candidates: &mut [Candidate]) -> Option<String> {
    let baseline = candidates
        .iter()
        .filter(|candidate| candidate.add_pours.is_none())
        .find_map(|candidate| {
            Some((candidate.name.clone(), candidate.parity_identities.clone()?))
        })?;
    for candidate in candidates {
        candidate.adds_parity_findings = candidate.parity_identities.as_ref().map(|after| {
            after
                .iter()
                .any(|(finding, count)| baseline.1.get(finding).is_none_or(|before| before < count))
        });
        candidate.eligible &= candidate.adds_parity_findings != Some(true);
    }
    Some(baseline.0)
}

fn select_eligible(candidates: &[Candidate]) -> Option<&Candidate> {
    candidates
        .iter()
        .filter(|candidate| candidate.eligible)
        .min_by(|a, b| {
            a.parity
                .cmp(&b.parity)
                .then_with(|| a.length_mm.unwrap().total_cmp(&b.length_mm.unwrap()))
                .then_with(|| a.vias.cmp(&b.vias))
        })
}

pub(super) fn compare(arguments: Vec<String>) -> Result<KiCadRunStatus, Failure> {
    let (positional, flags) = split_flags(arguments, &["--reference", "--placement-seed"], USAGE)?;
    positional_count(&positional, 4..=6, USAGE)?;
    let (source, board_id, output, policies_path) = (
        &positional[0],
        &positional[1],
        &positional[2],
        &positional[3],
    );
    check_project(source, board_id)?;
    check_fresh_output(output)?;
    reference_option(&flags, None, board_id)?;
    if flags
        .get("--placement-seed")
        .is_some_and(|seed| seed.parse::<u64>().is_err())
    {
        return Err(Failure::Usage(
            "--placement-seed needs a non-negative 64-bit integer".into(),
        ));
    }
    let policies: Vec<Policy> = read_json(policies_path)?;
    let names: BTreeSet<_> = policies.iter().map(|policy| policy.name.as_str()).collect();
    if names.len() < 2
        || names.len() != policies.len()
        || names.iter().any(|name| name.trim().is_empty())
    {
        return Err(Failure::Usage(
            "give at least two distinct, non-empty policy names".into(),
        ));
    }
    let layout_path = positional.get(5);
    let layout: pcb_kicad::KiCadBoardLayoutConfig = layout_path
        .map(|path| read_json(path))
        .transpose()?
        .unwrap_or_default();
    if !layout.total_seconds.is_finite() || layout.total_seconds <= 0.0 {
        return Err(Failure::Usage(
            "total_seconds must be a positive work budget per policy".into(),
        ));
    }
    let router_path = positional.get(4).map(String::as_str);
    let router =
        crate::board_router_config(router_path, source, board_id).map_err(Failure::Usage)?;
    if router.skip_native_verification {
        return Err(Failure::Usage(
            "pour comparisons require native KiCad verification".into(),
        ));
    }
    let router_file = router_path.filter(|path| *path != "auto");
    let common: Value = match router_file {
        Some(path) => read_json(path)?,
        None => json!({}),
    };
    // Validate the same request type the normal router accepts.
    for policy in &policies {
        if let Some(pours) = &policy.add_pours {
            let mut validated = router.clone();
            validated.add_pours = serde_json::from_value(Value::Array(pours.clone()))
                .map_err(|error| Failure::Usage(format!("{}: {error}", policy.name)))?;
        }
    }
    check_kicad(false)?;
    calibrate_work_rate(Some(&router))?;
    let rate = pcb_kicad::work_rate();
    let pinned = format!(
        "{},{}",
        rate.expansions_per_second, rate.placer_work_per_second
    );
    let executable =
        std::env::current_exe().map_err(|error| Failure::Internal(error.to_string()))?;
    let output = Path::new(output);
    fs::create_dir_all(output).map_err(|error| Failure::Internal(error.to_string()))?;
    let started = Instant::now();
    let mut candidates = Vec::with_capacity(policies.len());
    let has_schematic = Path::new(source)
        .join(format!("{board_id}.kicad_sch"))
        .is_file();
    for (index, policy) in policies.into_iter().enumerate() {
        let directory = output.join(format!("policy-{}", index + 1));
        let log_path = directory.with_extension("log");
        let log =
            fs::File::create(&log_path).map_err(|error| Failure::Internal(error.to_string()))?;
        let mut child = Command::new(&executable);
        child
            .arg("layout-kicad-board")
            .arg(source)
            .arg(board_id)
            .arg(&directory);
        if let Some(pours) = &policy.add_pours {
            let config_path = directory.with_extension("router.json");
            let mut config = common.clone();
            config["add_pours"] = Value::Array(pours.clone());
            if router_file.is_none()
                && let Some(seeds) = router.seeds
            {
                config["seeds"] = json!(seeds);
            }
            write_json(&config_path, &config)?;
            child.arg(config_path);
        } else {
            child.arg(router_path.unwrap_or("auto"));
        }
        if let Some(path) = layout_path {
            child.arg(path);
        }
        for (name, value) in &flags {
            child.arg(name).arg(value);
        }
        println!(
            "policy {} at {:.1} work seconds",
            policy.name, layout.total_seconds
        );
        let status = child
            .env(pcb_kicad::WORK_RATE_VARIABLE, &pinned)
            .stdout(
                log.try_clone()
                    .map_err(|error| Failure::Internal(error.to_string()))?,
            )
            .stderr(log)
            .status();
        let mut candidate = Candidate {
            name: policy.name,
            add_pours: policy.add_pours,
            layout: path_text(&directory),
            log: path_text(&log_path),
            ..Default::default()
        };
        match status {
            Ok(status) => {
                candidate.exit_code = status.code();
                candidate.inspect(&directory, board_id, has_schematic);
            }
            Err(error) => candidate.reason = Some(format!("could not launch layout: {error}")),
        }
        candidates.push(candidate);
    }
    let parity_baseline = mark_parity_changes(&mut candidates);
    let selected = select_eligible(&candidates);
    let path = output.join("pour-policies.json");
    let report = json!({
        "status": if selected.is_some() { "complete" } else { "no_eligible_candidate" },
        "project": path_text(Path::new(source)),
        "board_id": board_id,
        "layout_config": layout_path.map(|path| path_text(Path::new(path))),
        "work_budget_per_policy_seconds": layout.total_seconds,
        "work_budget_total_seconds": layout.total_seconds * candidates.len() as f64,
        "reported_work_seconds": candidates.iter().filter_map(|candidate| candidate.work_seconds).sum::<f64>(),
        "runs_with_reported_work": candidates.iter().filter(|candidate| candidate.work_seconds.is_some()).count(),
        "wall_seconds": started.elapsed().as_secs_f64(),
        "work_rate": pinned,
        "has_schematic": has_schematic,
        "parity_comparison_baseline": parity_baseline,
        "selected": selected,
        "candidates": candidates,
    });
    write_json(&path, &report)?;
    if let Some(selected) = selected {
        println!(
            "selected {} at {}",
            selected.name,
            selected.result.as_deref().unwrap()
        );
    } else {
        println!(
            "no policy qualified under the native layout verdict. See {}",
            path.display()
        );
    }
    Ok(if selected.is_some() {
        KiCadRunStatus::Complete
    } else {
        KiCadRunStatus::Incomplete
    })
}

fn write_json(path: &Path, value: &Value) -> Result<(), Failure> {
    pcb_kicad::write_atomically(path, format!("{value:#}\n").as_bytes()).map_err(Failure::Internal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_selection_prefers_eligibility_then_parity_length_and_vias() {
        let pick = |name: &str, eligible, parity, length_mm, vias| Candidate {
            name: name.into(),
            eligible,
            parity: Some(parity),
            length_mm: Some(length_mm),
            vias: Some(vias),
            ..Default::default()
        };
        let candidates = [
            pick("open", false, 0, 4.0, 0),
            pick("parity", true, 1, 6.0, 0),
            pick("short", true, 0, 8.0, 2),
            pick("long", true, 0, 9.0, 0),
            pick("short-one-via", true, 0, 8.0, 1),
        ];
        assert_eq!(select_eligible(&candidates).unwrap().name, "short-one-via");
        assert!(select_eligible(&candidates[..1]).is_none());
    }

    #[test]
    fn parity_uses_identities_and_counts_and_ignores_moved_item_positions() {
        let finding = |uuid, x| json!({"type": "net_conflict", "severity": "error", "description": "pin differs", "items": [{"uuid": uuid, "description": "U1", "pos": {"x": x, "y": 10.0}}]});
        let candidate = |name: &str, add_pours, findings| Candidate {
            name: name.into(),
            add_pours,
            eligible: true,
            parity_identities: parity_identities(&json!({"schematic_parity": findings})),
            ..Default::default()
        };
        let mut results = [
            candidate("same", None, vec![finding("one", 10.0)]),
            candidate("moved", Some(vec![]), vec![finding("one", 11.0)]),
            candidate(
                "duplicate",
                Some(vec![]),
                vec![finding("one", 9.0), finding("one", 9.0)],
            ),
            candidate("different", Some(vec![]), vec![finding("two", 10.0)]),
        ];
        assert_eq!(mark_parity_changes(&mut results).as_deref(), Some("same"));
        assert_eq!(
            results.map(|candidate| candidate.eligible),
            [true, true, false, false]
        );
    }
}
