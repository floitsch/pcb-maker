// Copyright (C) 2026 Florian Loitsch.

//! Compose ordinary schematic import and native-verified layout.

use super::{Failure, KiCadRunStatus, check_file, read_json};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

mod placement_retry;
mod stored_delivery;

const COMMAND: &str = "layout-kicad-schematic";
const USAGE: &str = "usage: pcb-maker layout-kicad-schematic <schematic.kicad_sch> <board-id> <out-dir> --constraints <constraints.json> [options]";
pub(super) const HELP: &str = concat!(
    "usage: pcb-maker layout-kicad-schematic <schematic.kicad_sch> <board-id> <out-dir> --constraints <constraints.json> [options]\n\n",
    "Imports the schematic with the supplied placement constraints, then runs ordinary layout.\n",
    "The output directory must be new. Read workflow.json for the stage, exit code and next actions.\n",
    "The import and layout retain their reports and complete projects under the output directory.\n\n",
    "When a native-checked layout is close to complete, the workflow may try one advised recovery\n",
    "within the unused layout budget. workflow.json identifies the selected result.\n\n",
    "Include an absolute place entry and face for each schematic-backed donor hold.\n",
    "Use placer.free in the layout settings to explicitly release a donor hold.\n\n",
    "  --layout-config FILE   ordinary layout settings. Omit placer.constraints or use constraints.json.\n",
    "  --router-config FILE   ordinary router settings. Default: auto.\n",
    "  --layers N, --footprints-from BOARD, --stackup-from BOARD, --mechanical-from BOARD,\n",
    "  --mechanical-footprint UUID[=NET], --keep-footprint-clearances\n",
    "                        passed to import-kicad-netlist with the same meanings.\n",
    "  --reference DIR|none, --reuse-routes BOARD, --placement-seed N,\n",
    "  --plane-escape-budget S, --terminal-launch-budget S\n",
    "                        passed to layout-kicad-board with the same meanings.\n",
);

struct Arguments {
    schematic: String,
    board_id: String,
    output: PathBuf,
    constraints: String,
    layout: Option<String>,
    router: String,
    import_options: Vec<String>,
    layout_options: Vec<String>,
}

struct LayoutInvocation {
    directory: &'static str,
    log: &'static str,
    seed: Option<u64>,
    work_rate: Option<String>,
    router: Option<PathBuf>,
    reuse_routes: Option<PathBuf>,
}

struct LayoutAttempt {
    exit: ExitStatus,
    verdict: Option<Value>,
    directory: &'static str,
    log: &'static str,
    result: PathBuf,
}

impl LayoutAttempt {
    fn status(&self) -> KiCadRunStatus {
        child_status(self.exit, self.verdict.as_ref())
    }

    fn outputs(&self) -> Value {
        let mut outputs = json!({"layout_log": self.log});
        if self.verdict.is_some() {
            outputs["layout_verdict"] = json!(format!("{}/verdict.json", self.directory));
        }
        if self.result.is_file() {
            outputs["result"] = json!(format!("{}/result", self.directory));
        }
        outputs
    }

    fn select(&self, report: &mut Value) {
        report["layout_exit_code"] = json!(self.exit.code());
        for (key, value) in self.outputs().as_object().unwrap() {
            report["outputs"][key] = value.clone();
        }
        if let Some(verdict) = &self.verdict {
            report["layout_status"] = verdict["status"].clone();
            if verdict["next_actions"].is_array() {
                report["next_actions"] = verdict["next_actions"].clone();
            }
        }
    }
}

impl Arguments {
    fn run_layout(
        &self,
        executable: &Path,
        project: &Path,
        config: &Path,
        invocation: LayoutInvocation,
    ) -> Result<LayoutAttempt, Failure> {
        let directory = self.output.join(invocation.directory);
        let log_path = self.output.join(invocation.log);
        let log = create(&log_path)?;
        eprintln!("running layout, see {}", log_path.display());
        let mut command = Command::new(executable);
        command
            .arg("layout-kicad-board")
            .arg(project)
            .arg(&self.board_id)
            .arg(&directory)
            .arg(
                invocation
                    .router
                    .as_deref()
                    .unwrap_or_else(|| Path::new(&self.router)),
            )
            .arg(config)
            .args(&self.layout_options);
        if let Some(seed) = invocation.seed {
            command.args(["--placement-seed", &seed.to_string()]);
        }
        if let Some(board) = invocation.reuse_routes {
            command.arg("--reuse-routes").arg(board);
        }
        if let Some(work_rate) = invocation.work_rate {
            command.env("PCB_WORK_RATE", work_rate);
        }
        let exit = command
            .stdout(
                log.try_clone()
                    .map_err(|error| Failure::Internal(error.to_string()))?,
            )
            .stderr(log)
            .status()
            .map_err(|error| Failure::Internal(error.to_string()))?;
        Ok(LayoutAttempt {
            exit,
            verdict: read_object(&directory.join("verdict.json")),
            directory: invocation.directory,
            log: invocation.log,
            result: directory
                .join("result")
                .join(format!("{}.kicad_pcb", self.board_id)),
        })
    }
}

