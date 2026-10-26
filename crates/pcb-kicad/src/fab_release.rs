// Copyright (C) 2026 Toit contributors.

//! A DRC-gated fab release (`fab-kicad-board`): KiCad's DRC (and ERC,
//! with a schematic) as `drc-kicad-board` runs it, and only when no copper
//! error and no unconnected item remains, everything a fab and an
//! assembler take, through kicad-cli: Gerbers, drill files with their map,
//! the placement file, the bill of materials, netlists, the 3D model, a
//! zip of the Gerbers and drill files, and `fab-manifest.json` (what was
//! exported, the checks, KiCad's version, a sha256 per file).

use super::*;

/// Conventions of a fab (`--profile`).
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadFabProfile {
    /// kicad-cli's own outputs: the layers the board uses.
    #[default]
    Kicad,
    /// JLCPCB's: every layer of their KiCad guide (both pastes and
    /// silkscreens), and the BOM and placement file in their columns
    /// (`Comment,Designator,Footprint,JLCPCB Part #`;
    /// `Designator,Mid X,Mid Y,Layer,Rotation`).
    Jlcpcb,
}

/// What `fab_kicad_board` exports and when it refuses.
#[derive(Clone, Debug, Default)]
pub struct KiCadFabOptions {
    /// Export even when copper errors or unconnected items remain.
    pub allow_findings: bool,
    /// The Gerber layers; default: those the board uses.
    pub layers: Option<Vec<String>>,
    pub profile: KiCadFabProfile,
    /// Skip the 3D model.
    pub no_step: bool,
}

/// KiCad's DRC as the release gate saw it.
#[derive(Clone, Debug, Default, Serialize)]
pub struct KiCadFabDrc {
    /// Error-severity findings by type, excluded and cosmetic ones left out,
    /// and a footprint's own (`footprint_own`): with unconnected items,
    /// what blocks a release.
    pub errors: BTreeMap<String, usize>,
    /// Error-severity findings by type that lie within one footprint each
    /// (its pads and holes as the library draws them, no short), which do
    /// not block: the verdict's `footprint_findings`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub footprint_own: BTreeMap<String, usize>,
    /// What those are and how KiCad's own DRC accepts them (a custom rule
    /// for the board's .kicad_dru, an exclusion).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footprint_note: Option<String>,
    pub unconnected_items: usize,
    pub warnings: usize,
    pub cosmetic: usize,
    /// No error and no unconnected item.
    pub clean: bool,
    /// Findings remained and `--allow-findings` let the release go ahead.
    pub findings_allowed: bool,
    pub report: String,
}

/// KiCad's ERC of the schematic (counted, never blocking).
#[derive(Clone, Debug, Default, Serialize)]
pub struct KiCadFabErc {
    pub errors: usize,
    pub warnings: usize,
    /// ERC's library warnings by type, not counted (`erc_library_warnings`).
    pub library_warnings: usize,
    pub report: String,
}

/// One exported file.
#[derive(Clone, Debug, Serialize)]
pub struct KiCadFabFile {
    /// Relative to the output directory.
    pub path: String,
    /// `gerber`, `gerber_job`, `drill`, `drill_map`, `placement`, `bom`, `netlist`,
    /// `step`, `zip`, `report`.
    pub kind: String,
    pub bytes: u64,
    pub sha256: String,
}

/// `fab-manifest.json`.
#[derive(Clone, Debug, Serialize)]
pub struct KiCadFabRelease {
    pub schema_version: u32,
    pub board_id: String,
    pub profile: KiCadFabProfile,
    pub kicad_version: Option<String>,
    /// Whether the fab files were written (the DRC passed, or findings
    /// were allowed).
    pub released: bool,
    pub drc: KiCadFabDrc,
    pub erc: Option<KiCadFabErc>,
    /// The Gerber layers plotted.
    pub layers: Vec<String>,
    /// `schematic` (kicad-cli's BOM of it) or `board` (its footprints).
    pub bom_source: Option<String>,
    /// The LCSC part number field of the schematic's symbols, if any.
    pub lcsc_field: Option<String>,
    /// Parts without an LCSC part number on the board.
    pub without_lcsc: Vec<String>,
    /// The zip of the Gerbers and drill files: what a fab's upload takes.
    pub zip: Option<String>,
    pub files: Vec<KiCadFabFile>,
    /// What was not exported, with why.
    pub skipped: Vec<String>,
    /// The first findings that blocked or were allowed, one line each.
    pub findings: Vec<String>,
}

