// Copyright (C) 2026 Florian Loitsch.

//! Add a short connection between two schematic-proved decoupling pads.

use super::*;
use crate::open_repair::{item_id, propose_direct_pad_path, write_additions};
use serde_json::json;
use std::time::Duration;

#[derive(Default, Serialize)]
pub struct DecouplingShortcut {
    pub added_segments: usize,
    pub added_vias: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<crate::open_repair::LocalSearch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_publication: Option<ParallelFeedFill>,
}

/// Match an exact capacitor supply terminal and power input in a schematic export.
pub fn schematic_decoupling_pair(export: &Expr, labels: [&str; 2], net: &str) -> Option<()> {
    let [a, b] = labels.map(|label| label.split_once(':'));
    let a = a?;
    let b = b?;
    let pins = export
        .child("nets")?
        .children()
        .iter()
        .filter(|net| net.head() == Some("net"))
        .flat_map(|net| {
            net.children()
                .iter()
                .filter(|pin| pin.head() == Some("node"))
                .map(move |pin| (net, pin))
        })
        .collect::<Vec<_>>();
    let exact = |(reference, pad)| {
        let mut matching = pins.iter().filter(|(_, pin)| {
            form_atom(pin, "ref", 1) == Some(reference) && form_atom(pin, "pin", 1) == Some(pad)
        });
        let first = matching.next()?;
        (matching.next().is_none() && form_atom(first.0, "name", 1) == Some(net)).then_some(first.1)
    };
    for (cap, supply) in [(a, b), (b, a)] {
        let Some(cap_pin) = exact(cap) else {
            continue;
        };
        if exact(supply).and_then(|pin| form_atom(pin, "pintype", 1)) != Some("power_in")
            || form_atom(cap_pin, "pintype", 1) != Some("passive")
        {
            continue;
        }
        let capacitor =
            export.child("components")?.children().iter().find(|part| {
                part.head() == Some("comp") && form_atom(part, "ref", 1) == Some(cap.0)
            })?;
        let library = capacitor.child("libsource")?;
        if form_atom(library, "lib", 1) != Some("Device")
            || !matches!(
                form_atom(library, "part", 1),
                Some("C" | "C_Small" | "C_Polarized" | "C_Polarized_Small")
            )
        {
            continue;
        }
        let terminals = pins
            .iter()
            .filter(|(_, pin)| form_atom(pin, "ref", 1) == Some(cap.0))
            .collect::<Vec<_>>();
        if terminals.len() == 2
            && terminals.into_iter().any(|(return_net, pin)| {
                form_atom(pin, "pin", 1).is_some_and(|number| number != cap.1)
                    && form_atom(return_net, "name", 1).is_some_and(crate::quality::ground)
            })
        {
            return Some(());
        }
    }
    None
}

fn measurement(
    file: &Path,
    version: &str,
    pads: [&str; 2],
    report_path: &Path,
) -> Result<Value, String> {
    let (response, digest) = native_report::routed_path_probe(
        file,
        version,
        &json!({"pairs": [pads], "max_tracks_per_net": 512}),
    )?;
    if digest != file_sha256(file)? {
        return Err("the board changed while measuring its physical pad path".into());
    }
    write_typed_json(report_path, &response)?;
    let Some([path]) = response["paths"].as_array().map(Vec::as_slice) else {
        return Err("the native path probe did not return the requested pad pair".into());
    };
    if path["from"] != pads[0] || path["to"] != pads[1] || path["status"] != "measured" {
        return Err(path["reason"]
            .as_str()
            .unwrap_or("the native pad path could not be measured")
            .into());
    }
    Ok(path.clone())
}

fn is_new_direct_path(path: &Value, ids: &BTreeSet<String>) -> bool {
    path["intermediate_pads"]
        .as_array()
        .is_some_and(Vec::is_empty)
        && path["trace_segments"].as_array().is_some_and(|items| {
            !items.is_empty()
                && items
                    .iter()
                    .all(|item| item["uuid"].as_str().is_some_and(|id| ids.contains(id)))
        })
        && path["vias"].as_array().is_some_and(|items| {
            items
                .iter()
                .all(|item| item.as_str().is_some_and(|id| ids.contains(id)))
        })
}