fn arguments(args: Vec<String>) -> Result<Arguments, Failure> {
    let [schematic, board_id, output, options @ ..] = args.as_slice() else {
        return Err(Failure::Usage(USAGE.into()));
    };
    let mut constraints = None;
    let mut layout = None;
    let mut router = None;
    let mut import_options = Vec::new();
    let mut layout_options = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut options = options.iter();
    while let Some(option) = options.next() {
        if option != "--mechanical-footprint" && !seen.insert(option) {
            return Err(Failure::Usage(format!(
                "{option} was supplied more than once"
            )));
        }
        if option == "--keep-footprint-clearances" {
            import_options.push(option.clone());
            continue;
        }
        let value = options
            .next()
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| Failure::Usage(format!("{option} needs a value. {USAGE}")))?;
        match option.as_str() {
            "--constraints" => constraints = Some(value.clone()),
            "--layout-config" => layout = Some(value.clone()),
            "--router-config" => router = Some(value.clone()),
            "--layers"
            | "--footprints-from"
            | "--stackup-from"
            | "--mechanical-from"
            | "--mechanical-footprint" => import_options.extend([option.clone(), value.clone()]),
            "--reference"
            | "--reuse-routes"
            | "--placement-seed"
            | "--plane-escape-budget"
            | "--terminal-launch-budget" => layout_options.extend([option.clone(), value.clone()]),
            _ => return Err(Failure::Usage(format!("unknown option {option}. {USAGE}"))),
        }
    }
    pcb_kicad::check_board_id(board_id).map_err(Failure::Usage)?;
    if Path::new(schematic)
        .extension()
        .and_then(|ext| ext.to_str())
        != Some("kicad_sch")
    {
        return Err(Failure::Usage(format!(
            "expected a .kicad_sch file. {USAGE}"
        )));
    }
    Ok(Arguments {
        schematic: schematic.clone(),
        board_id: board_id.clone(),
        output: output.into(),
        constraints: constraints.ok_or_else(|| Failure::Usage(USAGE.into()))?,
        layout,
        router: router.unwrap_or_else(|| "auto".into()),
        import_options,
        layout_options,
    })
}

pub(super) fn run(args: Vec<String>) -> Result<KiCadRunStatus, Failure> {
    let args = arguments(args)?;
    if let Some(parent) = args
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| Failure::Usage(error.to_string()))?;
    }
    fs::create_dir(&args.output).map_err(|error| {
        Failure::Usage(format!(
            "cannot create a new output directory {}: {error}",
            args.output.display()
        ))
    })?;
    let mut report = json!({
        "command": COMMAND,
        "board_id": args.board_id,
        "stage": "prepare",
        "status": "running",
        "inputs": {"schematic": args.schematic, "constraints": args.constraints},
        "outputs": {},
        "next_actions": [],
    });
    let status = match workflow(&args, &mut report) {
        Ok(status) => status,
        Err(error) => {
            let (status, message) = match error {
                Failure::Usage(message) => (KiCadRunStatus::UsageError, message),
                Failure::Internal(message) => (KiCadRunStatus::InternalError, message),
            };
            report["error"] = json!(message);
            report["next_actions"] = json!([message]);
            status
        }
    };
    if status != KiCadRunStatus::Complete
        && let Some(note) = report["pin_numbering_review"]["next_action"]
            .as_str()
            .map(str::to_owned)
        && let Some(actions) = report["next_actions"].as_array_mut()
    {
        actions.push(json!(note));
    }
    report["exit_code"] = json!(status.exit_code());
    report["status"] = json!(status);
    write_report(&args.output, &report)?;
    println!("{report:#}");
    Ok(status)
}