/// Seconds an export may take (`PCB_KICAD_TIMEOUT` as for the checks,
/// default 900).
fn export_timeout() -> std::time::Duration {
    let seconds = std::env::var("PCB_KICAD_TIMEOUT")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| *value > 0.0)
        .unwrap_or(900.0);
    std::time::Duration::from_secs_f64(seconds)
}

/// Runs kicad-cli with `arguments`, its output captured, killed after the
/// export timeout.
fn kicad_cli(arguments: &[&std::ffi::OsStr], what: &str) -> Result<(), String> {
    let mut command = Command::new(crate::native_report::kicad_cli());
    command
        .args(arguments)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("{what}: failed to run kicad-cli: {error}"))?;
    let started = Instant::now();
    let timeout = export_timeout();
    // Drain the pipes on their own threads, so a chatty export never
    // blocks on a full pipe.
    let drain = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_string(&mut text);
            }
            text
        })
    };
    let out = drain(
        child
            .stdout
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn std::io::Read + Send>),
    );
    let err = drain(
        child
            .stderr
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn std::io::Read + Send>),
    );
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "{what}: kicad-cli timed out after {:.0} s",
                timeout.as_secs_f64()
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let transcript = format!(
        "{}{}",
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default()
    );
    if !status.success() {
        return Err(format!("{what} failed ({status}): {}", transcript.trim()));
    }
    Ok(())
}

fn os<S: AsRef<std::ffi::OsStr> + ?Sized>(value: &S) -> &std::ffi::OsStr {
    value.as_ref()
}