pub(super) fn shorten(
    directory: &Path,
    board_id: &str,
    pads: [&str; 2],
    publish: ParallelFeedPublisher,
) -> Result<PadPathShortening, String> {
    let file = directory.join(format!("{board_id}.kicad_pcb"));
    let original = fs::read_to_string(&file).map_err(|error| error.to_string())?;
    let board = parse(&original)?;
    let project = project_companion_hashes(directory)?;
    let mut report = PadPathShortening {
        status: "unchanged",
        pads: pads.map(str::to_owned),
        span_measurement: "planar trace span outside KiCad pad and via copper",
        decoupling_shortcut: Some(DecouplingShortcut::default()),
        ..Default::default()
    };
    let codes = net_codes(&board);
    let [from_net, to_net] = pads.map(|pad| planner::pad_net(&board, &codes, pad));
    let Some(net) = from_net.filter(|net| to_net.as_ref() == Some(net) && pads[0] != pads[1])
    else {
        report.reason = "choose two distinct unique pads on the same named net".into();
        return Ok(report);
    };
    report.net = net.clone();
    let schematic = native_report::export_schematic_netlist(&file.with_extension("kicad_sch"))?;
    if schematic_decoupling_pair(&schematic, pads, &net).is_none() {
        report.reason = "the schematic pair must join a two-pin capacitor to a power-in pin, with the other capacitor terminal on ground".into();
        return Ok(report);
    }
    if governed_path(&file, &board, &net)? {
        report.reason = "the selected net has a differential or length requirement".into();
        return Ok(report);
    }
    let layers = LayerTable::from_pcb(&board)?;
    let geometry = |name| -> Option<_> {
        let (part, pad) = planner::find_pad(&board, name)?;
        if pad.children().get(2)?.atom()? != "smd" || pad.child("padstack").is_some() {
            return None;
        }
        let lowered = lower_pad(pad, form_at(part).ok()?).ok()?;
        let mask = layers.pad_mask(pad).ok()?;
        (mask.count_ones() == 1).then_some((name.replacen(':', ".", 1), lowered.center, mask))
    };
    let [Some(a), Some(b)] = pads.map(geometry) else {
        report.reason = "both selected pads need unique single-layer surface copper".into();
        return Ok(report);
    };
    let (additions, search) = propose_direct_pad_path(
        &board,
        &file,
        &net,
        [a, b]
            .each_ref()
            .map(|(label, at, mask)| (label.as_str(), *at, *mask)),
    )?;
    let shortcut = report
        .decoupling_shortcut
        .as_mut()
        .expect("shortcut report");
    shortcut.search = search;
    let vias = additions
        .iter()
        .filter(|object| object.head() == Some("via"))
        .count();
    let ids = additions
        .iter()
        .filter_map(item_id)
        .collect::<BTreeSet<_>>();
    if additions.is_empty()
        || additions.len() > 32
        || vias > 1
        || ids.len() != additions.len()
        || ids.iter().any(|id| original.contains(id))
    {
        report.reason =
            "the bounded search found no supported short path with at most one via".into();
        return Ok(report);
    }
    let reports_name = reserve_directory(directory, "pad-path-checks")?;
    let reports = directory.join(&reports_name);
    report.native_checks_directory = Some(reports_name);
    let before = [false, true]
        .into_iter()
        .zip(MODES)
        .map(|(refill, mode)| checked(&file, &reports, &format!("before-{mode}"), refill))
        .collect::<Result<Vec<_>, _>>()?;
    report.baseline_checks = before.len();
    if let Err(error) = MODES
        .into_iter()
        .zip(&before)
        .try_for_each(|(mode, native)| check_baseline(&reports, mode, native))
    {
        report.reason = error;
        return Ok(report);
    }
    let version = before[1]["kicad_version"]
        .as_str()
        .ok_or("missing native KiCad version")?;
    let measured = match measurement(&file, version, pads, &reports.join("path-before.json")) {
        Ok(measured) => measured,
        Err(reason) => {
            report.reason = reason;
            return Ok(report);
        }
    };
    let original_span = measured["trace_length_mm"]
        .as_f64()
        .ok_or("missing measured span")?;
    report.trace_span_before_mm = Some(original_span);
    report.trace_span_after_mm = Some(original_span);
    if fs::read_to_string(&file).map_err(|error| error.to_string())? != original
        || project_companion_hashes(directory)? != project
    {
        return Err("native checks changed the source board or project".into());
    }
    write_additions(&file, &original, &additions)?;
    let trial = fs::read_to_string(&file).map_err(|error| error.to_string())?;
    let accepted = (|| -> Result<(f64, Option<PathBuf>), String> {
        let after = measurement(&file, version, pads, &reports.join("path-trial.json"))?;
        let span = after["trace_length_mm"]
            .as_f64()
            .ok_or("missing trial path span")?;
        if original_span - span < 0.1 || !is_new_direct_path(&after, &ids) {
            return Err(
                "the added copper did not give a new direct measured path at least 0.1 mm shorter"
                    .into(),
            );
        }
        report.trial_checks += 1;
        let stored = checked(&file, &reports, "trial-0-stored", false)?;
        let needs_fill = match compare_trial(&before[0], &stored) {
            Ok(()) => false,
            Err(_) if feed::only_stale_fill_clearance(&board, &additions, &before[0], &stored)? => {
                true
            }
            Err(reason) => return Err(at_native_check(&reports, "trial-0-stored", &reason)),
        };
        report.trial_checks += 1;
        let refilled = checked(&file, &reports, "trial-0-refilled", true)?;
        compare_trial(&before[1], &refilled)
            .map_err(|reason| at_native_check(&reports, "trial-0-refilled", &reason))?;
        if fs::read_to_string(&file).map_err(|error| error.to_string())? != trial {
            return Err("native checks changed the trial board".into());
        }
        if !needs_fill {
            return Ok((span, None));
        }
        let stored_path = reports.join("trial-0-published-stored.json");
        let fill = publish(
            &file,
            &refilled,
            &stored_path,
            Instant::now() + Duration::from_secs(60),
        )?;
        report.trial_checks += fill.native_checks;
        let saved = fill.status == "saved";
        let reason = fill.reason.clone();
        shortcut.fill_publication = Some(fill);
        if !saved {
            return Err(reason.unwrap_or_else(|| "could not verify the stored fill".into()));
        }
        let reopened = native_report::read_kicad_report(&stored_path)
            .map_err(|reason| at_native_check(&reports, "trial-0-published-stored", &reason))?;
        compare_trial(&before[0], &reopened)
            .map_err(|reason| at_native_check(&reports, "trial-0-published-stored", &reason))?;
        let reopen = measurement(&file, version, pads, &reports.join("path-saved.json"))?;
        if reopen["trace_length_mm"] != after["trace_length_mm"]
            || !is_new_direct_path(&reopen, &ids)
        {
            return Err("saving the fill changed the measured shortcut".into());
        }
        Ok((span, Some(stored_path)))
    })();
    if project_companion_hashes(directory)? != project {
        return Err("native checks changed the project rules, schematic or libraries".into());
    }
    match accepted {
        Ok((span, saved)) => {
            record_kept_checks(directory, &reports, 0, saved.as_deref())?;
            shortcut.added_segments = additions.len() - vias;
            shortcut.added_vias = vias;
            report.trace_span_after_mm = Some(span);
            report.track_length_change_mm = additions
                .iter()
                .filter_map(|item| {
                    let [a, b] = [form_xy(item, "start").ok()?, form_xy(item, "end").ok()?];
                    Some(distance_squared(a, b).sqrt())
                })
                .sum();
            report.status = "shortened";
            report.reason = "the direct path shortened the measured span and passed stored and refilled KiCad checks".into();
        }
        Err(reason) => {
            fs::write(&file, original).map_err(|error| error.to_string())?;
            report.reason = reason;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMATIC: &str = r#"(export
      (components
        (comp (ref "C1") (libsource (lib "Device") (part "C")))
        (comp (ref "U1") (libsource (lib "Logic") (part "Buffer"))))
      (nets
        (net (name "/VREF")
          (node (ref "C1") (pin "2") (pintype "passive"))
          (node (ref "U1") (pin "8") (pintype "power_in"))
          (node (ref "R1") (pin "2") (pintype "passive")))
        (net (name "GND")
          (node (ref "C1") (pin "1") (pintype "passive")))
        (net (name "/UNRELATED")
          (node (ref "U1") (pin "3") (pintype "power_in")))))"#;

    #[test]
    fn schematic_requires_the_exact_capacitor_supply_and_return_pins() {
        let export = parse(SCHEMATIC).unwrap();
        assert!(schematic_decoupling_pair(&export, ["C1:2", "U1:8"], "/VREF").is_some());
        assert!(schematic_decoupling_pair(&export, ["U1:8", "C1:2"], "/VREF").is_some());
        for pads in [["C1:2", "R1:2"], ["C1:1", "U1:8"], ["C1:2", "U1:3"]] {
            assert!(schematic_decoupling_pair(&export, pads, "/VREF").is_none());
        }
        for changed in [
            SCHEMATIC.replace("GND", "/DATA"),
            SCHEMATIC.replace("(part \"C\")", "(part \"R\")"),
            SCHEMATIC.replace(
                "(pin \"8\") (pintype \"power_in\")",
                "(pin \"8\") (pintype \"input\")",
            ),
        ] {
            assert!(
                schematic_decoupling_pair(&parse(&changed).unwrap(), ["C1:2", "U1:8"], "/VREF")
                    .is_none()
            );
        }
    }

    #[test]
    fn measured_shortcut_cannot_use_intermediate_pads_or_original_copper() {
        let ids = BTreeSet::from(["new-track".into(), "new-via".into()]);
        let direct = json!({
            "intermediate_pads":[],
            "trace_segments":[{"uuid":"new-track"}],
            "vias":["new-via"]
        });
        assert!(is_new_direct_path(&direct, &ids));
        for [field, changed] in [["intermediate_pads", "C30:2"], ["vias", "existing-via"]] {
            let mut candidate = direct.clone();
            candidate[field] = json!([changed]);
            assert!(!is_new_direct_path(&candidate, &ids));
        }
        let mut candidate = direct.clone();
        candidate["trace_segments"][0]["uuid"] = json!("existing-track");
        assert!(!is_new_direct_path(&candidate, &ids));
    }
}