fn workflow(args: &Arguments, report: &mut Value) -> Result<KiCadRunStatus, Failure> {
    write_report(&args.output, report)?;
    check_file(&args.schematic)?;
    pcb_kicad::read_placement_constraints(Path::new(&args.constraints)).map_err(Failure::Usage)?;
    let layout = args
        .layout
        .as_ref()
        .map(|path| layout_config(path))
        .transpose()?;
    if args.router != "auto" {
        check_file(&args.router)?;
    }

    // Import reads this file for added parts and mechanical donor overrides.
    let project = args.output.join("project");
    fs::create_dir(&project).map_err(|error| Failure::Internal(error.to_string()))?;
    fs::copy(&args.constraints, project.join("constraints.json"))
        .map_err(|error| Failure::Internal(error.to_string()))?;
    let layout_config = project.join("layout.json");
    if let Some(layout) = &layout {
        pcb_kicad::write_atomically(&layout_config, format!("{layout:#}\n").as_bytes())
            .map_err(Failure::Internal)?;
    }
    report["outputs"]["project"] = json!("project");
    report["outputs"]["constraints"] = json!("project/constraints.json");
    report["stage"] = json!("import");
    report["outputs"]["import_log"] = json!("import.log");
    write_report(&args.output, report)?;
    eprintln!(
        "importing schematic, see {}",
        args.output.join("import.log").display()
    );
    let executable =
        std::env::current_exe().map_err(|error| Failure::Internal(error.to_string()))?;
    let imported = args.output.join("import.json");
    let import_status = Command::new(&executable)
        .arg("import-kicad-netlist")
        .arg(&args.schematic)
        .arg(&project)
        .arg(&args.board_id)
        .args(&args.import_options)
        .stdout(create(&imported)?)
        .stderr(create(&args.output.join("import.log"))?)
        .status()
        .map_err(|error| Failure::Internal(error.to_string()))?;
    let import = read_object(&imported);
    if let Some(import) = &import {
        report["outputs"]["import_report"] = json!("import.json");
        record_pin_numbering_review(report, import);
    }
    report["import_exit_code"] = json!(import_status.code());
    if !import_status.success() {
        let (reason, action) = match import.as_ref() {
            Some(import) if import["missing_parts"].as_u64().unwrap_or_default() > 0 => (
                "missing_footprints",
                "Review import.json.missing. Supply the appropriate project footprint libraries or a matching --footprints-from board, then retry in a new output directory.",
            ),
            Some(import) if import["mechanical_from"].is_object() => (
                "incomplete_mechanical_transfer",
                "Review import.json.mechanical_from for conflicting locks and omitted mechanical objects. Resolve them before layout.",
            ),
            _ => (
                "import_failed",
                "Read import.log for the import error, then retry with the corrected input.",
            ),
        };
        report["reason"] = json!(reason);
        report["next_actions"] = json!([action]);
        return Ok(child_status(import_status, None));
    }
    let import = import
        .filter(|import| import["footprints"].is_u64())
        .ok_or_else(|| {
            Failure::Internal(
                "Import succeeded without a readable footprint report in import.json.".into(),
            )
        })?;
    if import["mechanical_from"]["unmatched_locks"]
        .as_array()
        .is_some_and(|locks| !locks.is_empty())
    {
        report["notes"] = json!([
            "Some donor locks were unmatched. See import.json.mechanical_from.unmatched_locks."
        ]);
    }
    if let Some(changes) = import["footprint_id_changes"].as_array()
        && !changes.is_empty()
    {
        report["footprint_identity"] = json!({
            "changed_assignments": changes.len(),
            "import_report": "import.json",
            "field": "footprint_id_changes",
        });
    }

    let explicitly_free = layout
        .as_ref()
        .and_then(|config| config["placer"]["free"].as_array());
    let (released, unconfirmed): (Vec<_>, Vec<_>) = import["mechanical_from"]["locks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|lock| lock["applied"] == false)
        .partition(|lock| explicitly_free.is_some_and(|free| free.contains(&lock["reference"])));
    if !released.is_empty() {
        report["released_mechanical_locks"] = json!(released);
    }
    if !unconfirmed.is_empty() {
        report["reason"] = json!("unapplied_mechanical_locks");
        report["unapplied_mechanical_locks"] = json!(unconfirmed);
        report["next_actions"] = json!([
            "Give each listed donor hold an absolute place entry and a front or back assignment in the supplied constraints. To deliberately release a hold, list its reference in placer.free in the layout config. Retry in a new output directory."
        ]);
        return Ok(KiCadRunStatus::Incomplete);
    }

    report["stage"] = json!("layout");
    report["outputs"]["layout_log"] = json!("layout.log");
    write_report(&args.output, report)?;
    let incumbent = args.run_layout(
        &executable,
        &project,
        &layout_config,
        LayoutInvocation {
            directory: "layout",
            log: "layout.log",
            seed: None,
            work_rate: None,
            router: None,
            reuse_routes: None,
        },
    )?;
    if incumbent.status() == KiCadRunStatus::Complete
        && (incumbent.verdict.is_none() || !incumbent.result.is_file())
    {
        return Err(Failure::Internal(
            "Layout exited successfully without its verdict or result PCB. See layout.log.".into(),
        ));
    }
    incumbent.select(report);
    let alternate = placement_retry::try_alternate(args, &executable, &project, &incumbent, report);
    let selected = alternate.as_ref().unwrap_or(&incumbent);
    selected.select(report);
    placement_retry::body_near_next_action(report);
    if let Some(verdict) = &selected.verdict {
        if !verdict["body_near_grid_retry"].is_null() {
            report["body_near_grid_retry"] = verdict["body_near_grid_retry"].clone();
        }
        let checked = verdict["schematic_parity_checked"] == true;
        report["schematic_parity"] = json!({
            "scope": "staged_schematic",
            "checked": checked,
            "findings": if checked { &verdict["schematic_parity_issues"] } else { &Value::Null },
            "layout_verdict": report["outputs"]["layout_verdict"],
        });
    }
    let finish_fill = layout
        .as_ref()
        .and_then(|config| config["save_zone_fills"].as_bool())
        .unwrap_or(true);
    let status = stored_delivery::record(args, selected, finish_fill, report);
    if status != KiCadRunStatus::Complete
        && report["next_actions"].as_array().is_some_and(Vec::is_empty)
    {
        report["next_actions"] = json!([
            "Read layout.log for the reported failure. The imported project is in project/."
        ]);
    }
    Ok(status)
}

fn record_pin_numbering_review(workflow: &mut Value, import: &Value) {
    let exposed: Vec<_> = import["unmatched_pin_details"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|pin| {
            let evidence = &pin["donor_pin_numbering"];
            if evidence["status"] != "unique_candidate" {
                return None;
            }
            let [candidate] = evidence["candidates"].as_array()?.as_slice() else {
                return None;
            };
            Some(json!({
                "reference": pin["reference"],
                "schematic_pin": pin["pin"],
                "net": pin["net"],
                "donor_pad_number": candidate["number"],
                "donor_pad_instances": candidate["instances"].as_array()?.len(),
            }))
        })
        .collect();
    if !exposed.is_empty() {
        workflow["pin_numbering_review"] = json!({
            "matches": exposed,
            "import_report": "import.json",
            "field": "unmatched_pin_details",
            "next_action": "Review the reported schematic pin and donor pad numbers against the source footprint and symbol. If they identify the same intended terminal, correct the mapping in the source, re-import and check native schematic parity.",
        });
    }
}

fn layout_config(path: &str) -> Result<Value, Failure> {
    let mut config: Value = read_json(path)?;
    if !config.is_object() || config.get("placer").is_some_and(|value| !value.is_object()) {
        return Err(Failure::Usage(format!(
            "{path} and its optional placer must be JSON objects."
        )));
    }
    let checked: pcb_kicad::KiCadBoardLayoutConfig = serde_json::from_value(config.clone())
        .map_err(|error| Failure::Usage(format!("invalid layout config {path}: {error}")))?;
    match checked.placer.constraints {
        None => config["placer"]["constraints"] = json!("constraints.json"),
        Some(pcb_kicad::KiCadConstraintsSource::File(path)) if path == Path::new("constraints.json") => {}
        Some(_) => return Err(Failure::Usage(
            "The layout config's placer.constraints must be omitted or set to constraints.json. Supply the mechanical brief with --constraints.".into(),
        )),
    }
    Ok(config)
}

fn read_object(path: &Path) -> Option<Value> {
    let value: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    value.is_object().then_some(value)
}

fn write_report(directory: &Path, report: &Value) -> Result<(), Failure> {
    pcb_kicad::write_atomically(
        &directory.join("workflow.json"),
        format!("{report:#}\n").as_bytes(),
    )
    .map_err(Failure::Internal)
}

fn create(path: &Path) -> Result<fs::File, Failure> {
    fs::File::create(path)
        .map_err(|error| Failure::Internal(format!("{}: {error}", path.display())))
}

fn child_status(exit: ExitStatus, verdict: Option<&Value>) -> KiCadRunStatus {
    match exit.code() {
        Some(0) => KiCadRunStatus::Complete,
        Some(1) if verdict.is_some_and(|v| v["status"] == "router_complete_unverified") => {
            KiCadRunStatus::RouterCompleteUnverified
        }
        Some(1) => KiCadRunStatus::Incomplete,
        Some(2) => KiCadRunStatus::UsageError,
        Some(3) => KiCadRunStatus::Cut,
        _ => KiCadRunStatus::InternalError,
    }
}
