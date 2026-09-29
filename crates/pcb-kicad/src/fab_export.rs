// Copyright (C) 2026 Toit contributors.

//! What a fab takes, from a finished board: Gerbers and drill files (zipped),
//! a bill of materials and a placement (CPL) file in JLCPCB's format, and
//! the cost estimate. Everything but the BOM goes through `kicad-cli`, so the
//! files are exactly what KiCad would plot.

use super::*;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadFabExport {
    pub board_id: String,
    pub gerbers_zip: PathBuf,
    pub bom: PathBuf,
    pub cpl: PathBuf,
    /// BOM lines (distinct parts) and placements.
    pub bom_lines: usize,
    pub placements: usize,
    /// Parts without an LCSC part number (JLCPCB assembly needs one).
    pub without_lcsc: Vec<String>,
    pub cost: crate::cost::KiCadCost,
}

fn run(command: &mut Command, what: &str) -> Result<(), String> {
    let output = command.output().map_err(|error| format!("{what}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{what} failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

/// A CSV field, quoted.
fn field(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

pub fn export_kicad_fab(directory: &Path, board_id: &str, output: &Path) -> Result<KiCadFabExport, String> {
    let board = directory.join(format!("{board_id}.kicad_pcb"));
    if !board.exists() {
        return Err(format!("{} does not exist", board.display()));
    }
    let gerbers = output.join("gerbers");
    fs::create_dir_all(&gerbers).map_err(|error| format!("failed to create {}: {error}", gerbers.display()))?;
    // Zones filled as KiCad fills them; the layers a fab makes the board
    // from (copper, mask, paste, silkscreen, outline), nothing else.
    let pcb = parse(&fs::read_to_string(&board).map_err(|error| error.to_string())?)?;
    let mut layers = board_router::LayerTable::from_pcb(&pcb)?.names.clone();
    layers.extend(["F.Mask", "B.Mask", "F.Paste", "B.Paste", "F.SilkS", "B.SilkS", "Edge.Cuts"].map(String::from));
    run(
        Command::new("kicad-cli")
            .args(["pcb", "export", "gerbers", "--check-zones", "--subtract-soldermask", "--layers"])
            .arg(layers.join(","))
            .arg("-o")
            .arg(&gerbers)
            .arg(&board),
        "kicad-cli gerbers",
    )?;
    run(
        Command::new("kicad-cli")
            .args(["pcb", "export", "drill", "--format", "excellon", "--excellon-separate-th", "--generate-map", "--map-format", "gerberx2", "-o"])
            .arg(format!("{}/", gerbers.display()))
            .arg(&board),
        "kicad-cli drill",
    )?;
    let gerbers_zip = output.join(format!("{board_id}-gerbers.zip"));
    run(
        Command::new("python3").args([
            "-c",
            "import shutil, sys; shutil.make_archive(sys.argv[1], 'zip', sys.argv[2])",
            &gerbers_zip.with_extension("").display().to_string(),
            &gerbers.display().to_string(),
        ]),
        "zip",
    )?;

    // Placements from KiCad, in JLCPCB's columns.
    let positions = output.join("positions.csv");
    run(
        Command::new("kicad-cli")
            .args(["pcb", "export", "pos", "--format", "csv", "--units", "mm", "--side", "both", "--exclude-dnp", "-o"])
            .arg(&positions)
            .arg(&board),
        "kicad-cli pos",
    )?;
    let text = fs::read_to_string(&positions).map_err(|error| error.to_string())?;
    let mut cpl = String::from("Designator,Mid X,Mid Y,Layer,Rotation\n");
    let mut placements = 0;
    for line in text.lines().skip(1) {
        // Ref,Val,Package,PosX,PosY,Rot,Side
        let columns: Vec<String> = line.split(',').map(|column| column.trim_matches('"').to_string()).collect();
        if columns.len() < 7 {
            continue;
        }
        let layer = if columns[6].eq_ignore_ascii_case("bottom") { "Bottom" } else { "Top" };
        cpl.push_str(&format!("{},{}mm,{}mm,{},{}\n", field(&columns[0]), columns[3], columns[4], layer, columns[5]));
        placements += 1;
    }
    let cpl_path = output.join(format!("{board_id}-cpl.csv"));
    fs::write(&cpl_path, cpl).map_err(|error| error.to_string())?;

    // Bill of materials from the board's footprints: one line per value and
    // footprint, with the LCSC part number when a footprint carries one.
    let mut lines: BTreeMap<(String, String, String), Vec<String>> = BTreeMap::new();
    for footprint in pcb.children().iter().filter(|item| item.head() == Some("footprint")) {
        let property = |names: &[&str]| {
            footprint
                .children()
                .iter()
                .filter(|item| item.head() == Some("property"))
                .find(|item| {
                    item.children()
                        .get(1)
                        .and_then(Expr::atom)
                        .is_some_and(|name| names.iter().any(|wanted| name.eq_ignore_ascii_case(wanted)))
                })
                .and_then(|item| item.children().get(2).and_then(Expr::atom))
                .unwrap_or("")
                .to_string()
        };
        let reference = property(&["Reference"]);
        let excluded = footprint
            .child("attr")
            .is_some_and(|attr| attr.children().iter().any(|a| matches!(a.atom(), Some("exclude_from_bom" | "board_only" | "dnp"))));
        let has_pads = footprint.children().iter().any(|child| child.head() == Some("pad"));
        if reference.is_empty() || excluded || !has_pads {
            continue;
        }
        let package = footprint.children().get(1).and_then(Expr::atom).unwrap_or("");
        let package = package.rsplit(':').next().unwrap_or(package).to_string();
        let lcsc = property(&["LCSC", "LCSC Part", "LCSC Part #", "JLCPCB Part #", "JLC", "LCSC#"]);
        lines.entry((property(&["Value"]), package, lcsc)).or_default().push(reference);
    }
    let mut bom = String::from("Comment,Designator,Footprint,JLCPCB Part #\n");
    let mut without_lcsc = Vec::new();
    for ((value, package, lcsc), mut references) in lines.clone() {
        references.sort_by_key(|reference| {
            let digits: String = reference.chars().skip_while(|c| !c.is_ascii_digit()).collect();
            (reference.chars().take_while(|c| !c.is_ascii_digit()).collect::<String>(), digits.parse::<u64>().unwrap_or(0))
        });
        if lcsc.is_empty() {
            without_lcsc.extend(references.iter().cloned());
        }
        bom.push_str(&format!("{},{},{},{}\n", field(&value), field(&references.join(",")), field(&package), field(&lcsc)));
    }
    let bom_path = output.join(format!("{board_id}-bom.csv"));
    fs::write(&bom_path, bom).map_err(|error| error.to_string())?;

    let cost = crate::quality::score_kicad_board(directory, board_id)?.cost;
    let report = KiCadFabExport {
        board_id: board_id.into(),
        gerbers_zip,
        bom: bom_path,
        cpl: cpl_path,
        bom_lines: lines.len(),
        placements,
        without_lcsc,
        cost,
    };
    fs::write(
        output.join("fab-export.json"),
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(report)
}
