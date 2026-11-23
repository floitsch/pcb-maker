// Copyright (C) 2026 Toit contributors.

//! KiCad's existing import check on spatially separated footprints.

use super::{source_findings::footprint_findings, *};
use serde_json::Value;

struct PadGap {
    part: String,
    actual: f64,
    minimum: f64,
    authority: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadImportClearanceEvidence {
    pub reference: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supplied_footprint_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub netclass_gap: Option<KiCadImportNetclassGap>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadImportNetclassGap {
    pub pad_gap_mm: f64,
    pub rule: String,
    pub required_mm: f64,
    pub proposed_footprint_mm: f64,
    pub applied: bool,
}

/// Check the footprints apart from each other, in rows, with the project's
/// rules. On a stack KiCad stops reporting after 499 clearance errors.
fn isolated_footprint_drc(board: &Expr, project: &Path, board_id: &str) -> Result<Value, String> {
    let mut spread = board.clone();
    let Expr::List(items) = &mut spread else {
        return Ok(Value::Null);
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
    let directory = temporary_next_to(&project.join("pad-check"));
    fs::create_dir(&directory)
        .map_err(|error| format!("failed to create {}: {error}", directory.display()))?;
    let drc = (|| {
        for extension in ["kicad_pro", "kicad_dru"] {
            let name = format!("{board_id}.{extension}");
            let settings = project.join(&name);
            if settings.exists() {
                fs::copy(&settings, directory.join(&name))
                    .map_err(|error| format!("failed to copy {}: {error}", settings.display()))?;
            }
        }
        let board = directory.join(format!("{board_id}.kicad_pcb"));
        fs::write(&board, format!("{}\n", encode(&spread)))
            .map_err(|error| format!("failed to write {}: {error}", board.display()))?;
        let report = directory.join("drc.json");
        // Without kicad-cli (a netlist from elsewhere) there is no check.
        // KiCad's configuration without libraries spares their load.
        let mut command = Command::new(crate::native_report::kicad_cli());
        command
            .args(["pcb", "drc", "--format", "json", "--severity-error", "-o"])
            .arg(&report)
            .arg(&board);
        crate::kicad_config::configure_drc(
            &mut command,
            &crate::native_report::kicad_cli(),
            &board,
        );
        // Retry when KiCad exited 0 without a readable report.
        crate::native_report::with_one_rerun(|| {
            let _ = fs::remove_file(&report);
            let Ok(output) = crate::ledger::kicad_check(|| command.output()) else {
                return Ok(Ok(serde_json::Value::Null));
            };
            if !output.status.success() {
                return Err(format!(
                    "kicad-cli pcb drc failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            Ok(crate::native_report::read_kicad_report(&report))
        })
    })();
    let _ = fs::remove_dir_all(&directory);
    drc
}

/// Own-pad copper gaps with the authority KiCad named for each one.
fn own_pad_gaps(drc: &Value) -> Vec<PadGap> {
    // "Clearance violation (netclass 'Default' clearance 0.2800 mm; actual 0.2496 mm)"
    let millimetres = |text: &str| -> Option<f64> { text.split_whitespace().next()?.parse().ok() };
    let mut gaps = Vec::new();
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
        if let [Some(a), Some(b)] = owners.as_slice()
            && a == b
            && let Some((limit, actual)) = description
                .strip_prefix("Clearance violation (")
                .and_then(|text| text.split_once("; actual "))
            && let Some((authority, minimum)) = limit.rsplit_once(" clearance ")
            && let Some(minimum) = millimetres(minimum)
            && let Some(actual) = millimetres(actual)
        {
            gaps.push(PadGap {
                part: a.to_string(),
                actual,
                minimum,
                authority: authority.to_string(),
            });
        }
    }
    gaps
}

pub(super) struct FootprintChecks {
    pub clearances: Vec<String>,
    pub clearance_conflicts: Vec<String>,
    pub clearance_evidence: Vec<KiCadImportClearanceEvidence>,
    pub footprint_findings: Vec<KiCadImportFootprintFinding>,
}

/// Pads of one footprint closer than its netclass clearance (a solder
/// jumper's) can get their gap as their own clearance. A board minimum
/// or explicit KiCad rule still governs its gaps. Returns applied
/// clearances, retained violations and the same check's pad/drill findings.
pub(super) fn apply_pad_gap_clearances(
    board: &mut Expr,
    output: &Path,
    board_id: &str,
    board_path: &Path,
    keep_footprint_clearances: bool,
) -> Result<FootprintChecks, String> {
    let settings = read_json(&output.join(format!("{board_id}.kicad_pro"))).unwrap_or_default();
    let floor = settings["board"]["design_settings"]["rules"]["min_clearance"]
        .as_f64()
        .unwrap_or(0.0);
    let drc = isolated_footprint_drc(board, output, board_id)?;
    let footprint_findings = footprint_findings(board, &drc);
    let mut gaps = BTreeMap::new();
    let mut retained = BTreeMap::new();
    for gap in own_pad_gaps(&drc) {
        let netclass = gap.authority.starts_with("netclass ") || gap.authority.is_empty();
        let (authority, minimum) = if gap.authority == "board minimum" {
            ("board minimum (Board Setup)".to_string(), gap.minimum)
        } else if netclass && gap.actual < floor {
            ("board minimum (Board Setup)".to_string(), floor)
        } else if netclass {
            let entry = gaps
                .entry(gap.part)
                .or_insert_with(|| (gap.actual, gap.minimum, gap.authority.clone()));
            if gap.actual < entry.0 {
                *entry = (gap.actual, gap.minimum, gap.authority);
            }
            continue;
        } else {
            (gap.authority, gap.minimum)
        };
        let entry = retained
            .entry((gap.part, authority))
            .or_insert((gap.actual, minimum));
        if gap.actual < entry.0 {
            *entry = (gap.actual, minimum);
        }
    }
    let mut clearance_conflicts: Vec<_> = retained
        .into_iter()
        .map(|((part, authority), (gap, minimum))| {
            format!(
                "{part}: its pads are {gap} mm apart and {authority} requires {minimum} mm. Check the footprint's pad geometry and the applicable clearance rule."
            )
        })
        .collect();
    let mut clearances = Vec::new();
    let mut clearance_evidence = Vec::new();
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
        let supplied_footprint_mm = form_f64(footprint, "clearance", 1).ok();
        let evidence = gaps
            .get(&reference)
            .map(|(gap, required, rule)| KiCadImportNetclassGap {
                pad_gap_mm: *gap,
                rule: if rule.is_empty() {
                    "netclass".into()
                } else {
                    rule.clone()
                },
                required_mm: *required,
                proposed_footprint_mm: ((gap * 100.0).floor() / 100.0).max(floor),
                applied: !keep_footprint_clearances,
            });
        if let Some(evidence) = &evidence {
            let KiCadImportNetclassGap {
                pad_gap_mm,
                required_mm,
                proposed_footprint_mm,
                rule,
                applied,
            } = evidence;
            if *applied {
                let Expr::List(parts) = footprint else {
                    unreachable!("a footprint is a list")
                };
                parts.retain(|item| item.head() != Some("clearance"));
                let at = parts
                    .iter()
                    .position(|item| item.head() == Some("attr"))
                    .unwrap_or(parts.len());
                parts.insert(at, parse(&format!("(clearance {proposed_footprint_mm})"))?);
                clearances.push(format!(
                        "{reference}: {proposed_footprint_mm} mm (its pads are {pad_gap_mm} mm apart, the rule is {required_mm} mm)"
                    ));
            } else {
                clearance_conflicts.push(format!(
                        "{reference}: its pads are {pad_gap_mm} mm apart and {rule} requires {required_mm} mm. Preserved the supplied clearance policy. Review the pad geometry or specify an explicit project rule if this gap is intentional."
                    ));
            }
        }
        if supplied_footprint_mm.is_none() && evidence.is_none() {
            continue;
        }
        clearance_evidence.push(KiCadImportClearanceEvidence {
            reference,
            supplied_footprint_mm,
            netclass_gap: evidence,
        });
    }
    if !clearances.is_empty() {
        write_board(board_path, board)?;
    }
    Ok(FootprintChecks {
        clearances,
        clearance_conflicts,
        clearance_evidence,
        footprint_findings,
    })
}
