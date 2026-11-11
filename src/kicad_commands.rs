// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Commands that read, write or check KiCad files: the importer, the
//! fab export, the board strippers and shrinkers, the silkscreen and neck
//! repairs. Each takes the arguments after its name.

use std::{fs, path::Path};

use crate::{Arguments, agent_cli, lab_commands::board_router_config};

/// `verify-kicad-rung`: runs the completion policy on a materialized rung.
pub(crate) fn cmd_verify_kicad_rung(mut arguments: Arguments) -> Result<(), String> {
    let directory = arguments
        .next()
        .ok_or_else(|| "usage: pcb-maker verify-kicad-rung <directory> <board-id>".to_string())?;
    let board_id = arguments
        .next()
        .ok_or_else(|| "usage: pcb-maker verify-kicad-rung <directory> <board-id>".to_string())?;
    pcb_kicad::check_board_id(&board_id)?;
    if arguments.next().is_some() {
        return Err("usage: pcb-maker verify-kicad-rung <directory> <board-id>".into());
    }
    let report = pcb_kicad::verify_materialized_rung(Path::new(&directory), &board_id)?;
    println!(
        "{}: ERC={}, DRC={}, parity={}, selected-net-unconnected={}, intentional-no-connect-groups={}, library-metadata-warnings={}",
        if report.complete {
            "complete"
        } else {
            "incomplete"
        },
        report.erc_violations,
        report.drc_design_violations,
        report.schematic_parity_issues,
        report.selected_net_unconnected_items,
        report.intentional_no_connect_groups,
        report.library_metadata_warnings
    );
    if report.complete {
        Ok(())
    } else {
        Err("KiCad rung did not pass the completion policy".into())
    }
}

/// `repair-kicad-silkscreen`: moves silkscreen labels clear of what KiCad flags.
pub(crate) fn cmd_repair_kicad_silkscreen(mut arguments: Arguments) -> Result<(), String> {
    let usage =
        "usage: pcb-maker repair-kicad-silkscreen <source-directory> <board-id> <output-directory>";
    let source = arguments.next().ok_or(usage)?;
    let board_id = arguments.next().ok_or(usage)?;
    pcb_kicad::check_board_id(&board_id)?;
    let output = arguments.next().ok_or(usage)?;
    if arguments.next().is_some() {
        return Err(usage.into());
    }
    let report =
        pcb_kicad::repair_kicad_silkscreen(Path::new(&source), &board_id, Path::new(&output))?;
    println!(
        "wrote {output}: silkscreen-complete={}, remaining-annotation-findings={}",
        report["silkscreen_complete"], report["remaining_annotation_findings"]
    );
    if report["silkscreen_complete"] == true {
        Ok(())
    } else {
        let report_path = std::path::absolute(Path::new(&output).join("silkscreen-repair.json"))
            .unwrap_or_else(|_| Path::new(&output).join("silkscreen-repair.json"));
        Err(format!(
            "{}: {} silkscreen findings left; see {} (fields.blocked and proposal.unresolved_marks; KiCad's findings in drc.json and the rendering beside it): move those labels or marks by hand in KiCad, or give their parts room, and run repair-kicad-silkscreen {output} {board_id} <new-dir> again",
            agent_cli::SILKSCREEN_INCOMPLETE,
            report["remaining_annotation_findings"],
            report_path.display()
        ))
    }
}

/// `normalize-kicad-footprint-links`: normalizes the schematic-to-footprint links.
pub(crate) fn cmd_normalize_kicad_footprint_links(mut arguments: Arguments) -> Result<(), String> {
    let usage =
        "usage: pcb-maker normalize-kicad-footprint-links <directory> <board-id> [report.json]";
    let directory = arguments.next().ok_or_else(|| usage.to_string())?;
    let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
    pcb_kicad::check_board_id(&board_id)?;
    let report_path = arguments.next();
    if arguments.next().is_some() {
        return Err(usage.into());
    }
    let report = pcb_kicad::lab::normalize_kicad_footprint_links(Path::new(&directory), &board_id)?;
    let serialized = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    if let Some(report_path) = report_path {
        fs::write(&report_path, format!("{serialized}\n"))
            .map_err(|error| format!("failed to write {report_path}: {error}"))?;
        println!(
            "wrote {report_path}: normalized {} footprint link(s)",
            report.updated_references.len()
        );
    } else {
        println!("{serialized}");
    }
    Ok(())
}