/// The layers of the board's layer table (`(layers (0 "F.Cu" signal) ...)`).
fn defined_layers(pcb: &Expr) -> Vec<String> {
    pcb.child("layers")
        .map(|layers| {
            layers
                .children()
                .iter()
                .skip(1)
                .filter_map(|entry| entry.children().get(1).and_then(Expr::atom))
                .map(|name| name.trim_matches('"').to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// The Gerber layers of a release: every copper layer, both masks and the
/// outline, and the pastes and silkscreens something lies on (all four
/// for JLCPCB, whose guide plots them all).
fn release_layers(pcb: &Expr, profile: KiCadFabProfile) -> Result<Vec<String>, String> {
    let mut layers = board_router::LayerTable::from_pcb(pcb)?.names.clone();
    // What lies on the board: its items, not the layer table or setup.
    let text: String = pcb
        .children()
        .iter()
        .filter(|item| !matches!(item.head(), Some("layers" | "setup" | "general")))
        .map(encode)
        .collect();
    let used = |side: &str, kind: &str| {
        profile == KiCadFabProfile::Jlcpcb
            || text.contains(&format!("\"{side}.{kind}\""))
            || text.contains(&format!("\"*.{kind}\""))
            || text.contains(&format!("\"F&B.{kind}\""))
    };
    for (side, kind) in [
        ("F", "Paste"),
        ("B", "Paste"),
        ("F", "SilkS"),
        ("B", "SilkS"),
    ] {
        if used(side, kind) {
            layers.push(format!("{side}.{kind}"));
        }
    }
    layers.extend(["F.Mask", "B.Mask", "Edge.Cuts"].map(String::from));
    // Keep the board's order of fab layers: copper, then the rest as
    // above, deduplicated.
    let mut seen = BTreeSet::new();
    layers.retain(|layer| seen.insert(layer.clone()));
    Ok(layers)
}

/// The schematic's LCSC part number field: the most common of the names
/// JLCPCB's tools and libraries use on its symbols.
fn lcsc_field(directory: &Path) -> Option<String> {
    const NAMES: [&str; 8] = [
        "LCSC",
        "LCSC Part",
        "LCSC Part #",
        "LCSC#",
        "LCSC Part Number",
        "JLCPCB Part #",
        "JLC",
        "JLCPCB",
    ];
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for entry in fs::read_dir(directory).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("kicad_sch") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for piece in text.split("(property \"").skip(1) {
            let Some(name) = piece.split('"').next() else {
                continue;
            };
            // A field with an empty value names no part.
            let value = piece.split('"').nth(2).unwrap_or("");
            if !value.is_empty() && NAMES.iter().any(|known| known.eq_ignore_ascii_case(name)) {
                *counts.entry(name.to_string()).or_default() += 1;
            }
        }
    }
    counts
        .into_iter()
        .max_by_key(|(name, count)| (*count, std::cmp::Reverse(name.clone())))
        .map(|(name, _)| name)
}

/// kicad-cli's position file (Ref,Val,Package,PosX,PosY,Rot,Side) in
/// JLCPCB's columns.
fn jlcpcb_placements(text: &str) -> String {
    let mut cpl = String::from("Designator,Mid X,Mid Y,Layer,Rotation\n");
    for line in text.lines().skip(1) {
        let columns = csv_columns(line);
        if columns.len() < 7 {
            continue;
        }
        let layer = if columns[6].eq_ignore_ascii_case("bottom") {
            "Bottom"
        } else {
            "Top"
        };
        cpl.push_str(&format!(
            "\"{}\",{}mm,{}mm,{layer},{}\n",
            columns[0].replace('"', "\"\""),
            columns[3],
            columns[4],
            columns[5]
        ));
    }
    cpl
}

/// The fields of one CSV line (double quotes, doubled inside).
fn csv_columns(line: &str) -> Vec<String> {
    let mut columns = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut characters = line.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '"' if quoted && characters.peek() == Some(&'"') => {
                current.push('"');
                characters.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => columns.push(std::mem::take(&mut current)),
            _ => current.push(character),
        }
    }
    columns.push(current);
    columns
}

/// The standard CRC-32 (zip's).
fn crc32(bytes: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = [0u32; 256];
        for (index, entry) in table.iter_mut().enumerate() {
            let mut value = index as u32;
            for _ in 0..8 {
                value = if value & 1 != 0 {
                    0xEDB8_8320 ^ (value >> 1)
                } else {
                    value >> 1
                };
            }
            *entry = value;
        }
        table
    });
    !bytes.iter().fold(!0u32, |crc, &byte| {
        table[((crc ^ byte as u32) & 0xFF) as usize] ^ (crc >> 8)
    })
}

