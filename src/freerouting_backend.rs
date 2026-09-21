// Copyright (C) 2026 Toit contributors.

//! Native-project frontend for the same source-preserving external pipeline
//! used by the whole-board experiments. Physical rules are never CLI options.

use serde::Deserialize;
use serde_json::Value;
use std::{
    env, fs,
    path::{Component, Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    jar: PathBuf,
    java: PathBuf,
    #[serde(default = "default_python")]
    python: PathBuf,
    maximum_seconds: u64,
    maximum_passes: u32,
    pipeline_maximum_seconds: u64,
    #[serde(default)]
    island_bridges: Option<pcb_kicad::KiCadIslandBridgeConfig>,
}

fn default_python() -> PathBuf {
    PathBuf::from("/usr/bin/python3")
}

impl Config {
    fn validate(&self) -> Result<(), String> {
        if self.maximum_seconds == 0
            || self.maximum_passes == 0
            || self.pipeline_maximum_seconds < self.maximum_seconds
        {
            return Err("external routing requires positive search/pass bounds and a pipeline bound at least as large as the search bound".into());
        }
        if let Some(bridge) = &self.island_bridges {
            if bridge.maximum_attempts == 0 || bridge.maximum_pair_candidates == 0 {
                return Err("island bridges require positive attempt and pair bounds".into());
            }
            if !bridge.routing.connection_rules.is_empty()
                || bridge.routing.routing_demand.is_some()
            {
                return Err("backend island bridges require native-compiled rules and no net-specific forecasts".into());
            }
        }
        Ok(())
    }
}

// Include the authoritative sources rather than maintaining another export,
// routing, import or verification implementation. All local imports and Java
// companions used by the native-project path are included.
macro_rules! helper {
    ($name:literal) => {
        (
            $name,
            include_str!(concat!("../experiments/whole-board/", $name)),
        )
    };
}

const HELPERS: &[(&str, &str)] = &[
    helper!("compare_native_router.py"),
    helper!("run.py"),
    helper!("compile_net_classes.py"),
    helper!("audit_dsn_classes.py"),
    helper!("inspect_dsn_geometry.py"),
    helper!("translate_dsn_edge.py"),
    helper!("adapt_dsn_layers.py"),
    helper!("audit_partial_ripup.py"),
    helper!("report_adaptive_routing.py"),
    helper!("report_routing_demand.py"),
    helper!("run_routing_demand.py"),
    helper!("render_inspection_layers.py"),
    helper!("sequential_evidence.py"),
    helper!("NativeOutlineMode.java"),
    helper!("RouteOutsideDsn.java"),
    helper!("InspectDsn.java"),
];

fn java_tmp_option(path: &Path) -> Result<String, String> {
    let text = path
        .to_str()
        .ok_or("output path must be UTF-8 for the Java launcher")?;
    if text.chars().any(|c| matches!(c, '\"' | '\\' | '\n' | '\r')) {
        return Err("output path contains unsupported characters for JAVA_TOOL_OPTIONS".into());
    }
    Ok(format!("-Djava.io.tmpdir=\"{text}\""))
}

fn read_json(path: &Path) -> Result<Value, String> {
    serde_json::from_slice(&fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?)
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn require_complete(report: &Value) -> Result<(), String> {
    if report["status"] == "finished"
        && report["complete"] == true
        && report["routing_complete"] == true
        && report["native"]["complete"] == true
    {
        return Ok(());
    }
    Err(format!(
        "external routing did not produce a native-complete board (status={}, routing_complete={}, native_complete={}); partial artifacts are retained",
        report["status"], report["routing_complete"], report["native"]["complete"]
    ))
}

fn native_physical_clean(native: &Value) -> bool {
    [
        "drc_design_violations",
        "erc_violations",
        "schematic_parity_issues",
    ]
    .iter()
    .all(|key| native[*key].as_u64() == Some(0))
}

fn bridge_eligible(report: &Value) -> bool {
    report["status"] == "finished"
        && report["complete"] == false
        && report["native"]["complete"] == false
        && report["imported"] == true
        && report["source_unchanged"] == true
        && report["fixed_placement_matches"] == true
        && report["project_and_classes_unchanged"] == true
        && report["dimensional_mismatches"]
            .as_array()
            .is_some_and(Vec::is_empty)
        && native_physical_clean(&report["native"])
        && report["native"]["selected_net_unconnected_items"]
            .as_u64()
            .is_some_and(|n| n > 0)
}

fn bridge_complete(report: &Value) -> bool {
    report["status"] == "complete"
        && report["source_unchanged"] == true
        && report["selected_native"]["complete"] == true
        && native_physical_clean(&report["selected_native"])
        && report["selected_native"]["selected_net_unconnected_items"].as_u64() == Some(0)
        && report["selected_directory"]
            .as_str()
            .is_some_and(|p| !p.is_empty())
}

enum StageOutcome {
    Exited(ExitStatus),
    TimedOut,
}

/// Both child stages inherit one deadline and own their complete process group.
fn wait_stage(command: &mut Command, deadline: Instant) -> Result<StageOutcome, String> {
    if Instant::now() >= deadline {
        return Ok(StageOutcome::TimedOut);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|e| format!("routing stage: {e}"))?;
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Ok(StageOutcome::Exited(status));
        }
        if Instant::now() >= deadline {
            #[cfg(unix)]
            let _ = Command::new("/bin/kill")
                .args(["-KILL", "--", &format!("-{}", child.id())])
                .status();
            let _ = child.kill();
            let _ = child.wait();
            return Ok(StageOutcome::TimedOut);
        }
        thread::sleep(
            Duration::from_millis(25).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

/// The bridge file is its atomically published native incumbent, including on timeout.
/// This receipt never rewrites the independent external or bridge evidence.
fn publish_selection(
    output: &Path,
    external: &Value,
    bridge: Option<&Value>,
    status: &str,
    elapsed: Duration,
) -> Result<(), String> {
    let incumbent = bridge.filter(|b| {
        b["source_unchanged"] == true
            && native_physical_clean(&b["selected_native"])
            && b["selected_native"]["selected_net_unconnected_items"]
                .as_u64()
                .is_some()
            && b["selected_directory"].as_str().is_some()
    });
    let (stage, directory, native) = if let Some(b) = incumbent {
        (
            serde_json::json!("island_bridges"),
            b["selected_directory"].clone(),
            b["selected_native"].clone(),
        )
    } else if native_physical_clean(&external["native"])
        && (require_complete(external).is_ok() || bridge_eligible(external))
    {
        (
            serde_json::json!("external"),
            serde_json::json!(output.join("routing/result")),
            external["native"].clone(),
        )
    } else {
        (Value::Null, Value::Null, Value::Null)
    };
    let render = directory.as_str().map(|path| {
        Path::new(path).join(if incumbent.is_some() {
            "preview.svg"
        } else {
            "inspection-combined.svg"
        })
    });
    let receipt = serde_json::json!({"schema_version":1, "status":status,
        "complete":status == "complete", "selected_stage":stage, "selected_directory":directory,
        "selected_native":native, "selected_render":render, "external_report":output.join("routing/report.json"),
        "island_bridge_report":bridge.map(|_| output.join("island-bridges/island-bridges.json")),
        "pipeline_elapsed_seconds":elapsed.as_secs_f64()});
    let temporary = output.join(".final-selection.next.json");
    fs::write(
        &temporary,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&receipt).map_err(|e| e.to_string())?
        ),
    )
    .map_err(|e| e.to_string())?;
    fs::rename(temporary, output.join("final-selection.json")).map_err(|e| e.to_string())
}

pub fn run(source: &Path, board_id: &str, output: &Path, config_path: &Path) -> Result<(), String> {
    let config: Config = serde_json::from_value(read_json(config_path)?)
        .map_err(|e| format!("invalid external routing configuration: {e}"))?;
    config.validate()?;
    if !matches!(
        Path::new(board_id)
            .components()
            .collect::<Vec<_>>()
            .as_slice(),
        [Component::Normal(_)]
    ) {
        return Err("board-id must be a single filename stem".into());
    }
    let source = source.canonicalize().map_err(|e| format!("source: {e}"))?;
    let absolute_output = env::current_dir().map_err(|e| e.to_string())?.join(output);
    java_tmp_option(&absolute_output)?;
    if absolute_output.exists() {
        return Err(format!("output already exists: {}", output.display()));
    }
    let parent = absolute_output.parent().ok_or("output has no parent")?;
    let existing_parent = parent
        .ancestors()
        .find(|path| path.exists())
        .ok_or("output has no existing ancestor")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if existing_parent.starts_with(&source) {
        return Err("output must be outside the source project".into());
    }
    fs::create_dir_all(parent).map_err(|e| format!("output parent: {e}"))?;
    let output = parent.canonicalize().map_err(|e| e.to_string())?.join(
        absolute_output
            .file_name()
            .ok_or("output has no filename")?,
    );
    if output.starts_with(&source) {
        return Err("output must be outside the source project".into());
    }
    let config_base = config_path.canonicalize().map_err(|e| e.to_string())?;
    let resolve = |path: &Path| -> Result<PathBuf, String> {
        config_base
            .parent()
            .unwrap()
            .join(path)
            .canonicalize()
            .map_err(|e| format!("tool {}: {e}", path.display()))
    };
    let java_tmp = output.join("java-tmp");
    let java_options = java_tmp_option(&java_tmp)?;
    let jar = resolve(&config.jar)?;
    let java = resolve(&config.java)?;
    let python = resolve(&config.python)?;
    fs::create_dir(&output).map_err(|e| format!("output: {e}"))?;
    fs::copy(config_path, output.join("backend-config.json")).map_err(|e| e.to_string())?;
    let helpers = output.join("tooling/experiments/whole-board");
    fs::create_dir_all(&helpers).map_err(|e| e.to_string())?;
    for (name, contents) in HELPERS {
        fs::write(helpers.join(name), contents).map_err(|e| e.to_string())?;
    }
    // The pinned jar otherwise reads shared temporary settings and ambient
    // router overrides. Keep its defaults, explicit bounds and source layers
    // reproducible; the pipeline records the actual merged router settings.
    fs::create_dir_all(java_tmp.join("freerouting")).map_err(|e| e.to_string())?;
    fs::write(
        java_tmp.join("freerouting/freerouting.json"),
        "{\"router\":{\"optimizer\":{\"enabled\":false}}}\n",
    )
    .map_err(|e| e.to_string())?;
    let executable = env::current_exe().map_err(|e| e.to_string())?;
    let routing = output.join("routing");
    let log_path = output.join("backend.log");
    let log = fs::File::create(&log_path).map_err(|e| e.to_string())?;
    let mut command = Command::new(python);
    command
        .args([
            helpers.join("compare_native_router.py").as_os_str(),
            "--source".as_ref(),
            source.as_os_str(),
        ])
        .arg("--board-id")
        .arg(board_id)
        .arg("--binary")
        .arg(&executable)
        .arg("--output")
        .arg(&routing)
        .arg("--jar")
        .arg(jar)
        .arg("--java")
        .arg(java)
        .arg("--seconds")
        .arg(config.maximum_seconds.to_string())
        .arg("--passes")
        .arg(config.maximum_passes.to_string())
        // KiCad copper roles are descriptive, not track prohibitions. Keep
        // native metadata unchanged while allowing the same copper layers in
        // the exchange router. The helper rejects planes and existing wiring.
        .args([
            "--translate-edge-rule",
            "--outside-outline",
            "--all-copper-signal-layers",
        ])
        .env("JAVA_TOOL_OPTIONS", java_options)
        .env_remove("JDK_JAVA_OPTIONS")
        .env_remove("_JAVA_OPTIONS")
        .stdout(Stdio::from(log.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::from(log));
    for (key, _) in env::vars_os() {
        if key.to_string_lossy().starts_with("FREEROUTING__") {
            command.env_remove(key);
        }
    }
    let started = Instant::now();
    let deadline = started
        .checked_add(Duration::from_secs(config.pipeline_maximum_seconds))
        .ok_or("pipeline deadline is out of range")?;
    let report_path = routing.join("report.json");
    let outcome = wait_stage(&mut command, deadline)?;
    let report = read_json(&report_path).unwrap_or(Value::Null);
    let status = match outcome {
        StageOutcome::Exited(status) => status,
        StageOutcome::TimedOut => {
            publish_selection(&output, &report, None, "timed_out", started.elapsed())?;
            return Err(format!(
                "external pipeline timed out; retained artifacts: {}",
                output.display()
            ));
        }
    };
    if !status.success() {
        publish_selection(&output, &report, None, "error", started.elapsed())?;
        return Err(format!(
            "external pipeline failed ({status}); see {} and {}",
            log_path.display(),
            report_path.display()
        ));
    }
    let mut selected = routing.join("result");
    let mut selected_stage = "external";
    let mut bridge_report = None;
    if require_complete(&report).is_err() {
        let Some(bridge_config) = config
            .island_bridges
            .as_ref()
            .filter(|_| bridge_eligible(&report))
        else {
            publish_selection(&output, &report, None, "incomplete", started.elapsed())?;
            return Err(format!(
                "{}; see {}",
                require_complete(&report).unwrap_err(),
                report_path.display()
            ));
        };
        let bridge_output = output.join("island-bridges");
        let bridge_config_path = output.join("island-bridge-config.json");
        fs::write(
            &bridge_config_path,
            format!(
                "{}\n",
                serde_json::to_string_pretty(bridge_config).map_err(|e| e.to_string())?
            ),
        )
        .map_err(|e| e.to_string())?;
        let bridge_log_path = output.join("island-bridges.log");
        let bridge_log = fs::File::create(&bridge_log_path).map_err(|e| e.to_string())?;
        let mut bridge_command = Command::new(&executable);
        bridge_command
            .arg("bridge-kicad-open-connections")
            .arg(&selected)
            .arg(board_id)
            .arg(&bridge_config_path)
            .arg(&bridge_output)
            .stdout(Stdio::from(
                bridge_log.try_clone().map_err(|e| e.to_string())?,
            ))
            .stderr(Stdio::from(bridge_log));
        let outcome = wait_stage(&mut bridge_command, deadline)?;
        let path = bridge_output.join("island-bridges.json");
        bridge_report = read_json(&path).ok();
        let final_status = match outcome {
            StageOutcome::TimedOut => "timed_out",
            StageOutcome::Exited(status)
                if status.success() && bridge_report.as_ref().is_some_and(bridge_complete) =>
            {
                "complete"
            }
            StageOutcome::Exited(_) => "incomplete",
        };
        if final_status != "complete" {
            publish_selection(
                &output,
                &report,
                bridge_report.as_ref(),
                final_status,
                started.elapsed(),
            )?;
            return Err(format!(
                "native island bridge stage {final_status}; last atomic incumbent retained; see {} and {}",
                path.display(),
                bridge_log_path.display()
            ));
        }
        selected = PathBuf::from(
            bridge_report.as_ref().unwrap()["selected_directory"]
                .as_str()
                .unwrap(),
        );
        selected_stage = "external plus native island bridges";
    }
    publish_selection(
        &output,
        &report,
        bridge_report.as_ref(),
        "complete",
        started.elapsed(),
    )?;
    println!(
        "native-complete {selected_stage} route: {}",
        selected.display()
    );
    println!(
        "combined render: {}",
        selected
            .join(if bridge_report.is_some() {
                "preview.svg"
            } else {
                "inspection-combined.svg"
            })
            .display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn process_success_does_not_admit_incomplete_or_native_failed_output() {
        let mut report = json!({"status":"finished", "complete":true,
            "routing_complete":true, "native":{"complete":true}});
        assert!(require_complete(&report).is_ok());
        report["routing_complete"] = json!(false);
        assert!(require_complete(&report).is_err());
        report["routing_complete"] = json!(true);
        report["native"]["complete"] = json!(false);
        assert!(require_complete(&report).is_err());
        assert!(require_complete(&json!({"status":"failed"})).is_err());
    }

    #[test]
    fn configuration_cannot_override_physical_rules_or_native_layer_semantics() {
        let base = json!({"jar":"router.jar", "java":"java", "maximum_seconds":30,
            "maximum_passes":2, "pipeline_maximum_seconds":90});
        let config: Config = serde_json::from_value(base.clone()).unwrap();
        assert!(config.validate().is_ok());
        assert!(config.island_bridges.is_none());
        for field in ["rules", "all_copper_signal_layers", "outside_outline"] {
            let mut changed = base.clone();
            changed[field] = json!(true);
            assert!(serde_json::from_value::<Config>(changed).is_err());
        }
        let mut changed: Config = serde_json::from_value(base).unwrap();
        changed.maximum_seconds = 0;
        assert!(changed.validate().is_err());
        changed.maximum_seconds = 91;
        assert!(changed.validate().is_err());
    }

    #[test]
    fn bridge_stage_requires_finished_preserved_import_with_only_native_opens() {
        let eligible = json!({"status":"finished", "complete":false,
            "imported":true, "source_unchanged":true, "fixed_placement_matches":true,
            "project_and_classes_unchanged":true, "dimensional_mismatches":[],
            "native":{"complete":false, "selected_net_unconnected_items":1,
                "drc_design_violations":0, "erc_violations":0, "schematic_parity_issues":0,
                "library_metadata_warnings":3}});
        assert!(bridge_eligible(&eligible));
        for key in [
            "imported",
            "source_unchanged",
            "fixed_placement_matches",
            "project_and_classes_unchanged",
        ] {
            let mut changed = eligible.clone();
            changed[key] = json!(false);
            assert!(!bridge_eligible(&changed), "{key}");
        }
        for key in [
            "drc_design_violations",
            "erc_violations",
            "schematic_parity_issues",
        ] {
            let mut changed = eligible.clone();
            changed["native"][key] = json!(1);
            assert!(!bridge_eligible(&changed), "{key}");
            changed["native"][key] = Value::Null;
            assert!(!bridge_eligible(&changed), "missing {key}");
        }
        for value in [json!(0), Value::Null] {
            let mut changed = eligible.clone();
            changed["native"]["selected_net_unconnected_items"] = value;
            assert!(!bridge_eligible(&changed));
        }
        let mut changed = eligible.clone();
        changed["dimensional_mismatches"] = json!(["width"]);
        assert!(!bridge_eligible(&changed));
        changed = eligible;
        changed["status"] = json!("failed");
        assert!(!bridge_eligible(&changed));
    }

    #[test]
    fn bridge_completion_requires_atomic_status_source_and_native_evidence() {
        let complete = json!({"status":"complete", "source_unchanged":true,
            "selected_directory":"candidate-000", "selected_native":{"complete":true,
                "selected_net_unconnected_items":0, "drc_design_violations":0,
                "erc_violations":0, "schematic_parity_issues":0,
                "intentional_no_connect_groups":1, "library_metadata_warnings":2}});
        assert!(bridge_complete(&complete));
        for key in [
            "selected_net_unconnected_items",
            "drc_design_violations",
            "erc_violations",
            "schematic_parity_issues",
        ] {
            let mut changed = complete.clone();
            changed["selected_native"][key] = json!(1);
            assert!(!bridge_complete(&changed), "{key}");
            changed["selected_native"][key] = Value::Null;
            assert!(!bridge_complete(&changed), "missing {key}");
        }
        for status in ["running", "incomplete", "error"] {
            let mut changed = complete.clone();
            changed["status"] = json!(status);
            assert!(!bridge_complete(&changed));
        }
        let mut changed = complete.clone();
        changed["source_unchanged"] = json!(false);
        assert!(!bridge_complete(&changed));
        changed = complete.clone();
        changed["selected_native"]["complete"] = json!(false);
        assert!(!bridge_complete(&changed));
        changed = complete;
        changed["selected_directory"] = Value::Null;
        assert!(!bridge_complete(&changed));
    }

    #[test]
    fn bridge_option_is_bounded_and_has_no_named_endpoint_policy() {
        let base = json!({"jar":"router.jar", "java":"java", "maximum_seconds":30,
            "maximum_passes":2, "pipeline_maximum_seconds":90,
            "island_bridges":{"routing":{}, "maximum_attempts":2, "maximum_pair_candidates":8}});
        let config: Config = serde_json::from_value(base.clone()).unwrap();
        assert!(config.validate().is_ok());
        assert!(config.island_bridges.is_some());
        let mut changed = base.clone();
        changed["island_bridges"]["maximum_attempts"] = json!(0);
        assert!(
            serde_json::from_value::<Config>(changed)
                .unwrap()
                .validate()
                .is_err()
        );
        for key in ["connection", "start", "finish"] {
            let mut changed = base.clone();
            changed["island_bridges"][key] = json!("N");
            assert!(serde_json::from_value::<Config>(changed).is_err());
        }
    }

    #[test]
    fn expired_shared_deadline_does_not_start_another_stage() {
        let mut command = Command::new("nonexistent-island-bridge-deadline-control");
        assert!(matches!(
            wait_stage(&mut command, Instant::now()).unwrap(),
            StageOutcome::TimedOut
        ));
    }
}