/// `shrink-kicad-board`: scales a board down by a factor (a harder variant).
pub(crate) fn cmd_shrink_kicad_board(mut arguments: Arguments) -> Result<(), String> {
    let usage =
        "usage: pcb-maker shrink-kicad-board <source.kicad_pcb> <output.kicad_pcb> <factor>";
    let source = arguments.next().ok_or_else(|| usage.to_string())?;
    let output = arguments.next().ok_or_else(|| usage.to_string())?;
    let factor: f64 = arguments
        .next()
        .ok_or_else(|| usage.to_string())?
        .parse()
        .map_err(|error| format!("invalid factor: {error}"))?;
    if arguments.next().is_some() {
        return Err(usage.into());
    }
    let report =
        pcb_kicad::lab::shrink_kicad_board(Path::new(&source), Path::new(&output), factor)?;
    println!(
        "wrote {output}: {:.1} x {:.1} mm -> {:.1} x {:.1} mm, {} footprints, {} pushed back inside: {}",
        report.size_before_mm[0],
        report.size_before_mm[1],
        report.size_after_mm[0],
        report.size_after_mm[1],
        report.footprints,
        report.edge_bound.len(),
        report.edge_bound.join(" ")
    );
    Ok(())
}

/// `strip-kicad-tracks`: writes a board without tracks and vias (optionally keeping nets).
pub(crate) fn cmd_strip_kicad_tracks(mut arguments: Arguments) -> Result<(), String> {
    let usage = "usage: pcb-maker strip-kicad-tracks <source.kicad_pcb> <output.kicad_pcb> [--keep-nets <net>[,<net>...]]";
    let source = arguments.next().ok_or_else(|| usage.to_string())?;
    let output = arguments.next().ok_or_else(|| usage.to_string())?;
    let mut keep_nets: Vec<String> = Vec::new();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--keep-nets" => keep_nets.extend(
                arguments
                    .next()
                    .ok_or_else(|| usage.to_string())?
                    .split(',')
                    .map(str::trim)
                    .filter(|net| !net.is_empty())
                    .map(str::to_string),
            ),
            _ => return Err(usage.into()),
        }
    }
    let [kept, removed] = pcb_kicad::write_kicad_board_without_tracks_except(
        Path::new(&source),
        Path::new(&output),
        &keep_nets,
    )?;
    if keep_nets.is_empty() {
        println!("wrote {output} without tracks and vias; copper pours kept");
    } else {
        println!(
            "wrote {output}: kept {kept} tracks, arcs and vias of the nets {} and removed {removed} others; copper pours kept (route-kicad-board keeps the kept ones and routes the rest)",
            keep_nets.join(", ")
        );
    }
    Ok(())
}

/// `strip-kicad-copper`: writes a board without tracks, vias, zones and copper graphics.
pub(crate) fn cmd_strip_kicad_copper(mut arguments: Arguments) -> Result<(), String> {
    let usage =
        "usage: pcb-maker strip-kicad-copper <source.kicad_pcb> <output.kicad_pcb> [report.json]";
    let source = arguments.next().ok_or_else(|| usage.to_string())?;
    let output = arguments.next().ok_or_else(|| usage.to_string())?;
    let report_path = arguments.next();
    if arguments.next().is_some() {
        return Err(usage.into());
    }
    let report =
        pcb_kicad::write_kicad_board_without_copper(Path::new(&source), Path::new(&output))?;
    let serialized = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    if let Some(report_path) = report_path {
        fs::write(&report_path, format!("{serialized}\n"))
            .map_err(|error| format!("failed to write {report_path}: {error}"))?;
        println!(
            "wrote {output} and {report_path}: removed {} tracks/arcs, {} vias, {} copper zones, and {} copper graphics",
            report.removed_segments + report.removed_arcs,
            report.removed_vias,
            report.removed_copper_zones,
            report.removed_copper_graphics,
        );
    } else {
        println!("{serialized}");
    }
    Ok(())
}

