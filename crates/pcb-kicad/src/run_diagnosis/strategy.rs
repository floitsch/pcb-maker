// Copyright (C) 2026 Florian Loitsch.

//! Derives a routing experiment from the saved ladder ledger.

use serde_json::{Value, json};

pub(super) fn summarize(ledger: &[Value]) -> Option<Value> {
    let mut selected = None;
    for (index, attempt) in ledger.iter().enumerate() {
        if attempt["id"].as_u64()? != index as u64 {
            return None;
        }
        if attempt["selected"].as_bool()? && selected.replace(index).is_some() {
            return None;
        }
    }
    let index = selected.filter(|index| *index != 0)?;
    let earlier = &ledger[..index];
    let seconds = earlier.iter().try_fold(0.0, |sum, attempt| {
        let work = attempt["work_seconds"].as_f64()?;
        (work >= 0.0 && (sum + work).is_finite()).then_some(sum + work)
    })?;
    let mut history = json!({
        "selected_attempt_id": index,
        "earlier_attempt_count": earlier.len(),
        "earlier_recorded_work_seconds": seconds,
    });
    if let Some(trial) = isolated_rung_trial(&ledger[index], earlier) {
        history["next_trial"] = trial;
    }
    Some(history)
}

fn pitch(attempt: &Value) -> Option<f64> {
    attempt["grid_pitch_mm"]
        .as_f64()
        .filter(|pitch| pitch.is_finite() && *pitch > 0.0)
}

fn isolated_rung_trial(selected: &Value, earlier: &[Value]) -> Option<Value> {
    let pitch_mm = pitch(selected)?;
    let pours = selected["pours"].as_str()?;
    let pin_lines_on = pin_lines(selected)?;
    let (rung, label) = selected["name"]
        .as_str()?
        .strip_prefix("the rung ")?
        .split_once(" pitch=")?;
    let exclusive_planes = match (pours, rung) {
        ("connect", "pours=connect") => false,
        ("connect", "pours=connect (exclusive planes)") => true,
        _ => return None,
    };
    let recorded_pitch = label
        .strip_prefix('[')?
        .strip_suffix(']')?
        .parse::<f64>()
        .ok()?;
    if pitch_mm != recorded_pitch
        || selected["counted_by"] != "kicad"
        || selected["open"].as_u64()? != 0
        || selected["starved"].as_u64()? != 0
        || !earlier
            .iter()
            .all(|attempt| pitch(attempt).is_some_and(|pitch| pitch > pitch_mm))
    {
        return None;
    }
    let same_rung = earlier
        .iter()
        .filter(|attempt| {
            attempt["counted_by"] == "kicad"
                && attempt["starved"].as_u64() == Some(0)
                && attempt["pours"] == pours
                && pin_lines(attempt) == Some(pin_lines_on)
                && earlier_rung(attempt) == Some(rung)
        })
        .filter_map(|attempt| Some((attempt["open"].as_u64()?, attempt["id"].as_u64()?)))
        .min()?;
    if same_rung.0 == 0 {
        return None;
    }
    Some(json!({
        "kind": "isolated_rung_trial",
        "comparable_earlier_attempt_id": same_rung.1,
        "router_config_patch": {
            "pours": "connect",
            "exclusive_planes": exclusive_planes,
            "pin_lines": pin_lines_on,
            "seeds": 1,
            "tuning": {
                "plane_skeleton": false,
                "fixed_plane_stubs": false,
                "probe_ladder": false,
                "pitches": [pitch_mm],
                "refine_pitches_mm": [],
                "retry_seeds": 0,
            },
        },
    }))
}

fn pin_lines(attempt: &Value) -> Option<bool> {
    attempt.get("pin_lines").map_or(Some(false), Value::as_bool)
}

