// Copyright (C) 2026 Florian Loitsch.

use super::{Arguments, LayoutAttempt, LayoutInvocation, read_object, write_report};
use pcb_kicad::{
    BodyNearReseat, KiCadBoardLayoutConfig, KiCadPlacementRetryEvidence, KiCadRunStatus,
};
use serde_json::{Value, json};
use std::path::Path;

// Keep the extra search focused on boards that are already close to complete.
const MAX_NEAR_COMPLETE_OPENS: usize = 3;

fn finite_positive(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn admissible_verdict(verdict: &Value) -> bool {
    let empty = |field: &str| verdict[field].as_array().is_some_and(Vec::is_empty);
    verdict["kicad_checked"] == true
        && verdict["schematic_parity_checked"] == true
        && empty("unplaced")
        && empty("cut")
        && verdict["new_findings_strict"]
            .as_object()
            .is_some_and(|findings| findings.is_empty())
}

struct RetryPlan {
    strategy: RetryStrategy,
    config: Value,
    total_seconds: f64,
    first_work_seconds: f64,
    budget_seconds: f64,
    rate: [f64; 2],
    evidence: KiCadPlacementRetryEvidence,
}

enum RetryStrategy {
    AlternatePlacement(u64),
    PreparedRowEscape { router: Value, patch: Value },
    BodyNearReseat(BodyNearReseat),
}

fn has_authored_near_tuning(config: &Value) -> bool {
    let tuning = &config["placer"]["tuning"];
    [
        "grid_mm",
        "halo_overrides_mm",
        "halo_scale",
        "maximum_halo_mm",
        "track_pitch_mm",
        "pins_per_halo_track",
    ]
    .into_iter()
    .any(|key| tuning.get(key).is_some())
}

/// Use temporary fixed poses in the retry's placer options. The original
/// constraints.json, schematic and design-rule files remain its inputs.
fn hold_reseat_poses(brief: &mut Value, placed: &Value, reseat: &BodyNearReseat) -> Option<()> {
    let place = brief
        .as_object_mut()?
        .entry("place")
        .or_insert_with(|| json!([]))
        .as_array_mut()?;
    let footprints = placed["footprints"].as_array()?;
    let mut selected = 0;
    let mut ids = std::collections::BTreeSet::new();
    for row in footprints {
        let reference = row["reference"].as_str()?;
        let uuid = row["uuid"].as_str()?;
        if !ids.insert(uuid) {
            return None;
        }
        if row["fixed"].as_bool()? {
            continue;
        }
        let [x, y, angle] = if reference == reseat.part {
            selected += 1;
            reseat.at
        } else {
            serde_json::from_value::<[f64; 3]>(row["at"].clone()).ok()?
        };
        place.push(json!({"part": reference, "uuid": uuid, "origin":"absolute", "x":x,"y":y,"angle":angle}));
    }
    (selected == 1).then_some(())
}

fn prepared_row_router(verdict: &Value, mut router: Value) -> Option<RetryStrategy> {
    let options = router.as_object_mut()?;
    const KEY: &str = "prepared_row_escape_pads";
    if options.contains_key(KEY) {
        return None;
    }
    let advice = &verdict["prepared_row_escape_advice"];
    let patch = advice["router_config_patch"].as_object()?;
    let pads: Vec<String> = serde_json::from_value(patch.get(KEY)?.clone()).ok()?;
    if patch.len() != 1 || pads.len() != 2 || !pcb_kicad::valid_terminal_launch_pads(&pads) {
        return None;
    }
    let escapes = advice["escapes"].as_array()?;
    if escapes.len() != pads.len()
        || pads
            .iter()
            .any(|pad| !escapes.iter().any(|escape| escape["pad"] == pad.as_str()))
    {
        return None;
    }
    options.insert(KEY.into(), json!(pads));
    Some(RetryStrategy::PreparedRowEscape {
        router,
        patch: advice["router_config_patch"].clone(),
    })
}

fn retry_strategy(
    args: &Arguments,
    config: &KiCadBoardLayoutConfig,
    verdict: &Value,
) -> Option<RetryStrategy> {
    let prepared = if verdict["prepared_row_escape_advice"].is_object()
        && config.pin_swaps.is_none()
        && config.swappable.is_empty()
    {
        let router = if args.router == "auto" {
            Some(json!({}))
        } else {
            read_object(Path::new(&args.router))
        };
        router.and_then(|router| prepared_row_router(verdict, router))
    } else {
        None
    };
    prepared.or_else(|| {
        verdict["alternate_placement_seed"]
            .as_u64()
            .map(RetryStrategy::AlternatePlacement)
    })
}

fn plan(
    args: &Arguments,
    project: &Path,
    attempt: &LayoutAttempt,
    probe_work_seconds: &mut f64,
) -> Result<RetryPlan, String> {
    let skip = |reason: &str| reason.to_owned();
    if attempt.status() == KiCadRunStatus::Complete {
        return Err(skip("incumbent_complete"));
    }
    if attempt.status() != KiCadRunStatus::Incomplete {
        return Err(skip("incumbent_did_not_finish"));
    }
    let mut config =
        read_object(&project.join("layout.json")).ok_or_else(|| skip("no_layout_config"))?;
    let (options, _) = args.layout_options.as_chunks::<2>();
    if options.iter().any(|option| option[0] == "--placement-seed")
        || config["placer"]["tuning"].get("seed").is_some()
    {
        return Err(skip("explicit_placement_seed"));
    }
    if options.iter().any(|option| {
        matches!(
            option[0].as_str(),
            "--plane-escape-budget" | "--terminal-launch-budget" | "--reuse-routes"
        )
    }) {
        return Err(skip("explicit_routing_recovery"));
    }
    let checked: KiCadBoardLayoutConfig =
        serde_json::from_value(config.clone()).map_err(|_| skip("invalid_layout_config"))?;
    if checked.free_dimension.is_some()
        || checked.stop_after_first_route
        || !checked.save_zone_fills
    {
        return Err(skip("nonstandard_layout"));
    }
    let verdict = attempt
        .verdict
        .as_ref()
        .ok_or_else(|| skip("no_layout_verdict"))?;
    if !admissible_verdict(verdict) {
        return Err(skip("incumbent_failed_native_or_mechanical_gates"));
    }
    let first_work_seconds =
        finite_positive(&verdict["work_seconds"]).ok_or_else(|| skip("missing_layout_work"))?;
    let total_seconds = checked.total_seconds;
    let mut budget_seconds = (total_seconds - first_work_seconds).floor();
    if !budget_seconds.is_finite() || budget_seconds <= 0.0 {
        return Err(skip("no_work_remaining"));
    }
    let rate = ["work_rate", "placer_work_rate"].map(|key| finite_positive(&verdict[key]));
    let [Some(search_rate), Some(placement_rate)] = rate else {
        return Err(skip("missing_layout_work_rates"));
    };
    let result = attempt
        .result
        .parent()
        .ok_or_else(|| skip("no_result_project"))?;
    let evidence = KiCadPlacementRetryEvidence::read(result, &args.board_id, verdict)?;
    let stored = evidence
        .as_stored
        .as_ref()
        .ok_or_else(|| skip("missing_native_stored_proof"))?;
    let misses = verdict["constraints_missed"]
        .as_array()
        .ok_or_else(|| skip("missing_constraints_check"))?;
    let strategy = if misses.is_empty() {
        if ![stored.opens, evidence.refilled.opens]
            .into_iter()
            .all(|opens| (1..=MAX_NEAR_COMPLETE_OPENS).contains(&opens))
        {
            return Err(skip("incumbent_not_near_complete"));
        }
        retry_strategy(args, &checked, verdict)
            .ok_or_else(|| skip("no_supported_retry_suggested"))?
    } else {
        if misses.len() != 1
            || stored.opens != 0
            || evidence.refilled.opens != 0
            || checked.pin_swaps.is_some()
            || !checked.swappable.is_empty()
            || config["placer"]["constraints"] != "constraints.json"
        {
            return Err(skip("unsupported_body_near_retry"));
        }
        if budget_seconds < 2.0 * evidence.minimum_check_work_seconds + 3.0 {
            return Err(skip("insufficient_body_near_native_budget"));
        }
        if has_authored_near_tuning(&config) {
            return Err(skip("body_near_retry_keeps_authored_grid_and_halos"));
        }
        let placement = read_object(&result.join("board-placer.json"))
            .ok_or_else(|| skip("no_incumbent_placement"))?;
        let (reseat, work) = pcb_kicad::propose_body_near_reseat(
            project,
            &args.board_id,
            &checked.placer,
            &attempt.result,
            &placement,
            (placement_rate * (budget_seconds / 32.0).min(1.0)) as u64,
        );
        *probe_work_seconds = work as f64 / placement_rate;
        let reseat = reseat.map_err(skip)?;
        budget_seconds = (budget_seconds - *probe_work_seconds).floor();
        let mut brief = read_object(&project.join("constraints.json"))
            .ok_or_else(|| skip("missing_constraints"))?;
        hold_reseat_poses(&mut brief, &placement, &reseat)
            .ok_or_else(|| skip("incumbent_poses_are_ambiguous"))?;
        config["placer"]["constraints"] = brief;
        RetryStrategy::BodyNearReseat(reseat)
    };
    if budget_seconds < evidence.minimum_check_work_seconds {
        return Err(skip("insufficient_native_check_budget"));
    }
    config["total_seconds"] = json!(budget_seconds);
    Ok(RetryPlan {
        strategy,
        config,
        total_seconds,
        first_work_seconds,
        budget_seconds,
        rate: [search_rate, placement_rate],
        evidence,
    })
}

pub(super) fn try_alternate(
    args: &Arguments,
    executable: &Path,
    project: &Path,
    incumbent: &LayoutAttempt,
    report: &mut Value,
) -> Option<LayoutAttempt> {
    let mut probe_work_seconds = 0.0;
    let plan = match plan(args, project, incumbent, &mut probe_work_seconds) {
        Ok(plan) => plan,
        Err(reason) => {
            report["placement_retry"] = json!({"status": "skipped", "reason": reason});
            if probe_work_seconds > 0.0 {
                report["placement_retry"]["probe_work_seconds"] = json!(probe_work_seconds);
            }
            return None;
        }
    };
    const CONFIG: &str = "placement-retry-layout.json";
    const DIRECTORY: &str = "layout-placement-retry";
    const LOG: &str = "placement-retry.log";
    const ROUTER: &str = "placement-retry-router.json";
    let path = args.output.join(CONFIG);
    report["placement_retry"] = json!({
        "status": "running",
        "total_budget_seconds": plan.total_seconds,
        "retry_budget_seconds": plan.budget_seconds,
        "probe_work_seconds": probe_work_seconds,
        "config": CONFIG,
        "incumbent": {
            "outputs": incumbent.outputs(),
            "work_seconds": plan.first_work_seconds,
            "native": plan.evidence,
        },
        "candidate": {"outputs": {"layout_log": LOG}},
    });
    let (seed, router_config) = match &plan.strategy {
        RetryStrategy::AlternatePlacement(seed) => {
            report["placement_retry"]["strategy"] = json!("alternate_placement");
            report["placement_retry"]["seed"] = json!(seed);
            (Some(*seed), None)
        }
        RetryStrategy::PreparedRowEscape { router, patch } => {
            report["placement_retry"]["strategy"] = json!("prepared_row_escape");
            report["placement_retry"]["router_config"] = json!(ROUTER);
            report["placement_retry"]["router_config_patch"] = patch.clone();
            (None, Some(router))
        }
        RetryStrategy::BodyNearReseat(reseat) => {
            report["placement_retry"]["strategy"] = json!("body_near_reseat");
            report["placement_retry"]["body_near_reseat"] = json!(reseat);
            (None, None)
        }
    };
    let run = (|| {
        pcb_kicad::write_atomically(&path, format!("{:#}\n", plan.config).as_bytes())
            .map_err(super::Failure::Internal)?;
        let router = if let Some(config) = router_config {
            let path = args.output.join(ROUTER);
            pcb_kicad::write_atomically(&path, format!("{config:#}\n").as_bytes())
                .map_err(super::Failure::Internal)?;
            Some(path)
        } else {
            None
        };
        write_report(&args.output, report)?;
        let [search, placement] = plan.rate;
        args.run_layout(
            executable,
            project,
            &path,
            LayoutInvocation {
                directory: DIRECTORY,
                log: LOG,
                seed,
                work_rate: Some(format!("{search},{placement}")),
                router,
                reuse_routes: matches!(plan.strategy, RetryStrategy::BodyNearReseat(_))
                    .then(|| incumbent.result.clone()),
            },
        )
    })();
    let result = &mut report["placement_retry"];
    let candidate = match run {
        Ok(candidate) => candidate,
        Err(super::Failure::Usage(error) | super::Failure::Internal(error)) => {
            result["status"] = json!("declined");
            result["reason"] = json!("retry_failed");
            result["detail"] = json!(error);
            return None;
        }
    };
    result["candidate"] =
        json!({"outputs": candidate.outputs(), "exit_code": candidate.exit.code()});
    let decision = (|| {
        let verdict = candidate
            .verdict
            .as_ref()
            .ok_or("missing_candidate_verdict")?;
        result["candidate"]["layout_status"] = verdict["status"].clone();
        let work = finite_positive(&verdict["work_seconds"]).ok_or("missing_candidate_work")?;
        result["candidate"]["work_seconds"] = json!(work);
        result["combined_work_seconds"] =
            json!(plan.first_work_seconds + probe_work_seconds + work);
        if work > plan.budget_seconds {
            return Err("candidate_exceeded_work_budget");
        }
        if ["work_rate", "placer_work_rate"]
            .into_iter()
            .map(|key| finite_positive(&verdict[key]))
            .ne(plan.rate.map(Some))
        {
            return Err("candidate_work_rates_changed");
        }
        if !admissible_verdict(verdict)
            || !verdict["constraints_missed"]
                .as_array()
                .is_some_and(Vec::is_empty)
        {
            return Err("candidate_failed_native_or_mechanical_gates");
        }
        let project = candidate.result.parent().ok_or("no_candidate_result")?;
        let evidence = KiCadPlacementRetryEvidence::read(project, &args.board_id, verdict)
            .map_err(|error| {
                result["detail"] = json!(error);
                "no_candidate_native_evidence"
            })?;
        let comparison = if let RetryStrategy::BodyNearReseat(reseat) = &plan.strategy {
            pcb_kicad::verify_body_near_reseat(&incumbent.result, &candidate.result, reseat)
                .map_err(|error| {
                    result["detail"] = json!(error);
                    "body_near_changed_other_geometry"
                })?;
            evidence.qualifies_body_near_against(&plan.evidence)
        } else {
            evidence.qualifies_against(&plan.evidence)
        };
        result["candidate"]["native"] = json!(evidence);
        comparison?;
        if candidate.status() != KiCadRunStatus::Complete || verdict["status"] != "complete" {
            return Err("candidate_did_not_complete");
        }
        Ok(())
    })();
    match decision {
        Ok(()) => {
            result["status"] = json!("selected");
            result["reason"] = json!("completed_with_native_proof");
            Some(candidate)
        }
        Err(reason) => {
            result["status"] = json!("declined");
            result["reason"] = json!(reason);
            None
        }
    }
}

pub(super) fn body_near_next_action(report: &mut Value) {
    let action = match report["placement_retry"]["reason"].as_str() {
        Some("body_near_retry_keeps_authored_grid_and_halos") => Some(
            "The body-to-pin retry requires permission to use a local finer grid and vary inferred routing room. Review the authored placer grid and halo settings before comparing another layout with the same mechanical brief.",
        ),
        Some("no_legal_fixed_neighbour_body_near_pose") => Some(
            "The bounded local search found no pose meeting the exact requested distance with neighboring poses held. If the assembly permits it, try a different pose or angle for the target footprint and check the same limit again.",
        ),
        Some("insufficient_body_near_probe_budget") => Some(
            "The local search reached its work limit before verifying a body-to-pin pose. Compare a fresh layout with enough work for placement and native checks.",
        ),
        Some("insufficient_body_near_native_budget") => Some(
            "The remaining work was too small for a body-to-pin placement retry and its native checks.",
        ),
        _ => None,
    };
    if let (Some(action), Some(actions)) = (action, report["next_actions"].as_array_mut()) {
        actions.insert(0, json!(action));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_near_retry_respects_explicit_spacing_controls_and_freezes_neighbors() {
        assert!(!has_authored_near_tuning(
            &json!({"placer":{"tuning":{"spacing_mm":0.5}}})
        ));
        assert!(has_authored_near_tuning(
            &json!({"placer":{"tuning":{"grid_mm":0.635}}})
        ));
        assert!(has_authored_near_tuning(
            &json!({"placer":{"tuning":{"halo_overrides_mm":{"C1":0}}}})
        ));
        let mut brief = json!({"place":[{"part":"J1","x": 1.0,"y": 2.0}]});
        let poses = json!({"footprints":[
            {"reference":"C1","uuid":"c1","at":[1,3,90],"fixed":false},
            {"reference":"U1","uuid":"u1","at":[1,4,180],"fixed":false},
            {"reference":"J1","uuid":"j1","at":[1,2,0],"fixed":true}
        ]});
        let reseat = BodyNearReseat {
            part: "C1".into(),
            pin_of: "U1:5".into(),
            before_at: [1.0, 3.0, 90.0],
            at: [1.2, 3.0, 90.0],
            before_mm: 1.7,
            distance_mm: 1.3,
            max_mm: 1.5,
            local_pitch_mm: 0.1,
            relaxed_halos: ["C1".into(), "U1".into()],
        };
        assert!(hold_reseat_poses(&mut brief, &poses, &reseat).is_some());
        assert_eq!(brief["place"].as_array().unwrap().len(), 3);
        assert_eq!(brief["place"][0]["part"], "J1");
        assert_eq!(brief["place"][1]["x"], 1.2);
        assert_eq!(brief["place"][2]["y"], 4.0);
        assert_eq!(brief["place"][2]["angle"], 180.0);
    }

    #[test]
    fn body_near_feedback_distinguishes_an_ineligible_probe_from_an_exhausted_scan() {
        let actions = |reason| {
            let mut report = json!({"placement_retry":{"reason":reason}, "next_actions":[]});
            body_near_next_action(&mut report);
            report["next_actions"].as_array().unwrap().clone()
        };
        assert!(actions("unsupported_body_near_probe_geometry").is_empty());
        let remaining = actions("insufficient_body_near_probe_budget");
        assert!(remaining[0].as_str().unwrap().contains("work limit"));
        let no_pose = actions("no_legal_fixed_neighbour_body_near_pose");
        assert!(
            no_pose[0]
                .as_str()
                .unwrap()
                .contains("neighboring poses held")
        );
    }

    #[test]
    fn prepared_row_retry_adds_only_valid_advice_and_respects_explicit_policy() {
        let options = json!({
            "tuning": {"seed": 7, "pin_lines": true},
            "add_pours": [{"net": "GND", "layers": ["B.Cu"]}],
        });
        let mut verdict = json!({
            "prepared_row_escape_advice": {
                "escapes": [{"pad":"U3:9"}, {"pad":"U3:10"}],
                "router_config_patch": {"prepared_row_escape_pads":["U3:9", "U3:10"]},
            },
        });
        let Some(RetryStrategy::PreparedRowEscape { router, patch }) =
            prepared_row_router(&verdict, options.clone())
        else {
            panic!("expected the verified pair")
        };
        assert_eq!(router["tuning"], options["tuning"]);
        assert_eq!(router["add_pours"], options["add_pours"]);
        assert_eq!(
            router["prepared_row_escape_pads"],
            patch["prepared_row_escape_pads"]
        );
        assert!(prepared_row_router(&verdict, json!({"prepared_row_escape_pads": []})).is_none());
        verdict["prepared_row_escape_advice"]["router_config_patch"]["other"] = json!(true);
        assert!(prepared_row_router(&verdict, options.clone()).is_none());
        verdict["prepared_row_escape_advice"]["router_config_patch"]
            .as_object_mut()
            .unwrap()
            .remove("other");
        verdict["prepared_row_escape_advice"]["escapes"][1]["pad"] = json!("U4:10");
        assert!(prepared_row_router(&verdict, options).is_none());
    }
}
