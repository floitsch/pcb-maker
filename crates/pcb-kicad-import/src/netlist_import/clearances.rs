// Copyright (C) 2026 Toit contributors.

//! Footprints whose own pads are closer than the board's clearance.

use super::*;

/// Per footprint, the smallest gap between its own pads that KiCad's DRC
/// finds below the clearance rule, and that rule. The footprints are
/// checked apart from each other, in rows, with the project's rules (on a
/// stack of footprints KiCad stops reporting after 499 clearance errors).
pub(super) fn own_pad_gaps(
    board: &Expr,
    project: &Path,
    board_id: &str,
) -> Result<BTreeMap<String, (f64, f64)>, String> {
    let mut spread = board.clone();
    let Expr::List(items) = &mut spread else {
        return Ok(BTreeMap::new());
    };
    let (mut x, mut y, mut row) = (0.0f64, 0.0f64, 0.0f64);
    for footprint in items
        .iter_mut()
        .filter(|item| item.head() == Some("footprint"))
    {
        // Pads reach this far from the origin.
        let reach = footprint
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
            .map(|pad| {
                let at = form_at(pad).unwrap_or([0.0; 3]);
                let size = form_f64(pad, "size", 1)
                    .unwrap_or(0.0)
                    .max(form_f64(pad, "size", 2).unwrap_or(0.0));
                at[0].abs().max(at[1].abs()) + size
            })
            .fold(0.0, f64::max)
            + 5.0;
        if x + 2.0 * reach > 1000.0 {
            (x, y, row) = (0.0, y + row, 0.0);
        }
        if let Expr::List(parts) = footprint
            && let Some(at) = parts.iter_mut().find(|item| item.head() == Some("at"))
        {
            *at = parse(&format!("(at {} {})", x + reach, y + reach))?;
        }
        x += 2.0 * reach;
        row = row.max(2.0 * reach);
    }
    let directory = project.join(".pad-check");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("failed to create {}: {error}", directory.display()))?;
    let settings = project.join(format!("{board_id}.kicad_pro"));
    if settings.exists() {
        fs::copy(&settings, directory.join(format!("{board_id}.kicad_pro")))
            .map_err(|error| error.to_string())?;
    }
    let board = directory.join(format!("{board_id}.kicad_pcb"));
    fs::write(&board, format!("{}\n", encode(&spread)))
        .map_err(|error| format!("failed to write {}: {error}", board.display()))?;
    let report = directory.join("drc.json");
    // Without kicad-cli (a netlist from elsewhere) there is no check.
    // Only clearance findings are read: KiCad's configuration without
    // libraries (`kicad_config`) spares the footprint library load.
    let mut command = Command::new(crate::native_report::kicad_cli());
    command
        .args(["pcb", "drc", "--format", "json", "--severity-error", "-o"])
        .arg(&report)
        .arg(&board);
    crate::kicad_config::configure_drc(&mut command, &crate::native_report::kicad_cli(), &board);
    let Ok(output) = crate::ledger::kicad_check(|| command.output()) else {
        let _ = fs::remove_dir_all(&directory);
        return Ok(BTreeMap::new());
    };
    let text = fs::read_to_string(&report);
    let _ = fs::remove_dir_all(&directory);
    if !output.status.success() {
        return Err(format!(
            "kicad-cli pcb drc failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text =
        text.map_err(|error| format!("failed to read the pad check's DRC report: {error}"))?;
    let drc: serde_json::Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    // "Clearance violation (netclass 'Default' clearance 0.2800 mm; actual 0.2496 mm)"
    let number_after = |text: &str, key: &str| -> Option<f64> {
        let rest = &text[text.find(key)? + key.len()..];
        rest.split_whitespace().next()?.parse().ok()
    };
    let mut gaps: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    for violation in drc["violations"].as_array().into_iter().flatten() {
        if violation["type"] != "clearance" {
            continue;
        }
        let owners: Vec<Option<&str>> = violation["items"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|item| {
                let description = item["description"].as_str()?;
                description.starts_with("Pad ").then_some(())?;
                description.rsplit_once(" of ")?.1.split_whitespace().next()
            })
            .collect();
        let description = violation["description"].as_str().unwrap_or("");
        if let ([Some(a), Some(b)], Some(gap), Some(rule)) = (
            owners.as_slice(),
            number_after(description, "actual"),
            number_after(description, "clearance"),
        ) && a == b
        {
            let entry = gaps.entry(a.to_string()).or_insert((gap, rule));
            entry.0 = entry.0.min(gap);
        }
    }
    Ok(gaps)
}

/// Pads of one footprint closer than the board's clearance (a solder
/// jumper's): no placement or routing changes that. The footprint gets
/// its pads' gap as its own clearance, as designers do; the board file is
/// written again when any does. Returns the report's lines.
pub(super) fn apply_pad_gap_clearances(
    board: &mut Expr,
    output: &Path,
    board_id: &str,
    board_path: &Path,
) -> Result<Vec<String>, String> {
    let mut clearances = Vec::new();
    let gaps = own_pad_gaps(board, output, board_id)?;
    if !gaps.is_empty() {
        let Expr::List(items) = &mut *board else {
            unreachable!("the board is a list")
        };
        for footprint in items
            .iter_mut()
            .filter(|item| item.head() == Some("footprint"))
        {
            let reference = footprint
                .children()
                .iter()
                .find(|item| {
                    item.head() == Some("property")
                        && item.children().get(1).and_then(Expr::atom) == Some("Reference")
                })
                .and_then(|item| item.children().get(2).and_then(Expr::atom))
                .unwrap_or("")
                .to_string();
            let Some((gap, rule)) = gaps.get(&reference) else {
                continue;
            };
            let clearance = (gap * 100.0).floor() / 100.0;
            let Expr::List(parts) = footprint else {
                continue;
            };
            parts.retain(|item| item.head() != Some("clearance"));
            let at = parts
                .iter()
                .position(|item| item.head() == Some("attr"))
                .unwrap_or(parts.len());
            parts.insert(at, parse(&format!("(clearance {clearance})"))?);
            clearances.push(format!(
                "{reference}: {clearance} mm (its pads are {gap} mm apart, the rule is {rule} mm)"
            ));
        }
        write_board(board_path, board)?;
    }
    Ok(clearances)
}