/// `import-kicad-netlist`: builds a board project from a schematic or netlist.
pub(crate) fn cmd_import_kicad_netlist(mut arguments: Arguments) -> Result<(), String> {
    let usage = "usage: pcb-maker import-kicad-netlist <schematic.kicad_sch|netlist.net> <out-dir> <board-id> [--layers N] [--footprints-from <board.kicad_pcb>] [--near-decoupling]";
    let input = arguments.next().ok_or_else(|| usage.to_string())?;
    let output = arguments.next().ok_or_else(|| usage.to_string())?;
    let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
    pcb_kicad::check_board_id(&board_id)?;
    let mut layers = None;
    let mut footprints_from = None;
    let mut near_decoupling = false;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--near-decoupling" => near_decoupling = true,
            "--layers" => {
                layers = Some(
                    arguments
                        .next()
                        .and_then(|value| value.parse::<usize>().ok())
                        .filter(|value| *value >= 2 && value % 2 == 0)
                        .ok_or_else(|| usage.to_string())?,
                );
            }
            "--footprints-from" => {
                footprints_from = Some(std::path::PathBuf::from(
                    arguments.next().ok_or_else(|| usage.to_string())?,
                ));
            }
            _ => return Err(usage.into()),
        }
    }
    let report = pcb_kicad_import::import_kicad_netlist_options(
        Path::new(&input),
        Path::new(&output),
        &board_id,
        layers,
        footprints_from.as_deref(),
        near_decoupling,
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
    );
    if !report.missing.is_empty() {
        return Err(agent_cli::IMPORT_INCOMPLETE.into());
    }
    Ok(())
}

/// `export-kicad-fab`: exports the fabrication files of a board.
pub(crate) fn cmd_export_kicad_fab(mut arguments: Arguments) -> Result<(), String> {
    let usage = "usage: pcb-maker export-kicad-fab <project-dir> <board-id> <out-dir>";
    let directory = arguments.next().ok_or_else(|| usage.to_string())?;
    let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
    pcb_kicad::check_board_id(&board_id)?;
    let output = arguments.next().ok_or_else(|| usage.to_string())?;
    if arguments.next().is_some() {
        return Err(usage.into());
    }
    let report = pcb_kicad::export_kicad_fab(Path::new(&directory), &board_id, Path::new(&output))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
    );
    Ok(())
}

/// `score-kicad-board`: scores a placed board.
pub(crate) fn cmd_score_kicad_board(mut arguments: Arguments) -> Result<(), String> {
    let usage = "usage: pcb-maker score-kicad-board <project-dir> <board-id>";
    let directory = arguments.next().ok_or_else(|| usage.to_string())?;
    let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
    pcb_kicad::check_board_id(&board_id)?;
    if arguments.next().is_some() {
        return Err(usage.into());
    }
    let report = pcb_kicad::score_kicad_board(Path::new(&directory), &board_id)?;
    let mut json = serde_json::to_value(&report).map_err(|error| error.to_string())?;
    // The pours' pieces by the router's pour model (the verdict's
    // `pours`), with the project's rules.
    let config = board_router_config(None, &directory, &board_id)?;
    let pours = pcb_kicad::measure_pours(Path::new(&directory), &board_id, &config)?;
    json["pours"] = serde_json::to_value(&pours).map_err(|error| error.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json).map_err(|error| error.to_string())?
    );
    Ok(())
}