/// Writes a zip of `files` (name in the archive, bytes), stored without
/// compression and with a fixed date, so the same files give the same
/// zip byte for byte.
pub(crate) fn write_zip(path: &Path, files: &[(String, Vec<u8>)]) -> Result<(), String> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    // 1980-01-01 00:00, the earliest date zip has.
    let (time, date) = (0u16, (1u16 << 5) | 1);
    for (name, bytes) in files {
        let size = u32::try_from(bytes.len())
            .map_err(|_| format!("{name} is too large for a zip without zip64"))?;
        let crc = crc32(bytes);
        let offset = u32::try_from(out.len()).map_err(|_| "the zip is too large".to_string())?;
        let name_bytes = name.as_bytes();
        let header = |signature: u32, central: bool| {
            let mut header = Vec::new();
            header.extend(signature.to_le_bytes());
            if central {
                header.extend(20u16.to_le_bytes()); // made by
            }
            header.extend(20u16.to_le_bytes()); // needed
            header.extend(0x0800u16.to_le_bytes()); // UTF-8 names
            header.extend(0u16.to_le_bytes()); // stored
            header.extend(time.to_le_bytes());
            header.extend(date.to_le_bytes());
            header.extend(crc.to_le_bytes());
            header.extend(size.to_le_bytes());
            header.extend(size.to_le_bytes());
            header.extend((name_bytes.len() as u16).to_le_bytes());
            header.extend(0u16.to_le_bytes()); // extra
            if central {
                header.extend(0u16.to_le_bytes()); // comment
                header.extend(0u16.to_le_bytes()); // disk
                header.extend(0u16.to_le_bytes()); // internal attributes
                header.extend(0u32.to_le_bytes()); // external attributes
                header.extend(offset.to_le_bytes());
            }
            header.extend(name_bytes);
            header
        };
        out.extend(header(0x0403_4b50, false));
        out.extend(bytes);
        central.extend(header(0x0201_4b50, true));
    }
    let central_offset = out.len() as u32;
    let central_size = central.len() as u32;
    out.extend(central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]); // disks
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend(central_size.to_le_bytes());
    out.extend(central_offset.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    fs::write(path, out).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn sha256_of(path: &Path) -> Result<(u64, String), String> {
    let bytes =
        fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    Ok((bytes.len() as u64, format!("{:x}", Sha256::digest(&bytes))))
}

/// The findings of a DRC report that count, one line each: errors
/// (neither excluded nor cosmetic), then unconnected items.
fn finding_lines(report: &serde_json::Value, limit: usize) -> Vec<String> {
    let describe = |finding: &serde_json::Value| {
        let items: Vec<String> = finding["items"]
            .as_array()
            .into_iter()
            .flatten()
            .take(2)
            .map(|item| {
                format!(
                    "{} at ({:.2}, {:.2})",
                    item["description"].as_str().unwrap_or("?"),
                    item["pos"]["x"].as_f64().unwrap_or(0.0),
                    item["pos"]["y"].as_f64().unwrap_or(0.0)
                )
            })
            .collect();
        format!(
            "{}: {}",
            finding["type"].as_str().unwrap_or("?"),
            items.join("; ")
        )
    };
    let mut lines: Vec<String> = report["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|finding| {
            finding["excluded"].as_bool() != Some(true)
                && finding["severity"].as_str() == Some("error")
                && !is_cosmetic(finding["type"].as_str().unwrap_or(""))
                && !footprint_own_unreferenced(&drc_finding(finding))
        })
        .map(describe)
        .collect();
    lines.extend(
        report["unconnected_items"]
            .as_array()
            .into_iter()
            .flatten()
            .map(describe),
    );
    lines.truncate(limit);
    lines
}

/// KiCad's DRC as the release gate judges it: the errors that block, and
/// apart from them a footprint's own findings (a receptacle's pads at its
/// own locating holes), which no layout changes and which do not block,
/// with the custom rule for `dru` that accepts them.
fn release_gate(tally: &crate::verdict::DrcTally, dru: &str) -> KiCadFabDrc {
    let own: Vec<&crate::verdict::DrcFinding> = tally
        .findings
        .iter()
        .filter(|finding| footprint_own_unreferenced(finding))
        .collect();
    let mut errors = tally.errors.clone();
    let mut footprint_own = BTreeMap::new();
    for finding in &own {
        *footprint_own.entry(finding.kind.clone()).or_insert(0) += 1;
        if let Some(count) = errors.get_mut(&finding.kind) {
            *count -= 1;
            if *count == 0 {
                errors.remove(&finding.kind);
            }
        }
    }
    KiCadFabDrc {
        clean: errors.is_empty() && tally.unconnected == 0,
        errors,
        footprint_note: footprint_findings_note(&own, dru),
        footprint_own,
        unconnected_items: tally.unconnected,
        warnings: tally.warnings,
        cosmetic: tally.cosmetic,
        findings_allowed: false,
        report: "drc.json".into(),
    }
}

