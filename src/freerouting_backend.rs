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
    helper!("translate_dsn_classes.py"),
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

/// A finished, preserved import whose only native findings are open
/// connections: its board is still the selection.
fn clean_with_only_opens(report: &Value) -> bool {
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

enum StageOutcome {
    Exited(ExitStatus),
    TimedOut,
}

/// The child stage runs until the deadline and owns its complete process group.
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

/// Publishes final-selection.json atomically; it never rewrites the
/// external evidence.
fn publish_selection(
    output: &Path,
    external: &Value,
    status: &str,
    elapsed: Duration,
) -> Result<(), String> {
    let (stage, directory, native) = if native_physical_clean(&external["native"])
        && (require_complete(external).is_ok() || clean_with_only_opens(external))
    {
        (
            serde_json::json!("external"),
            serde_json::json!(output.join("routing/result")),
            external["native"].clone(),
        )
    } else {
        (Value::Null, Value::Null, Value::Null)
    };
    let render = directory
        .as_str()
        .map(|path| Path::new(path).join("inspection-combined.svg"));
    let receipt = serde_json::json!({"schema_version":1, "status":status,
        "complete":status == "complete", "selected_stage":stage, "selected_directory":directory,
        "selected_native":native, "selected_render":render, "external_report":output.join("routing/report.json"),
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
            publish_selection(&output, &report, "timed_out", started.elapsed())?;
            return Err(format!(
                "external pipeline timed out; retained artifacts: {}",
                output.display()
            ));
        }
    };
    if !status.success() {
        publish_selection(&output, &report, "error", started.elapsed())?;
        return Err(format!(
            "external pipeline failed ({status}); see {} and {}",
            log_path.display(),
            report_path.display()
        ));
    }
    let selected = routing.join("result");
    if let Err(incomplete) = require_complete(&report) {
        publish_selection(&output, &report, "incomplete", started.elapsed())?;
        return Err(format!("{incomplete}; see {}", report_path.display()));
    }
    publish_selection(&output, &report, "complete", started.elapsed())?;
    println!("native-complete external route: {}", selected.display());
    println!(
        "combined render: {}",
        selected.join("inspection-combined.svg").display()
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
    fn selection_keeps_a_finished_preserved_import_with_only_native_opens() {
        let eligible = json!({"status":"finished", "complete":false,
            "imported":true, "source_unchanged":true, "fixed_placement_matches":true,
            "project_and_classes_unchanged":true, "dimensional_mismatches":[],
            "native":{"complete":false, "selected_net_unconnected_items":1,
                "drc_design_violations":0, "erc_violations":0, "schematic_parity_issues":0,
                "library_metadata_warnings":3}});
        assert!(clean_with_only_opens(&eligible));
        for key in [
            "imported",
            "source_unchanged",
            "fixed_placement_matches",
            "project_and_classes_unchanged",
        ] {
            let mut changed = eligible.clone();
            changed[key] = json!(false);
            assert!(!clean_with_only_opens(&changed), "{key}");
        }
        for key in [
            "drc_design_violations",
            "erc_violations",
            "schematic_parity_issues",
        ] {
            let mut changed = eligible.clone();
            changed["native"][key] = json!(1);
            assert!(!clean_with_only_opens(&changed), "{key}");
            changed["native"][key] = Value::Null;
            assert!(!clean_with_only_opens(&changed), "missing {key}");
        }
        for value in [json!(0), Value::Null] {
            let mut changed = eligible.clone();
            changed["native"]["selected_net_unconnected_items"] = value;
            assert!(!clean_with_only_opens(&changed));
        }
        let mut changed = eligible.clone();
        changed["dimensional_mismatches"] = json!(["width"]);
        assert!(!clean_with_only_opens(&changed));
        changed = eligible;
        changed["status"] = json!("failed");
        assert!(!clean_with_only_opens(&changed));
    }

    #[test]
    fn expired_deadline_does_not_start_the_stage() {
        let mut command = Command::new("nonexistent-stage-deadline-control");
        assert!(matches!(
            wait_stage(&mut command, Instant::now()).unwrap(),
            StageOutcome::TimedOut
        ));
    }
}