/// `describe-kicad-board`: describes a board's outline, parts and nets as JSON.
pub(crate) fn cmd_describe_kicad_board(mut arguments: Arguments) -> Result<(), String> {
    let usage = "usage: pcb-maker describe-kicad-board <project-dir> <board-id> [--constraints-template <file>]";
    let mut positional = Vec::new();
    let mut template = None;
    while let Some(argument) = arguments.next() {
        if argument == "--constraints-template" {
            template = Some(arguments.next().ok_or_else(|| usage.to_string())?);
        } else {
            positional.push(argument);
        }
    }
    let [directory, board_id] =
        <[String; 2]>::try_from(positional).map_err(|_| usage.to_string())?;
    pcb_kicad::check_board_id(&board_id)?;
    let description = pcb_kicad::describe_kicad_board(Path::new(&directory), &board_id)?;
    if let Some(file) = template {
        let constraints = pcb_kicad_import::constraints_template(Path::new(&directory), &board_id)?;
        fs::write(
            &file,
            serde_json::to_string_pretty(&constraints).map_err(|error| error.to_string())? + "\n",
        )
        .map_err(|error| format!("failed to write {file}: {error}"))?;
        let banks = constraints["comment"].as_array().map_or(0, |lines| {
            lines
                .iter()
                .filter(|line| line.as_str().is_some_and(|line| line.starts_with("bank: ")))
                .count()
        });
        eprintln!(
            "constraints template: {file} ({} edge, {} near entries{}; read its comments)",
            constraints["edge"].as_array().map_or(0, Vec::len),
            constraints["near"].as_array().map_or(0, Vec::len),
            if banks > 0 {
                format!(", {banks} bank of parts on two nets with its row in a comment")
            } else {
                String::new()
            }
        );
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&description).map_err(|error| error.to_string())?
    );
    Ok(())
}

/// `swap-kicad-pins`: swaps interchangeable pins to shorten routes.
pub(crate) fn cmd_swap_kicad_pins(mut arguments: Arguments) -> Result<(), String> {
    let usage = "usage: pcb-maker swap-kicad-pins <source-directory> <board-id> <output-directory> <pin-swaps.json> [config.json]";
    let source = arguments.next().ok_or_else(|| usage.to_string())?;
    let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
    pcb_kicad::check_board_id(&board_id)?;
    let output = arguments.next().ok_or_else(|| usage.to_string())?;
    let spec = pcb_kicad::read_pin_swap_spec(Path::new(
        &arguments.next().ok_or_else(|| usage.to_string())?,
    ))?;
    let config: pcb_kicad::KiCadPinSwapConfig = match arguments.next() {
        Some(path) => serde_json::from_str(
            &std::fs::read_to_string(&path)
                .map_err(|error| format!("failed to read {path}: {error}"))?,
        )
        .map_err(|error| format!("failed to parse {path}: {error}"))?,
        None => Default::default(),
    };
    let result = pcb_kicad::swap_kicad_pins(
        Path::new(&source),
        &board_id,
        Path::new(&output),
        &spec,
        &config,
    )?;
    println!(
        "{} groups, {} units: ratsnest {:.0} mm with {} crossings -> {:.0} mm with {} crossings; {} pins changed, {} schematic pins relabelled; {:.2}s",
        result.groups,
        result.units,
        result.before.length_mm,
        result.before.crossings,
        result.after.length_mm,
        result.after.crossings,
        result.changes.len(),
        result.schematic_labels_changed,
        result.seconds,
    );
    Ok(())
}

/// `repair-kicad-necks`: widens connection-width necks of a routed board.
pub(crate) fn cmd_repair_kicad_necks(mut arguments: Arguments) -> Result<(), String> {
    let usage = "usage: pcb-maker repair-kicad-necks <routed-directory> <board-id> [config.json|auto] (changes the directory in place)";
    let directory = arguments.next().ok_or_else(|| usage.to_string())?;
    let board_id = arguments.next().ok_or_else(|| usage.to_string())?;
    pcb_kicad::check_board_id(&board_id)?;
    let config = board_router_config(arguments.next().as_deref(), &directory, &board_id)?;
    let repair = pcb_kicad::repair_kicad_necks(Path::new(&directory), &board_id, &config)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&repair).map_err(|error| error.to_string())?
    );
    Ok(())
}