/// The release of `<directory>/<board_id>.kicad_pcb` into `output` (which
/// exists): checks, the gate, then the exports (see the module's
/// comment). Returns the manifest, written as `fab-manifest.json`; with
/// `released: false` when findings blocked it.
pub fn fab_kicad_board(
    directory: &Path,
    board_id: &str,
    output: &Path,
    options: &KiCadFabOptions,
) -> Result<KiCadFabRelease, String> {
    let board = directory.join(format!("{board_id}.kicad_pcb"));
    let text = fs::read_to_string(&board)
        .map_err(|error| format!("failed to read {}: {error}", board.display()))?;
    let pcb = parse(&text).map_err(input_error)?;
    let defined = defined_layers(&pcb);
    let layers = match &options.layers {
        Some(layers) => {
            let unknown: Vec<&String> = layers
                .iter()
                .filter(|layer| !defined.contains(layer))
                .collect();
            if !unknown.is_empty() {
                return Err(input_error(format!(
                    "--layers names layers the board does not define: {unknown:?} (it defines {})",
                    defined.join(", ")
                )));
            }
            layers.clone()
        }
        // Only layers the board defines: kicad-cli plots no other.
        None => release_layers(&pcb, options.profile)?
            .into_iter()
            .filter(|layer| defined.is_empty() || defined.contains(layer))
            .collect(),
    };
    let program = crate::native_report::kicad_cli();
    let program = program.as_os_str();
    let mut manifest = KiCadFabRelease {
        schema_version: 1,
        board_id: board_id.into(),
        profile: options.profile,
        kicad_version: crate::drc_cache::tool_version(program),
        released: false,
        drc: KiCadFabDrc::default(),
        erc: None,
        layers: layers.clone(),
        bom_source: None,
        lcsc_field: None,
        without_lcsc: Vec::new(),
        zip: None,
        files: Vec::new(),
        skipped: Vec::new(),
        findings: Vec::new(),
    };
    // The checks: the board's DRC as drc-kicad-board runs it (cached,
    // without the global libraries, zones refilled), the schematic's ERC.
    let drc_path = output.join("drc.json");
    let drc = kicad_drc_report(directory, board_id, &drc_path, true)?;
    let dru = directory.join(format!("{board_id}.kicad_dru"));
    manifest.drc = release_gate(&tally_drc(&drc), &dru.display().to_string());
    let schematic = directory.join(format!("{board_id}.kicad_sch"));
    if schematic.is_file() {
        let erc_path = output.join("erc.json");
        run_kicad_report(
            &[
                "sch",
                "erc",
                "--format",
                "json",
                "--severity-all",
                "--output",
            ],
            &erc_path,
            &schematic,
        )?;
        let erc: serde_json::Value = read_json(&erc_path)?;
        let issues = crate::native_report::erc_design_issues(&erc);
        let all = erc["sheets"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|sheet| sheet["violations"].as_array().into_iter().flatten())
            .count();
        manifest.erc = Some(KiCadFabErc {
            errors: issues
                .iter()
                .filter(|finding| finding["severity"].as_str() == Some("error"))
                .count(),
            warnings: issues
                .iter()
                .filter(|finding| finding["severity"].as_str() != Some("error"))
                .count(),
            library_warnings: all - issues.len(),
            report: "erc.json".into(),
        });
    } else {
        manifest
            .skipped
            .push("ERC, schematic netlist: no schematic beside the board".into());
    }
    manifest.findings = finding_lines(&drc, 20);
    if !manifest.drc.clean && !options.allow_findings {
        write_manifest(output, &mut manifest)?;
        return Ok(manifest);
    }
    manifest.drc.findings_allowed = !manifest.drc.clean;
    manifest.released = true;

    // Gerbers and drill files.
    let gerbers = output.join("gerbers");
    fs::create_dir_all(&gerbers)
        .map_err(|error| format!("failed to create {}: {error}", gerbers.display()))?;
    let layer_list = layers.join(",");
    kicad_cli(
        &[
            os("pcb"),
            os("export"),
            os("gerbers"),
            os("--check-zones"),
            os("--subtract-soldermask"),
            os("--layers"),
            os(&layer_list),
            os("-o"),
            gerbers.as_os_str(),
            board.as_os_str(),
        ],
        "kicad-cli pcb export gerbers",
    )?;
    let drill_directory = format!("{}/", gerbers.display());
    kicad_cli(
        &[
            os("pcb"),
            os("export"),
            os("drill"),
            os("--format"),
            os("excellon"),
            os("--excellon-units"),
            os("mm"),
            os("--excellon-zeros-format"),
            os("decimal"),
            os("--excellon-oval-format"),
            os("alternate"),
            os("--drill-origin"),
            os("absolute"),
            os("--excellon-separate-th"),
            os("--generate-map"),
            os("--map-format"),
            os("gerberx2"),
            os("-o"),
            os(&drill_directory),
            board.as_os_str(),
        ],
        "kicad-cli pcb export drill",
    )?;
    let mut fab_files: Vec<PathBuf> = fs::read_dir(&gerbers)
        .map_err(|error| error.to_string())?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    fab_files.sort();
    let zip_name = format!("{board_id}-gerbers.zip");
    let mut entries = Vec::new();
    for path in &fab_files {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        entries.push((name, bytes));
    }
    write_zip(&output.join(&zip_name), &entries)?;
    manifest.zip = Some(zip_name);

    // Placement: kicad-cli's position file, both sides, DNP parts out.
    let assembly = output.join("assembly");
    fs::create_dir_all(&assembly)
        .map_err(|error| format!("failed to create {}: {error}", assembly.display()))?;
    let positions = assembly.join(format!("{board_id}-pos.csv"));
    kicad_cli(
        &[
            os("pcb"),
            os("export"),
            os("pos"),
            os("--format"),
            os("csv"),
            os("--units"),
            os("mm"),
            os("--side"),
            os("both"),
            os("--exclude-dnp"),
            os("-o"),
            positions.as_os_str(),
            board.as_os_str(),
        ],
        "kicad-cli pcb export pos",
    )?;
    if options.profile == KiCadFabProfile::Jlcpcb {
        let text = fs::read_to_string(&positions).map_err(|error| error.to_string())?;
        let cpl = assembly.join(format!("{board_id}-cpl-jlcpcb.csv"));
        fs::write(&cpl, jlcpcb_placements(&text))
            .map_err(|error| format!("failed to write {}: {error}", cpl.display()))?;
    }

    // The bill of materials: kicad-cli's of the schematic, grouped by
    // value, footprint and part number; else the board's footprints.
    let board_bom = crate::fab_export::board_bom(&pcb);
    manifest.without_lcsc = board_bom.without_lcsc.clone();
    let bom = assembly.join(format!("{board_id}-bom.csv"));
    if schematic.is_file() {
        let lcsc = lcsc_field(directory);
        manifest.lcsc_field = lcsc.clone();
        let (mut fields, mut labels, mut group) = (
            "Reference,Value,Footprint,${QUANTITY},${DNP}".to_string(),
            "Refs,Value,Footprint,Qty,DNP".to_string(),
            "Value,Footprint".to_string(),
        );
        if let Some(field) = &lcsc {
            fields.push_str(&format!(",{field}"));
            labels.push_str(&format!(",{field}"));
            group.push_str(&format!(",{field}"));
        }
        kicad_cli(
            &[
                os("sch"),
                os("export"),
                os("bom"),
                os("--fields"),
                os(&fields),
                os("--labels"),
                os(&labels),
                os("--group-by"),
                os(&group),
                os("-o"),
                bom.as_os_str(),
                schematic.as_os_str(),
            ],
            "kicad-cli sch export bom",
        )?;
        if options.profile == KiCadFabProfile::Jlcpcb {
            // JLCPCB's columns, each designator listed (no ranges), DNP
            // parts out.
            let jlcpcb = assembly.join(format!("{board_id}-bom-jlcpcb.csv"));
            let fields = format!(
                "Value,Reference,Footprint{}",
                lcsc.as_ref()
                    .map(|field| format!(",{field}"))
                    .unwrap_or_default()
            );
            let labels = format!(
                "Comment,Designator,Footprint{}",
                if lcsc.is_some() { ",JLCPCB Part #" } else { "" }
            );
            kicad_cli(
                &[
                    os("sch"),
                    os("export"),
                    os("bom"),
                    os("--fields"),
                    os(&fields),
                    os("--labels"),
                    os(&labels),
                    os("--group-by"),
                    os(&group),
                    os("--ref-range-delimiter"),
                    os(""),
                    os("--exclude-dnp"),
                    os("-o"),
                    jlcpcb.as_os_str(),
                    schematic.as_os_str(),
                ],
                "kicad-cli sch export bom (JLCPCB)",
            )?;
        }
        manifest.bom_source = Some("schematic".into());
    } else {
        fs::write(&bom, &board_bom.csv)
            .map_err(|error| format!("failed to write {}: {error}", bom.display()))?;
        manifest.bom_source = Some("board".into());
    }

    // Netlists: the board's IPC-D-356 (the fab's electrical test), the
    // schematic's KiCad netlist.
    let netlists = output.join("netlist");
    fs::create_dir_all(&netlists)
        .map_err(|error| format!("failed to create {}: {error}", netlists.display()))?;
    kicad_cli(
        &[
            os("pcb"),
            os("export"),
            os("ipcd356"),
            os("-o"),
            netlists.join(format!("{board_id}.d356")).as_os_str(),
            board.as_os_str(),
        ],
        "kicad-cli pcb export ipcd356",
    )?;
    if schematic.is_file() {
        kicad_cli(
            &[
                os("sch"),
                os("export"),
                os("netlist"),
                os("--format"),
                os("kicadsexpr"),
                os("-o"),
                netlists.join(format!("{board_id}.net")).as_os_str(),
                schematic.as_os_str(),
            ],
            "kicad-cli sch export netlist",
        )?;
    }

    // The 3D model: STEP with the parts' models where KiCad finds them; a
    // failure is noted, not fatal.
    if options.no_step {
        manifest.skipped.push("STEP: --no-step".into());
    } else {
        let step = output.join(format!("{board_id}.step"));
        if let Err(error) = kicad_cli(
            &[
                os("pcb"),
                os("export"),
                os("step"),
                os("--force"),
                os("--subst-models"),
                os("--no-dnp"),
                os("-o"),
                step.as_os_str(),
                board.as_os_str(),
            ],
            "kicad-cli pcb export step",
        ) {
            let _ = fs::remove_file(&step);
            manifest.skipped.push(format!(
                "STEP: {}",
                error.lines().next().unwrap_or_default()
            ));
        }
    }
    write_manifest(output, &mut manifest)?;
    Ok(manifest)
}

