// Copyright (C) 2026 Florian Loitsch.

use super::{Arguments, KiCadRunStatus, LayoutAttempt};
use serde_json::{Value, json};

pub(super) fn record(
    args: &Arguments,
    selected: &LayoutAttempt,
    requested: bool,
    report: &mut Value,
) -> KiCadRunStatus {
    let status = selected.status();
    let Some(verdict) = selected
        .verdict
        .as_ref()
        .filter(|_| selected.result.is_file())
    else {
        return status;
    };
    if verdict["kicad_checked"] != true {
        report["stored_delivery"] = json!({"scope": "native_unverified", "requested": requested});
        return status;
    }
    let fill = &verdict["fill_publication"];
    if fill.is_null() {
        report["stored_delivery"] = json!({
            "scope": "stored_copper",
            "requested": requested,
            "reason": "no_fillable_zones",
        });
        return status;
    }
    if fill["status"] == "saved" {
        let stored = &fill["saved_as_stored"];
        report["stored_delivery"] = json!({
            "scope": "stored_copper",
            "requested": requested,
            "native": stored,
        });
        if status == KiCadRunStatus::Complete
            && stored["opens"].as_u64().is_some_and(|opens| opens > 0)
        {
            report["reason"] = json!("stored_copper_has_opens");
            return KiCadRunStatus::Incomplete;
        }
        return status;
    }

    let mut evidence = json!({
        "scope": "refilled_copper_only",
        "requested": requested,
        "fill_status": fill["status"],
        "reason": fill["reason"],
    });
    // An unfinished route already has more relevant routing advice. For a
    // completed refill, give a direct way to deliver and verify saved copper.
    if status != KiCadRunStatus::Complete {
        report["stored_delivery"] = evidence;
        return status;
    }
    let argv = fill["retry_argv"]
        .as_array()
        .and_then(|argv| {
            argv.iter()
                .map(|item| item.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
        })
        .filter(|argv| !argv.is_empty())
        .unwrap_or_else(|| {
            vec![
                "pcb-maker".into(),
                "drc-kicad-board".into(),
                selected.result.parent().unwrap().display().to_string(),
                args.board_id.clone(),
                args.output
                    .join("stored-fill-drc.json")
                    .display()
                    .to_string(),
                "--save-filled".into(),
                args.output.join("stored-fill").display().to_string(),
            ]
        });
    let command = argv
        .iter()
        .map(|argument| super::super::shell_argument(argument))
        .collect::<Vec<_>>()
        .join(" ");
    evidence["retry_argv"] = json!(argv);
    report["stored_delivery"] = evidence;
    let mut actions = report["next_actions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    actions.insert(0, json!(format!(
        "KiCad verified the board after refill. To verify and save copper in a separate output, run `{command}`."
    )));
    report["next_actions"] = json!(actions);
    if requested {
        report["reason"] = json!("requested_stored_copper_unverified");
        KiCadRunStatus::Incomplete
    } else {
        status
    }
}
