// Copyright (C) 2026 Florian Loitsch.

//! Native paths and guarded recovery trials for authored near pairs.

use super::*;

/// Inspect a saved PCB without requiring a router report for its copper.
pub fn inspect_saved_near_paths(
    board: &Path,
    constraints: Option<&Path>,
    net: Option<&str>,
) -> serde_json::Value {
    let default = board.with_file_name("constraints.json");
    near_path_evidence(board, constraints.unwrap_or(&default), net)
        .unwrap_or_else(|reason| serde_json::json!({"status": "unavailable", "reason": reason}))
}

fn near_path_evidence(
    board: &Path,
    path: &Path,
    net: Option<&str>,
) -> Result<serde_json::Value, String> {
    if !path.is_file() {
        return Err("no authored near brief found; supply --near-constraints or keep constraints.json beside the PCB".into());
    }
    let brief_sha256 = crate::fs_util::file_sha256(path)?;
    let constraints = crate::read_placement_constraints(path)?;
    let near: Vec<_> = constraints
        .near
        .iter()
        .filter(|entry| entry.pin_of.is_some())
        .collect();
    if near.is_empty() {
        return Err("the brief has no near entries with a named pin_of".into());
    }
    let version = crate::check_kicad_cli()?;
    let (mut value, hash) = pcb_kicad_check::native_report::routed_path_probe(
        board,
        &version,
        &serde_json::json!({"near": near, "net": net}),
    )?;
    let paths = value["paths"]
        .as_array_mut()
        .ok_or("native pad-path result lacks its path list")?;
    if paths.is_empty() {
        return Err("no authored near pins match the selected net".into());
    }
    if crate::fs_util::file_sha256(path)? != brief_sha256 {
        return Err("the near constraints changed during measurement".into());
    }
    add_detour_recovery(board, paths);
    Ok(serde_json::json!({
        "status": "checked",
        "board": board,
        "board_sha256": hash,
        "constraints": path,
        "constraints_sha256": brief_sha256,
        "kicad_version": version,
        "paths": paths,
    }))
}

fn off_chord_pads(path: &serde_json::Value) -> Vec<&serde_json::Value> {
    if path["status"] != "measured" {
        return Vec::new();
    }
    let Some(spacing) = path["pad_center_distance_mm"].as_f64() else {
        return Vec::new();
    };
    // Compare two measured dimensions, without setting a trace limit.
    path["intermediate_pad_contacts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|contact| {
            contact["distance_from_pad_chord_mm"]
                .as_f64()
                .is_some_and(|offset| offset > spacing)
                && contact["pad"].is_string()
                && contact["pad_uuid"]
                    .as_str()
                    .is_some_and(|id| !id.is_empty())
        })
        .collect()
}

fn add_detour_recovery(board: &Path, paths: &mut [serde_json::Value]) {
    use serde_json::json;
    if !paths
        .iter()
        .any(|entry| !off_chord_pads(&entry["routed_path"]).is_empty())
    {
        return;
    }
    let Ok(board) = std::path::absolute(board) else {
        return;
    };
    let (Some(project), Some(board_id)) = (
        board.parent(),
        board.file_stem().and_then(std::ffi::OsStr::to_str),
    ) else {
        return;
    };
    if !board.with_extension("kicad_sch").is_file() {
        return;
    }
    let netlist = board.with_extension("net");
    let Ok(source) = fs::read_to_string(&netlist) else {
        return;
    };
    let Ok(export) = parse(&source) else { return };
    let Some(project_name) = project.file_name().and_then(std::ffi::OsStr::to_str) else {
        return;
    };
    let Some(project_text) = project.to_str() else {
        return;
    };
    for (index, entry) in paths.iter_mut().enumerate() {
        let path = &entry["routed_path"];
        let pads = off_chord_pads(path);
        let (Some(from), Some(to), Some(net)) = (
            path["from"].as_str(),
            path["to"].as_str(),
            path["net"].as_str(),
        ) else {
            continue;
        };
        if pads.is_empty()
            || pcb_kicad_route::schematic_decoupling_pair(&export, [from, to], net).is_none()
        {
            continue;
        }
        let output = project.with_file_name(format!("{project_name}-near-shortcut-{}", index + 1));
        if !output
            .symlink_metadata()
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            continue;
        }
        let Some(output_text) = output.to_str() else {
            continue;
        };
        entry["recovery"] = json!({
            "kind": "decoupling_shortcut_trial",
            "intermediate_pads_farther_from_chord_than_pad_spacing": pads.iter().map(|contact| &contact["pad"]).collect::<Vec<_>>(),
            "intermediate_pad_contacts": pads,
            "schematic_evidence": netlist,
            "reason": "The measured path visits a distant intermediate pad. The saved project netlist identifies the requested decoupling pair. The trial rechecks it against the live schematic and native rules.",
            "argv": ["pcb-maker", "shorten-kicad-pad-path", project_text, board_id, output_text,
                     "--pads", from, to, "--decoupling-shortcut"],
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chord_comparison_uses_actual_intermediate_pad_positions() {
        let mut path = serde_json::json!({
            "status":"measured", "pad_center_distance_mm":1.75,
            "intermediate_pad_contacts":[
                {"pad":"J3:2","pad_uuid":"distant-land","distance_from_pad_chord_mm":26.2},
                {"pad":"C30:2","pad_uuid":"near-land","distance_from_pad_chord_mm":0.635},
                {"pad":"J3:2","pad_uuid":"second-land","distance_from_pad_chord_mm":0.9},
                {"pad":"U8:3","distance_from_pad_chord_mm":100}
            ],
        });
        let pads = off_chord_pads(&path);
        assert_eq!(pads.len(), 1);
        assert_eq!(pads[0]["pad_uuid"], "distant-land");
        path["status"] = serde_json::json!("unavailable");
        assert!(off_chord_pads(&path).is_empty());
    }
}
