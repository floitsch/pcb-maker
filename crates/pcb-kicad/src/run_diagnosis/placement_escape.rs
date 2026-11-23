// Copyright (C) 2026 Florian Loitsch.

use super::*;

fn names_part(row: &serde_json::Value, part: &str) -> bool {
    match row {
        serde_json::Value::String(value) => {
            value == part || value.split_once(':').is_some_and(|(name, _)| name == part)
        }
        serde_json::Value::Array(items) => items.iter().any(|item| names_part(item, part)),
        serde_json::Value::Object(items) => items.values().any(|item| names_part(item, part)),
        _ => false,
    }
}

fn linked_pads<'a>(
    result: &'a LoweredBoard,
    terminal: &NamedTerminal,
    drc: &serde_json::Value,
) -> Option<([&'a BoardPad; 2], [String; 2])> {
    let [first, second]: [&BoardPad; 2] = result
        .pads
        .iter()
        .filter(|pad| pad.net.as_deref() == Some(&terminal.2))
        .collect::<Vec<_>>()
        .try_into()
        .ok()?;
    if ![first, second]
        .iter()
        .any(|p| p.reference == terminal.0 && p.number == terminal.1)
    {
        return None;
    }
    let ids = [
        first.uuid.as_ref()?.to_ascii_lowercase(),
        second.uuid.as_ref()?.to_ascii_lowercase(),
    ];
    if ids[0] == ids[1] {
        return None;
    }
    let mut exact = drc["unconnected_items"]
        .as_array()?
        .iter()
        .filter(|finding| {
            let Some([a, b]) = finding["items"].as_array().map(Vec::as_slice) else {
                return false;
            };
            [a, b].iter().all(|item| {
                [first, second].iter().zip(&ids).any(|(pad, id)| {
                    item["uuid"]
                        .as_str()
                        .is_some_and(|uuid| uuid.eq_ignore_ascii_case(id))
                        && item["pos"]["x"]
                            .as_f64()
                            .zip(item["pos"]["y"].as_f64())
                            .is_some_and(|(x, y)| distance_squared([x, y], pad.at) < 1e-6)
                })
            }) && a["uuid"] != b["uuid"]
        });
    exact.next()?;
    exact.next().is_none().then_some(([first, second], ids))
}

pub(super) fn probe(
    source: &Path,
    terminal: &NamedTerminal,
    drc: &serde_json::Value,
    run: &RunInputs,
    result: &LoweredBoard,
) -> Result<serde_json::Value, String> {
    let filename = format!("{}.kicad_pcb", run.files.board_id);
    let placed = source.join(&filename);
    let routed = run.files.directory.join(&filename);
    let before = file_sha256(&routed)?;
    if pcb_kicad_route::open_repair::report_capped(drc)
        || !drc["schematic_parity"]
            .as_array()
            .is_some_and(Vec::is_empty)
    {
        return Err("requires an uncapped KiCad report with clean schematic parity".into());
    }
    let Some((pads, ids)) = linked_pads(result, terminal, drc) else {
        return Ok(serde_json::json!({"status": "no_exact_two_pad_native_open"}));
    };
    for ext in ["kicad_pro", "kicad_dru", "kicad_sch"] {
        let original = placed.with_extension(ext);
        let actual = routed.with_extension(ext);
        if (original.is_file() || actual.is_file())
            && file_sha256(&original)? != file_sha256(&actual)?
        {
            return Err(format!(
                "the placed project differs from routing input at {ext}"
            ));
        }
    }
    let report = read_value(&source.join("board-placer.json"))?;
    if ["unplaced", "illegal"]
        .iter()
        .any(|key| !report[key].as_array().is_some_and(Vec::is_empty))
    {
        return Err("the placement report does not attest a legal placement".into());
    }
    let constraints = report["constraints"]
        .as_array()
        .ok_or("placement constraints are missing")?;
    let poses = report["footprints"]
        .as_array()
        .ok_or("placement poses are missing")?;
    let movable: Vec<_> = poses
        .iter()
        .filter(|part| part["fixed"] == false && part["side"] == "front")
        .filter_map(|part| part["reference"].as_str())
        .filter(|name| {
            !constraints
                .iter()
                .any(|constraint| names_part(constraint, name))
        })
        .collect();
    let config = run.config.as_ref().ok_or("project rules unavailable")?;
    let rules = config
        .connection_rules
        .get(&terminal.2)
        .or(config.default_rules.as_ref())
        .ok_or("project geometry unavailable for this net")?;
    let codes = crate::kicad_nets::net_codes(&result.pcb);
    let net = result
        .pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .flat_map(|footprint| footprint.children().iter())
        .find_map(|pad| {
            let uuid = form_atom(pad, "uuid", 1).or_else(|| form_atom(pad, "tstamp", 1))?;
            ids.iter()
                .any(|id| id.eq_ignore_ascii_case(uuid))
                .then(|| crate::kicad_nets::item_net(pad, &codes).map(str::to_owned))
                .flatten()
        })
        .ok_or("native net name is unavailable")?;
    let request = serde_json::json!({
        "routed": std::path::absolute(&routed).map_err(|error| error.to_string())?,
        "pads": ids, "net": net, "movable": movable,
        "width_mm": rules.trace_width_mm, "clearance_mm": rules.clearance_mm,
        "via_size_mm": rules.via_size_mm, "via_drill_mm": rules.via_drill_mm,
        "courtyard_spacing_mm": report["spacing_mm"].as_f64().ok_or("courtyard spacing is missing")?,
    });
    let version = drc["kicad_version"]
        .as_str()
        .ok_or("KiCad report version missing")?;
    let (mut advice, placed_hash) =
        crate::native_report::placement_escape_probe(&placed, version, &request)?;
    if file_sha256(&routed)? != before {
        return Err("the routed board changed during the probe".into());
    }
    advice["placed_board_sha256"] = serde_json::json!(placed_hash);
    advice["routed_board_sha256"] = serde_json::json!(before);
    advice["net"] = serde_json::json!(net);
    advice["pads"] = serde_json::json!(pads.map(|pad| format!("{}:{}", pad.reference, pad.number)));
    if advice["status"] == "same_face_move_screened" {
        let moved = advice["footprint"]
            .as_str()
            .ok_or("native probe did not name its moved footprint")?;
        if !movable.contains(&moved) {
            return Err("native probe did not select an eligible movable footprint".into());
        }
        if !poses.iter().any(|pose| {
            pose["reference"] == moved
                && pose["uuid"].is_string()
                && pose["uuid"] == advice["footprint_uuid"]
                && pose["at"][0] == advice["original_pose_mm"][0]
                && pose["at"][1] == advice["original_pose_mm"][1]
        }) {
            return Err(
                "the placement report does not match the blocked footprint's current pose".into(),
            );
        }
        let fixed: Vec<_> = poses
            .iter()
            .filter_map(|pose| pose["reference"].as_str())
            .filter(|name| *name != moved)
            .collect();
        advice["constraints_patch"]["fixed"] = serde_json::json!(fixed);
        advice["next_action"] = serde_json::json!(
            "Apply constraints_patch to a copy of the placement brief using the inspected placed project. Keep its face and mechanical declarations. Run placement, reroute, and compare native findings before keeping the candidate."
        );
    }
    Ok(advice)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_moving_a_part_mentioned_as_a_pin_or_constraint_member() {
        let rule = serde_json::json!({"source":"J51:1", "constraints":["U17", "U2:7"]});
        assert!(names_part(&rule, "J51"));
        assert!(names_part(&rule, "U2"));
        assert!(names_part(&rule, "U17"));
        assert!(!names_part(&rule, "J5"));
    }
}