fn earlier_rung(attempt: &Value) -> Option<&str> {
    let name = attempt["name"].as_str()?;
    if let Some(rung) = name.strip_prefix("the continuation of the probed rung ") {
        return Some(rung);
    }
    let (rung, label) = name.strip_prefix("the rung ")?.split_once(" pitch=")?;
    // Retries that used extra seeds have their own records.
    (label == "regular"
        || label
            .strip_prefix('[')?
            .strip_suffix(']')?
            .parse::<f64>()
            .is_ok())
    .then_some(rung)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(id: usize, grid_pitch_mm: f64, label: &str, open: usize, selected: bool) -> Value {
        json!({
            "id": id,
            "name": format!("the rung pours=connect (exclusive planes) pitch={label}"),
            "grid_pitch_mm": grid_pitch_mm,
            "counted_by": "kicad",
            "pours": "connect",
            "work_seconds": 7.25,
            "open": open,
            "starved": 0,
            "selected": selected,
        })
    }

    #[test]
    fn a_later_finer_winner_provides_a_paid_work_summary_and_an_isolated_trial() {
        let ledger = [
            attempt(0, 0.1, "regular", 4, false),
            attempt(1, 0.075, "[0.075]", 2, false),
            attempt(2, 0.05, "[0.05]", 0, true),
            attempt(3, 0.1, "regular", 4, false),
        ];
        let history = summarize(&ledger).unwrap();
        assert_eq!(
            history,
            json!({
                "selected_attempt_id": 2,
                "earlier_attempt_count": 2,
                "earlier_recorded_work_seconds": 14.5,
                "next_trial": {
                    "kind": "isolated_rung_trial",
                    "comparable_earlier_attempt_id": 1,
                    "router_config_patch": {
                        "pours": "connect",
                        "exclusive_planes": true,
                        "pin_lines": false,
                        "seeds": 1,
                        "tuning": {
                            "plane_skeleton": false,
                            "fixed_plane_stubs": false,
                            "probe_ladder": false,
                            "pitches": [0.05],
                            "refine_pitches_mm": [],
                            "retry_seeds": 0,
                        },
                    },
                },
            })
        );
        let config: crate::board_router::KiCadBoardRouterConfig =
            serde_json::from_value(history["next_trial"]["router_config_patch"].clone()).unwrap();
        assert_eq!(config.pours, crate::board_router::KiCadPourMode::Connect);
        assert_eq!(config.exclusive_planes, Some(true));
        assert!(config.connection_rules.is_empty());
    }

    #[test]
    fn first_winner_or_incomplete_ledger_has_no_history() {
        assert!(summarize(&[]).is_none());
        assert!(summarize(&[attempt(0, 0.05, "regular", 0, true)]).is_none());
        let mut ledger = [
            attempt(0, 0.1, "regular", 4, false),
            attempt(1, 0.05, "[0.05]", 0, true),
        ];
        ledger[0]["selected"] = json!(true);
        assert!(summarize(&ledger).is_none());
        ledger[0]["selected"] = json!(false);
        ledger[0]["work_seconds"] = Value::Null;
        assert!(summarize(&ledger).is_none());
    }

    #[test]
    fn recorded_work_does_not_imply_a_transferable_pitch() {
        let winner = attempt(1, 0.05, "[0.05]", 0, true);
        let coarse = attempt(0, 0.1, "regular", 4, false);
        let mut changed_rung = coarse.clone();
        changed_rung["name"] = json!("the rung pours=connect pitch=regular");
        let mut changed_lines = coarse.clone();
        changed_lines["pin_lines"] = json!(true);
        for earlier in [
            attempt(0, 0.05, "regular", 4, false),
            attempt(0, 0.1, "regular", 0, false),
            changed_rung,
            changed_lines,
        ] {
            let history = summarize(&[earlier, winner.clone()]).unwrap();
            assert!(history.get("next_trial").is_none());
            assert_eq!(history["earlier_recorded_work_seconds"], 7.25);
        }
        let mut retry = winner;
        retry["name"] =
            json!("the rung pours=connect (exclusive planes) pitch=[0.05] again with 4 seeds");
        assert!(
            summarize(&[coarse, retry])
                .unwrap()
                .get("next_trial")
                .is_none()
        );
    }
}