/// The kind of an exported file, by where it is and its name.
fn file_kind(relative: &str) -> &'static str {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    if relative.ends_with(".zip") {
        "zip"
    } else if relative.starts_with("gerbers/") {
        if name.ends_with(".drl") {
            "drill"
        } else if name.ends_with(".gbrjob") {
            "gerber_job"
        } else if name.contains("-drl_map") {
            "drill_map"
        } else {
            "gerber"
        }
    } else if relative.starts_with("assembly/") {
        if name.contains("-bom") {
            "bom"
        } else {
            "placement"
        }
    } else if relative.starts_with("netlist/") {
        "netlist"
    } else if name.ends_with(".step") {
        "step"
    } else {
        "report"
    }
}

/// Lists every file of `output` (but the manifest and the verdict) with
/// its sha256 and writes `fab-manifest.json`.
fn write_manifest(output: &Path, manifest: &mut KiCadFabRelease) -> Result<(), String> {
    fn walk(directory: &Path, found: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(directory).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, found);
            } else {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(output, &mut found);
    found.sort();
    manifest.files.clear();
    for path in found {
        let relative = path
            .strip_prefix(output)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if relative == "fab-manifest.json" || relative == "verdict.json" {
            continue;
        }
        let (bytes, sha256) = sha256_of(&path)?;
        manifest.files.push(KiCadFabFile {
            kind: file_kind(&relative).into(),
            path: relative,
            bytes,
            sha256,
        });
    }
    let path = output.join("fab-manifest.json");
    fs::write(
        &path,
        serde_json::to_string_pretty(manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_footprints_own_findings_do_not_block_the_release() {
        let own = serde_json::json!({"type": "hole_clearance", "severity": "error",
            "description": "Hole clearance violation (board setup constraints hole clearance 0.2500 mm; actual 0.1944 mm)",
            "items": [{"description": "Pad A1 [GND] of J1 on F.Cu", "pos": {"x": 1.0, "y": 1.0}},
                      {"description": "NPTH pad of J1", "pos": {"x": 1.0, "y": 2.0}}]});
        let report = |violations: Vec<serde_json::Value>| serde_json::json!({"violations": violations, "unconnected_items": []});
        let gate = release_gate(&tally_drc(&report(vec![own.clone()])), "p/b.kicad_dru");
        assert!(gate.clean && gate.errors.is_empty());
        assert_eq!(
            gate.footprint_own,
            BTreeMap::from([("hole_clearance".to_string(), 1)])
        );
        let note = gate.footprint_note.unwrap();
        assert!(
            note.contains("p/b.kicad_dru") && note.contains("min 0.19mm"),
            "{note}"
        );
        assert!(finding_lines(&report(vec![own.clone()]), 20).is_empty());
        // Between two parts, it blocks.
        let mut apart = own;
        apart["items"][1]["description"] = "NPTH pad of H1".into();
        let gate = release_gate(&tally_drc(&report(vec![apart.clone()])), "p/b.kicad_dru");
        assert!(!gate.clean && gate.footprint_note.is_none());
        assert_eq!(finding_lines(&report(vec![apart]), 20).len(), 1);
    }

    #[test]
    fn a_zip_holds_its_files_with_their_crcs() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let directory = std::env::temp_dir().join(format!(
            "pcb-maker-zip-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("a.zip");
        let files = vec![
            ("one.gbr".to_string(), b"G04 one*\n".to_vec()),
            ("two.drl".to_string(), Vec::new()),
        ];
        write_zip(&path, &files).unwrap();
        let first = fs::read(&path).unwrap();
        write_zip(&path, &files).unwrap();
        // The same files, the same bytes.
        assert_eq!(fs::read(&path).unwrap(), first);
        assert_eq!(&first[..4], &[0x50, 0x4b, 0x03, 0x04]);
        // Python's zipfile reads it back, when there is a python3.
        if let Ok(output) = Command::new("python3")
            .args([
                "-I",
                "-c",
                "import sys, zipfile; z = zipfile.ZipFile(sys.argv[1]); assert z.testzip() is None; print(','.join(z.namelist()), z.read('one.gbr').decode().strip())",
            ])
            .arg(&path)
            .output()
        {
            assert!(output.status.success(), "{output:?}");
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                "one.gbr,two.drl G04 one*"
            );
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn placements_take_jlcpcbs_columns() {
        let kicad = "Ref,Val,Package,PosX,PosY,Rot,Side\n\"C1\",\"100n\",\"C_0402\",10.5,-3.2,90.0,top\n\"U1\",\"MCU, \"\"big\"\"\",\"QFN\",1,2,180,bottom\n";
        assert_eq!(
            jlcpcb_placements(kicad),
            "Designator,Mid X,Mid Y,Layer,Rotation\n\"C1\",10.5mm,-3.2mm,Top,90.0\n\"U1\",1mm,2mm,Bottom,180\n"
        );
        assert_eq!(
            csv_columns("a,\"b,c\",\"d\"\"e\""),
            vec!["a", "b,c", "d\"e"]
        );
    }

    #[test]
    fn the_release_plots_the_layers_the_board_uses() {
        let text = r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (1 "F.Mask" user) (3 "B.Mask" user) (13 "F.Paste" user) (15 "B.Paste" user) (5 "F.SilkS" user) (7 "B.SilkS" user) (25 "Edge.Cuts" user))
          (footprint "a" (at 5 5) (fp_text reference "R1" (at 0 0) (layer "F.SilkS")) (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu" "F.Paste" "F.Mask"))))"#;
        let pcb = parse(text).unwrap();
        assert_eq!(
            release_layers(&pcb, KiCadFabProfile::Kicad).unwrap(),
            [
                "F.Cu",
                "B.Cu",
                "F.Paste",
                "F.SilkS",
                "F.Mask",
                "B.Mask",
                "Edge.Cuts"
            ]
        );
        assert_eq!(
            release_layers(&pcb, KiCadFabProfile::Jlcpcb).unwrap().len(),
            9
        );
        assert_eq!(defined_layers(&pcb).len(), 9);
        assert_eq!(file_kind("gerbers/b-F_Cu.gtl"), "gerber");
        assert_eq!(file_kind("gerbers/b-PTH.drl"), "drill");
        assert_eq!(file_kind("gerbers/b-PTH-drl_map.gbr"), "drill_map");
        assert_eq!(file_kind("assembly/b-bom.csv"), "bom");
        assert_eq!(file_kind("assembly/b-pos.csv"), "placement");
        assert_eq!(file_kind("b-gerbers.zip"), "zip");
        assert_eq!(file_kind("b.step"), "step");
        assert_eq!(file_kind("drc.json"), "report");
    }
}
