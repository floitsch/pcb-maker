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
    /// The project the board was laid out or routed from (`--input`): only
    /// the parts still where its board has them, and its zones and board
    /// graphics unchanged, are the input's (`input_own`); `None`: the
    /// board's every footprint, zone and board graphic is its own.
    pub input: Option<PathBuf>,
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
    /// Error-severity findings by type that are the board's own and not
    /// electrical (`input_own` with every footprint, zone and board graphic
    /// the board's: a pad at the board edge, a fiducial's mask opening over
    /// a zone, two parts' courtyards), which do not block: the verdict's
    /// `input_findings`. A track or a via in a finding, and a short or a
    /// clearance between two nets, still block.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub input_own: BTreeMap<String, usize>,
    /// What those are and how KiCad's own DRC accepts them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_note: Option<String>,
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

/// The next action for parts without an LCSC part number: how many BOM
/// lines that is (the numbers to find) and where the number goes, by where
/// the BOM came from. Without a schematic beside the board (a layout's
/// result) the symbol's field and Update PCB from Schematic cannot be
/// followed there (trial 7: MIDAS-MK2's 207 parts on 78 lines).
pub(crate) fn lcsc_action(manifest: &KiCadFabRelease) -> Option<String> {
    if manifest.without_lcsc.is_empty() {
        return None;
    }
    let shown = manifest.without_lcsc.len().min(10);
    let lines = match manifest.without_lcsc_lines {
        0 => String::new(),
        1 => " on 1 BOM line".to_string(),
        lines => format!(" on {lines} BOM lines (value and footprint: one number each)"),
    };
    let how = if manifest.bom_source.as_deref() == Some("schematic") {
        format!(
            "a field {} on the symbols, then Update PCB from Schematic and release again",
            manifest
                .lcsc_field
                .as_deref()
                .map_or("LCSC".to_string(), |field| format!("\"{field}\""))
        )
    } else {
        format!(
            "the BOM comes from the board's footprints (no schematic beside it): a property {} on each footprint (KiCad's board editor, footprint properties; or the symbol's field in the project it came from, Update PCB from Schematic and lay out again), then release again",
            crate::fab_export::BOARD_LCSC_PROPERTIES
                .iter()
                .map(|name| format!("\"{name}\""))
                .collect::<Vec<_>>()
                .join(" or ")
        )
    };
    Some(format!(
        "{} parts{lines} have no LCSC part number, which JLCPCB's assembly needs ({how}): {}{} (fab-manifest.json's without_lcsc lists them)",
        manifest.without_lcsc.len(),
        manifest.without_lcsc[..shown].join(", "),
        if shown < manifest.without_lcsc.len() {
            ", ..."
        } else {
            ""
        }
    ))
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
    /// The BOM lines (value and footprint) of those parts: the part
    /// numbers to find.
    pub without_lcsc_lines: usize,
    /// The zip of the Gerbers and drill files: what a fab's upload takes.
    pub zip: Option<String>,
    pub files: Vec<KiCadFabFile>,
    /// What was not exported, with why.
    pub skipped: Vec<String>,
    /// The first findings that blocked or were allowed, one line each.
    pub findings: Vec<String>,
    /// The board's layer stack as its file gives it (Board Setup's
    /// Physical Stackup, which the job file carries too): the board's
    /// thickness, its copper finish and per layer its type, thickness,
    /// material and dielectric constant. Absent where the board states
    /// no stackup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stackup: Option<KiCadFabStackup>,
    /// What the fab's impedance control is for: the classes that look
    /// like a controlled impedance (widths per layer from the custom
    /// rules, or a class narrower than Default) and the custom rules'
    /// track geometry on some layers only, with the action to order it.
    /// Absent where nothing asks for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controlled_impedance: Option<crate::KiCadControlledImpedance>,
    /// The polarised parts the assembler places (diodes, LEDs,
    /// transistors, ICs, polarised capacitors; by reference and footprint
    /// name): the parts whose rotation to check in the fab's placement
    /// preview. Empty before the placement file is written.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub polarised_parts: Vec<String>,
}

/// A board's layer stack (`KiCadFabRelease::stackup`).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct KiCadFabStackup {
    /// The board's thickness (`general`'s), mm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thickness_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copper_finish: Option<String>,
    pub layers: Vec<KiCadFabStackupLayer>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct KiCadFabStackupLayer {
    pub name: String,
    /// KiCad's type: `copper`, `core`, `prepreg`, `Top Solder Mask`, ...
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thickness_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epsilon_r: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loss_tangent: Option<f64>,
}

/// Copper heavier than 1 oz (35 um) in the stackup, which fabs make only
/// when the order says so: the action naming the layers and their weight
/// (trial 8: the 4in1-mini's outer layers are 2 oz, 0.07 mm, its custom
/// rules widen the outer tracks for it, and the release said nothing).
fn copper_weight_action(stackup: &KiCadFabStackup) -> Option<String> {
    let heavy: Vec<(String, f64)> = stackup
        .layers
        .iter()
        .filter(|layer| layer.kind == "copper")
        .filter_map(|layer| {
            let thickness = layer.thickness_mm?;
            (thickness > 0.04).then(|| (layer.name.clone(), thickness))
        })
        .collect();
    if heavy.is_empty() {
        return None;
    }
    let ounces = |thickness: f64| ((thickness / 0.035) * 2.0).round() / 2.0;
    let mut by_weight: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, thickness) in &heavy {
        by_weight
            .entry(format!("{} mm, {} oz", thickness, ounces(*thickness)))
            .or_default()
            .push(name.clone());
    }
    let listed: Vec<String> = by_weight
        .iter()
        .map(|(weight, layers)| format!("{} ({weight})", layers.join(", ")))
        .collect();
    Some(format!(
        "the stackup has copper heavier than 1 oz: {}; a fab makes 1 oz unless the order says otherwise, so order that copper weight (fab-manifest.json's stackup)",
        listed.join("; ")
    ))
}

/// The stackup of `pcb` (`setup`'s `stackup`), `None` without one.
pub(crate) fn board_stackup(pcb: &Expr) -> Option<KiCadFabStackup> {
    let stackup = pcb.child("setup")?.child("stackup")?;
    let number =
        |item: &Expr, key: &str| form_atom(item, key, 1).and_then(|value| value.parse().ok());
    let layers: Vec<KiCadFabStackupLayer> = stackup
        .children()
        .iter()
        .filter(|item| item.head() == Some("layer"))
        .map(|layer| KiCadFabStackupLayer {
            name: layer
                .children()
                .get(1)
                .and_then(Expr::atom)
                .unwrap_or("")
                .trim_matches('"')
                .to_string(),
            kind: form_atom(layer, "type", 1)
                .unwrap_or("")
                .trim_matches('"')
                .to_string(),
            thickness_mm: number(layer, "thickness"),
            material: form_atom(layer, "material", 1)
                .map(|value| value.trim_matches('"').to_string()),
            epsilon_r: number(layer, "epsilon_r"),
            loss_tangent: number(layer, "loss_tangent"),
        })
        .collect();
    if layers.is_empty() {
        return None;
    }
    Some(KiCadFabStackup {
        thickness_mm: pcb
            .child("general")
            .and_then(|general| number(general, "thickness")),
        copper_finish: form_atom(stackup, "copper_finish", 1)
            .map(|value| value.trim_matches('"').to_string()),
        layers,
    })
}

/// Reference prefixes of polarised parts: diodes, LEDs, transistors,
/// ICs.
const POLARISED_PREFIXES: [&str; 8] = ["D", "LED", "LD", "CR", "ZD", "Q", "U", "IC"];

/// Whether a part is polarised (its rotation matters to the assembler):
/// by its reference's prefix (`POLARISED_PREFIXES`) or its footprint's
/// name (a diode's or LED's library footprint, a polarised capacitor's:
/// `CP_`, electrolytic, tantalum).
pub(crate) fn polarised(reference: &str, footprint: &str) -> bool {
    let prefix = reference.trim_end_matches(|c: char| c.is_ascii_digit() || c == '_');
    let name = footprint.rsplit(':').next().unwrap_or(footprint);
    let lower = name.to_ascii_lowercase();
    POLARISED_PREFIXES.contains(&prefix)
        || name.starts_with("D_")
        || name.starts_with("LED_")
        || name.starts_with("CP_")
        || lower.contains("elec")
        || lower.contains("tantal")
        || lower.contains("polari")
}

/// The polarised parts (`polarised`) of `pcb` among `placed` (the
/// position file's references), in natural order.
fn polarised_parts(pcb: &Expr, placed: &BTreeSet<String>) -> Vec<String> {
    let mut parts: Vec<String> = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .filter_map(|footprint| {
            let reference = footprint_reference(footprint)?;
            let name = footprint
                .children()
                .get(1)
                .and_then(Expr::atom)
                .unwrap_or("");
            (placed.contains(&reference) && polarised(&reference, name.trim_matches('"')))
                .then_some(reference)
        })
        .collect();
    parts.sort_by(|a, b| crate::canonical_order::natural_cmp(a, b));
    parts.dedup();
    parts
}

/// The references of kicad-cli's position file (its `Ref` column).
fn position_references(text: &str) -> BTreeSet<String> {
    text.lines()
        .skip(1)
        .filter_map(|line| csv_columns(line).into_iter().next())
        .filter(|reference| !reference.is_empty())
        .collect()
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
    // Inside the process's reservation in the memory ledger
    // (`ledger::kicad_check`): an export loads the board.
    crate::ledger::kicad_check(|| kicad_cli_unreserved(arguments, what))
}

fn kicad_cli_unreserved(arguments: &[&std::ffi::OsStr], what: &str) -> Result<(), String> {
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
fn finding_lines(
    report: &serde_json::Value,
    limit: usize,
    input: Option<&crate::verdict::KiCadInputItems>,
) -> Vec<String> {
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
                && crate::verdict::unreferenced_apart(&drc_finding(finding), input).is_none()
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
/// own locating holes), which no layout changes, and the board's own
/// findings that are not electrical (`input_own`, the board's items in
/// `input`: a connector's pads at the edge), which do not block, with the
/// custom rules for `dru` that accept them.
fn release_gate(
    tally: &crate::verdict::DrcTally,
    dru: &str,
    input: Option<&crate::verdict::KiCadInputItems>,
    alone: bool,
) -> KiCadFabDrc {
    use crate::verdict::{Apart, unreferenced_apart};
    let apart = |which: Apart| -> Vec<&crate::verdict::DrcFinding> {
        tally
            .findings
            .iter()
            .filter(|finding| unreferenced_apart(finding, input) == Some(which))
            .collect()
    };
    let own = apart(Apart::Footprint);
    let board = apart(Apart::Input);
    let mut errors = tally.errors.clone();
    let mut footprint_own = BTreeMap::new();
    let mut input_own = BTreeMap::new();
    for (findings, kinds) in [(&own, &mut footprint_own), (&board, &mut input_own)] {
        for finding in findings.iter() {
            *kinds.entry(finding.kind.clone()).or_insert(0) += 1;
            if let Some(count) = errors.get_mut(&finding.kind) {
                *count -= 1;
                if *count == 0 {
                    errors.remove(&finding.kind);
                }
            }
        }
    }
    KiCadFabDrc {
        clean: errors.is_empty() && tally.unconnected == 0,
        errors,
        footprint_note: footprint_findings_note(&own, dru),
        footprint_own,
        input_note: crate::verdict::input_findings_note(&board, dru, alone),
        input_own,
        unconnected_items: tally.unconnected,
        warnings: tally.warnings,
        cosmetic: tally.cosmetic,
        findings_allowed: false,
        report: "drc.json".into(),
    }
}

/// The layers to plot: `--layers` (each one the board defines), else the
/// profile's.
fn fab_layers(
    pcb: &Expr,
    defined: &[String],
    options: &KiCadFabOptions,
) -> Result<Vec<String>, String> {
    Ok(match &options.layers {
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
        None => release_layers(pcb, options.profile)?
            .into_iter()
            .filter(|layer| defined.is_empty() || defined.contains(layer))
            .collect(),
    })
}

/// The manifest before any check: the board's identity, stackup and
/// controlled impedance; nothing released yet.
fn starting_manifest(
    directory: &Path,
    board_id: &str,
    board: &Path,
    pcb: &Expr,
    layers: Vec<String>,
    options: &KiCadFabOptions,
) -> KiCadFabRelease {
    let program = crate::native_report::kicad_cli();
    let program = program.as_os_str();
    KiCadFabRelease {
        schema_version: 1,
        board_id: board_id.into(),
        profile: options.profile,
        kicad_version: crate::drc_cache::tool_version(program),
        released: false,
        drc: KiCadFabDrc::default(),
        erc: None,
        layers,
        bom_source: None,
        lcsc_field: None,
        without_lcsc: Vec::new(),
        without_lcsc_lines: 0,
        zip: None,
        files: Vec::new(),
        skipped: Vec::new(),
        findings: Vec::new(),
        stackup: board_stackup(pcb),
        controlled_impedance: crate::resolve_project_rules(
            &directory.join(format!("{board_id}.kicad_pro")),
            board,
        )
        .ok()
        .and_then(|config| config.project_rules)
        .and_then(|report| {
            let nets: Vec<String> = report.net_classes.keys().cloned().collect();
            crate::class_view::controlled_impedance(&report, &nets)
        }),
        polarised_parts: Vec::new(),
    }
}

/// The input's items: with --input, the parts still where its board has
/// them and its zones and graphics unchanged; else the board judged on
/// its own, every footprint, zone and board graphic its own (tracks and
/// vias never are).
fn release_input(
    options: &KiCadFabOptions,
    board_id: &str,
    pcb: &Expr,
) -> Result<crate::verdict::KiCadInputItems, String> {
    Ok(match &options.input {
        None => crate::verdict::input_items_of(pcb, pcb),
        Some(project) => {
            let path = project.join(format!("{board_id}.kicad_pcb"));
            let text = fs::read_to_string(&path).map_err(|error| {
                input_error(format!("failed to read {}: {error}", path.display()))
            })?;
            crate::verdict::input_items_of(&parse(&text).map_err(input_error)?, pcb)
        }
    })
}

/// The schematic's ERC, written as `erc.json` in `output`, summarised.
fn schematic_erc(output: &Path, schematic: &Path) -> Result<KiCadFabErc, String> {
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
        schematic,
    )?;
    let erc: serde_json::Value = read_json(&erc_path)?;
    let issues = crate::native_report::erc_design_issues(&erc);
    let all = erc["sheets"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|sheet| sheet["violations"].as_array().into_iter().flatten())
        .count();
    Ok(KiCadFabErc {
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
    })
}

/// Gerbers and drill files into `output/gerbers`, and the zip of them.
fn export_gerbers(
    output: &Path,
    board: &Path,
    board_id: &str,
    layers: &[String],
    manifest: &mut KiCadFabRelease,
) -> Result<(), String> {
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
    Ok(())
}

/// Placement: kicad-cli's position file, both sides, DNP parts out (and
/// JLCPCB's columns of it); notes the polarised parts. Returns the
/// `assembly` directory.
fn export_placement(
    output: &Path,
    board: &Path,
    board_id: &str,
    pcb: &Expr,
    profile: KiCadFabProfile,
    manifest: &mut KiCadFabRelease,
) -> Result<PathBuf, String> {
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
    manifest.polarised_parts = polarised_parts(
        pcb,
        &position_references(&fs::read_to_string(&positions).unwrap_or_default()),
    );
    if profile == KiCadFabProfile::Jlcpcb {
        let text = fs::read_to_string(&positions).map_err(|error| error.to_string())?;
        let cpl = assembly.join(format!("{board_id}-cpl-jlcpcb.csv"));
        fs::write(&cpl, jlcpcb_placements(&text))
            .map_err(|error| format!("failed to write {}: {error}", cpl.display()))?;
    }
    Ok(assembly)
}

/// The bill of materials into `assembly`: kicad-cli's of the schematic,
/// grouped by value, footprint and part number; else the board's
/// footprints.
fn export_bom(
    directory: &Path,
    assembly: &Path,
    board_id: &str,
    pcb: &Expr,
    schematic: &Path,
    profile: KiCadFabProfile,
    manifest: &mut KiCadFabRelease,
) -> Result<(), String> {
    let board_bom = crate::fab_export::board_bom(pcb);
    manifest.without_lcsc = board_bom.without_lcsc.clone();
    manifest.without_lcsc_lines = board_bom.lines_without_lcsc;
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
        if profile == KiCadFabProfile::Jlcpcb {
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
    Ok(())
}

/// Netlists: the board's IPC-D-356 (the fab's electrical test), the
/// schematic's KiCad netlist.
fn export_netlists(
    output: &Path,
    board: &Path,
    board_id: &str,
    schematic: &Path,
) -> Result<(), String> {
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
    Ok(())
}

/// The 3D model: STEP with the parts' models where KiCad finds them; a
/// failure is noted, not fatal.
fn export_step(
    output: &Path,
    board: &Path,
    board_id: &str,
    options: &KiCadFabOptions,
    manifest: &mut KiCadFabRelease,
) {
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
    let layers = fab_layers(&pcb, &defined, options)?;
    let mut manifest =
        starting_manifest(directory, board_id, &board, &pcb, layers.clone(), options);
    // The checks: the board's DRC as drc-kicad-board runs it (cached,
    // without the global libraries, zones refilled), the schematic's ERC.
    let drc_path = output.join("drc.json");
    let drc = kicad_drc_report(directory, board_id, &drc_path, true)?;
    let dru = directory.join(format!("{board_id}.kicad_dru"));
    let input = release_input(options, board_id, &pcb)?;
    manifest.drc = release_gate(
        &tally_drc(&drc),
        &dru.display().to_string(),
        Some(&input),
        options.input.is_none(),
    );
    let schematic = directory.join(format!("{board_id}.kicad_sch"));
    if schematic.is_file() {
        manifest.erc = Some(schematic_erc(output, &schematic)?);
    } else {
        manifest
            .skipped
            .push("ERC, schematic netlist: no schematic beside the board".into());
    }
    manifest.findings = finding_lines(&drc, 20, Some(&input));
    if !manifest.drc.clean && !options.allow_findings {
        write_manifest(output, &mut manifest)?;
        return Ok(manifest);
    }
    manifest.drc.findings_allowed = !manifest.drc.clean;
    manifest.released = true;

    export_gerbers(output, &board, board_id, &layers, &mut manifest)?;
    let assembly = export_placement(
        output,
        &board,
        board_id,
        &pcb,
        options.profile,
        &mut manifest,
    )?;
    export_bom(
        directory,
        &assembly,
        board_id,
        &pcb,
        &schematic,
        options.profile,
        &mut manifest,
    )?;
    export_netlists(output, &board, board_id, &schematic)?;
    export_step(output, &board, board_id, options, &mut manifest);
    write_manifest(output, &mut manifest)?;
    Ok(manifest)
}

/// What fab-kicad-board's verdict says (`fab_verdict_lines`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KiCadFabVerdictLines {
    /// The verdict's reason: released (with findings) or refused, with
    /// KiCad's counts.
    pub reason: String,
    /// The release's own actions first, then the input's own observations
    /// tagged (`with_input_observations`).
    pub next_actions: Vec<String>,
    /// The input's own observations in full: the footprints' own findings'
    /// note and the board's own findings' note.
    pub input_observations: Vec<String>,
}

/// fab-kicad-board's verdict lines from its manifest, in the order of a
/// layout's verdict: the release's own actions first (on a refusal what to
/// fix, with `--input` the findings between parts the layout kept where
/// the input has them (`kept`, `kept_count` of them:
/// `kept_parts_actions`) in their place after it, then the findings that
/// block; on a release the findings it went ahead with, the parts without
/// an LCSC number, the rotations to check), the footprints' own and the
/// board's own findings that do not block last, tagged. Until 2026-11-04
/// those notes came first, ahead of the refusal's findings.
pub fn fab_verdict_lines(
    manifest: &KiCadFabRelease,
    kept: Vec<String>,
    kept_count: usize,
    input_given: bool,
) -> KiCadFabVerdictLines {
    let drc = &manifest.drc;
    let errors: usize = drc.errors.values().sum();
    let counts = format!(
        "KiCad's DRC: {errors} errors{}, {} unconnected items, {} warnings, {} cosmetic",
        if drc.errors.is_empty() {
            String::new()
        } else {
            format!(
                " ({})",
                drc.errors
                    .iter()
                    .map(|(kind, count)| format!("{kind} {count}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        },
        drc.unconnected_items,
        drc.warnings,
        drc.cosmetic
    ) + &if drc.footprint_own.is_empty() {
        String::new()
    } else {
        format!(
            ", {} within one footprint each (not blocking)",
            drc.footprint_own.values().sum::<usize>()
        )
    } + &if drc.input_own.is_empty() {
        String::new()
    } else {
        format!(
            ", {} the board's own, not electrical (not blocking)",
            drc.input_own.values().sum::<usize>()
        )
    };
    let mut actions = Vec::new();
    let reason = if manifest.released {
        if drc.findings_allowed {
            actions.push(format!(
                "the release went ahead with KiCad's findings (--allow-findings): {counts}; make sure the fab accepts them"
            ));
            actions.extend(manifest.findings.iter().cloned());
        }
        actions.extend(lcsc_action(manifest));
        actions.extend(manifest.controlled_impedance.as_ref().map(|impedance| {
            if manifest.profile == KiCadFabProfile::Jlcpcb {
                format!(
                    "{}; at JLCPCB, impedance control on the order with the stackup of theirs whose dielectrics match these",
                    impedance.action
                )
            } else {
                impedance.action.clone()
            }
        }));
        actions.extend(manifest.stackup.as_ref().and_then(copper_weight_action));
        actions.push(rotation_action(&manifest.polarised_parts));
        format!(
            "released{}: {} files, {}; {counts}",
            if drc.findings_allowed {
                " with findings (--allow-findings)"
            } else {
                ""
            },
            manifest.files.len(),
            manifest.zip.as_deref().unwrap_or("no zip")
        )
    } else {
        if kept.is_empty() && !input_given {
            // Judged on its own the board's parts are all its own: which of
            // them a layout or a route kept where its input has them (the
            // input's to move, not the run's) only --input tells.
            actions.push(
                "fix the findings (route the open connections, move what is too close) and release again; --allow-findings exports anyway; for a board pcb-maker laid out or routed, --input <the project it came from> names the findings between parts kept where that project has them, which are the input's to move, not the run's (and the layout's or route's --reference <the designer's project> counts the designer's own as pre-existing)"
                    .into(),
            );
        } else if kept.is_empty() {
            actions.push(
                "fix the findings (route the open connections, move what is too close) and release again; --allow-findings exports anyway"
                    .into(),
            );
        } else {
            actions.push(format!(
                "{kept_count} of the errors lie between parts the layout kept where the input has them (below): they are the input's, not the layout's to move; fix the others (route the open connections, move what the layout put too close) and release again; --allow-findings exports anyway"
            ));
            actions.extend(kept);
        }
        actions.extend(manifest.findings.iter().cloned());
        format!("refused: {counts}; nothing was exported")
    };
    let observations: Vec<String> = drc
        .footprint_note
        .iter()
        .chain(drc.input_note.iter())
        .cloned()
        .collect();
    KiCadFabVerdictLines {
        reason,
        next_actions: crate::verdict::with_input_observations(actions, &observations),
        input_observations: observations,
    }
}

/// The release's line on the rotations to check: the polarised parts
/// on the placement file, the first 20 named.
fn rotation_action(parts: &[String]) -> String {
    const SHOWN: usize = 20;
    if parts.is_empty() {
        return "check the rotations of polarised parts in the fab's placement preview: library orientations differ between KiCad and assemblers (none found by reference or footprint: diodes, LEDs, transistors, ICs, polarised capacitors)".into();
    }
    format!(
        "check the rotations of the {} polarised parts in the fab's placement preview (library orientations differ between KiCad and assemblers; fab-manifest.json's polarised_parts): {}{}",
        parts.len(),
        parts[..parts.len().min(SHOWN)].join(", "),
        if parts.len() > SHOWN {
            format!(" and {} more", parts.len() - SHOWN)
        } else {
            String::new()
        }
    )
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
        let gate = release_gate(
            &tally_drc(&report(vec![own.clone()])),
            "p/b.kicad_dru",
            None,
            true,
        );
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
        assert!(finding_lines(&report(vec![own.clone()]), 20, None).is_empty());
        // Between two parts, it blocks.
        let mut apart = own;
        apart["items"][1]["description"] = "NPTH pad of H1".into();
        let gate = release_gate(
            &tally_drc(&report(vec![apart.clone()])),
            "p/b.kicad_dru",
            None,
            true,
        );
        assert!(!gate.clean && gate.footprint_note.is_none());
        assert_eq!(finding_lines(&report(vec![apart]), 20, None).len(), 1);
    }

    #[test]
    fn the_boards_own_findings_block_only_when_electrical() {
        let board = r#"(kicad_pcb (version 20240108)
            (footprint "lib:A" (layer "F.Cu") (uuid "fa") (at 10 10)
              (property "Reference" "J1") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (uuid "pa1")))
            (footprint "lib:B" (layer "F.Cu") (uuid "fb") (at 20 10)
              (property "Reference" "J2") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (uuid "pb1")))
            (gr_line (start 0 0) (end 30 0) (layer "Edge.Cuts") (uuid "e1"))
            (segment (start 1 1) (end 2 2) (width 0.2) (layer "F.Cu") (net 1) (uuid "s1")))"#;
        let input = crate::verdict::board_items(board).unwrap();
        let finding = |kind: &str, items: &[(&str, &str)]| {
            serde_json::json!({"type": kind, "severity": "error", "description": kind,
                "items": items.iter().map(|(description, uuid)| serde_json::json!({"description": description, "uuid": uuid, "pos": {"x": 0.0, "y": 0.0}})).collect::<Vec<_>>()})
        };
        let report = |violations: Vec<serde_json::Value>| serde_json::json!({"violations": violations, "unconnected_items": []});
        let edge = finding(
            "copper_edge_clearance",
            &[
                ("Segment on Edge.Cuts", "e1"),
                ("Pad 1 [GND] of J1 on F.Cu", "pa1"),
            ],
        );
        let gate = release_gate(
            &tally_drc(&report(vec![edge.clone()])),
            "p/b.kicad_dru",
            Some(&input),
            true,
        );
        assert!(gate.clean, "{:?}", gate.errors);
        assert_eq!(
            gate.input_own,
            BTreeMap::from([("copper_edge_clearance".to_string(), 1)])
        );
        let note = gate.input_note.unwrap();
        assert!(note.contains("A.memberOfFootprint('J1')"));
        // Judged on its own, the note does not claim an input kept the
        // parts (trial 4), and names --input.
        assert!(note.contains("judged on its own (no --input)"), "{note}");
        assert!(!note.contains("kept where the input has it"), "{note}");
        assert!(finding_lines(&report(vec![edge.clone()]), 20, Some(&input)).is_empty());
        // The designer's own short, or a track at the edge, blocks.
        for blocking in [
            finding(
                "shorting_items",
                &[
                    ("Pad 1 [GND] of J1 on F.Cu", "pa1"),
                    ("Pad 1 [VCC] of J2 on F.Cu", "pb1"),
                ],
            ),
            finding(
                "clearance",
                &[
                    ("Pad 1 [GND] of J1 on F.Cu", "pa1"),
                    ("Pad 1 [VCC] of J2 on F.Cu", "pb1"),
                ],
            ),
            finding(
                "copper_edge_clearance",
                &[
                    ("Segment on Edge.Cuts", "e1"),
                    ("Track [VCC] on F.Cu, length 1.0000 mm", "s1"),
                ],
            ),
        ] {
            let gate = release_gate(
                &tally_drc(&report(vec![edge.clone(), blocking.clone()])),
                "p/b.kicad_dru",
                Some(&input),
                true,
            );
            assert!(
                !gate.clean && gate.errors.values().sum::<usize>() == 1,
                "{blocking}"
            );
            assert_eq!(
                finding_lines(&report(vec![blocking.clone()]), 20, Some(&input)).len(),
                1
            );
        }
    }

    #[test]
    fn the_releases_actions_come_before_the_boards_own_findings() {
        // The release's own actions first, the notes that do not block
        // last and tagged; with --input the kept parts' findings keep
        // their place after the refusal's first line. Until 2026-11-04 the
        // footprints' and the board's own notes led the list.
        let mut manifest = KiCadFabRelease {
            schema_version: 1,
            board_id: "b".into(),
            profile: KiCadFabProfile::Kicad,
            kicad_version: None,
            released: false,
            drc: KiCadFabDrc {
                errors: BTreeMap::from([("clearance".to_string(), 2)]),
                footprint_own: BTreeMap::from([("hole_clearance".to_string(), 8)]),
                footprint_note: Some("8 DRC findings lie within one footprint each (J1: hole_clearance 8)".into()),
                input_own: BTreeMap::from([("copper_edge_clearance".to_string(), 8)]),
                input_note: Some("8 DRC findings are the input's (copper_edge_clearance 8)".into()),
                unconnected_items: 1,
                ..Default::default()
            },
            erc: None,
            layers: Vec::new(),
            bom_source: None,
            lcsc_field: None,
            without_lcsc: Vec::new(),
            without_lcsc_lines: 0,
            zip: None,
            files: Vec::new(),
            skipped: Vec::new(),
            findings: vec!["clearance: Pad 1 [A] of R31 on F.Cu at (1.00, 2.00); Pad 2 [B] of J1 on F.Cu at (1.10, 2.00)".into()],
            stackup: None,
            controlled_impedance: None,
            polarised_parts: Vec::new(),
        };
        let refused = fab_verdict_lines(&manifest, Vec::new(), 0, false);
        assert_eq!(
            refused.reason,
            "refused: KiCad's DRC: 2 errors (clearance 2), 1 unconnected items, 0 warnings, 0 cosmetic, 8 within one footprint each (not blocking), 8 the board's own, not electrical (not blocking); nothing was exported"
        );
        let actions = &refused.next_actions;
        assert_eq!(actions.len(), 4, "{actions:#?}");
        assert!(actions[0].starts_with("fix the findings"), "{actions:#?}");
        // Judged on its own: --input names the parts a run kept.
        assert!(
            actions[0].contains(
                "--input <the project it came from> names the findings between parts kept"
            ),
            "{actions:#?}"
        );
        let with_input = fab_verdict_lines(&manifest, Vec::new(), 0, true);
        assert!(!with_input.next_actions[0].contains("--input"));
        assert!(
            actions[1].starts_with("clearance: Pad 1 [A] of R31"),
            "{actions:#?}"
        );
        assert_eq!(
            actions[2],
            "nothing to do (the input's): 8 DRC findings lie within one footprint each (J1: hole_clearance 8)"
        );
        assert!(
            actions[3].starts_with("nothing to do (the input's): 8 DRC findings are the input's")
        );
        assert_eq!(refused.input_observations.len(), 2);
        // --input: the kept parts' findings after the first line.
        let kept = fab_verdict_lines(
            &manifest,
            vec!["clearance 2 (...): every finding lies between parts kept where the input has them (J1, R31: unchanged from the input)".into()],
            2,
            true,
        );
        assert!(
            kept.next_actions[0].starts_with("2 of the errors lie between parts the layout kept")
        );
        assert!(
            kept.next_actions[1]
                .starts_with("clearance 2 (...): every finding lies between parts kept")
        );
        assert!(kept.next_actions[2].starts_with("clearance: Pad 1"));
        assert!(kept.next_actions[3].starts_with(crate::verdict::INPUT_TAG));
        // Released: the release's lines, then the notes.
        manifest.released = true;
        manifest.drc.errors.clear();
        manifest.drc.unconnected_items = 0;
        let released = fab_verdict_lines(&manifest, Vec::new(), 0, false);
        assert!(
            released
                .reason
                .starts_with("released: 0 files, no zip; KiCad's DRC: 0 errors")
        );
        assert!(released.next_actions[0].starts_with("check the rotations"));
        assert!(
            released.next_actions[1..]
                .iter()
                .all(|action| action.starts_with(crate::verdict::INPUT_TAG))
        );
    }

    #[test]
    fn the_manifest_has_the_stackup_the_impedance_classes_and_the_polarised_parts() {
        // Trial 7: Sisu_SSE-9's release carried its stackup only in the
        // job file, said nothing of its impedance classes, and every
        // release's rotation line named no part.
        let (directory, _) = crate::class_view::tests::impedance_project("fab");
        let text = fs::read_to_string(directory.join("board.kicad_pcb")).unwrap();
        let pcb = parse(&text).unwrap();
        let stackup = board_stackup(&pcb).unwrap();
        assert_eq!(
            (stackup.thickness_mm, stackup.copper_finish.as_deref()),
            (Some(1.6), Some("ENIG"))
        );
        let prepreg = &stackup.layers[2];
        assert_eq!(
            (
                prepreg.name.as_str(),
                prepreg.kind.as_str(),
                prepreg.thickness_mm,
                prepreg.material.as_deref(),
                prepreg.epsilon_r
            ),
            (
                "dielectric 1",
                "prepreg",
                Some(0.0918),
                Some("3313"),
                Some(4.1)
            )
        );
        assert_eq!(stackup.layers[1].kind, "copper");
        assert_eq!(board_stackup(&parse("(kicad_pcb (setup))").unwrap()), None);
        // The polarised parts on the placement file.
        for (reference, footprint, expected) in [
            ("D3", "Diode_SMD:D_SOD-123", true),
            ("LED1", "LED_SMD:LED_0603_1608Metric", true),
            ("U7", "Package_DFN_QFN:QFN-80", true),
            ("Q2", "Package_TO_SOT_SMD:SOT-23", true),
            ("C9", "Capacitor_Tantalum_SMD:CP_EIA-3216-18_Kemet-A", true),
            ("C10", "Capacitor_SMD:C_0402_1005Metric", false),
            ("R1", "Resistor_SMD:R_0402_1005Metric", false),
            ("J1", "Connector_USB:USB_C_Receptacle", false),
            ("TP1", "TestPoint:TestPoint_Pad_D1.0mm", false),
        ] {
            assert_eq!(polarised(reference, footprint), expected, "{reference}");
        }
        let board = r#"(kicad_pcb
            (footprint "Diode_SMD:D_SOD-123" (property "Reference" "D10") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu")))
            (footprint "Diode_SMD:D_SOD-123" (property "Reference" "D2") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu")))
            (footprint "Diode_SMD:D_SOD-123" (property "Reference" "D3") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu")))
            (footprint "Resistor_SMD:R_0402" (property "Reference" "R1") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu"))))"#;
        let positions = "Ref,Val,Package,PosX,PosY,Rot,Side\nD10,x,y,0,0,0,top\nD2,x,y,0,0,0,top\nR1,x,y,0,0,0,top\n";
        // D3 is not on the placement file (DNP): not the assembler's.
        let parts = polarised_parts(&parse(board).unwrap(), &position_references(positions));
        assert_eq!(parts, ["D2", "D10"]);
        let line = rotation_action(&parts);
        assert_eq!(
            line,
            "check the rotations of the 2 polarised parts in the fab's placement preview (library orientations differ between KiCad and assemblers; fab-manifest.json's polarised_parts): D2, D10"
        );
        let many: Vec<String> = (1..=25).map(|n| format!("U{n}")).collect();
        assert!(rotation_action(&many).ends_with("U19, U20 and 5 more"));
        // The release's lines: the impedance order, then the rotations.
        let report = crate::resolve_project_rules(
            &directory.join("board.kicad_pro"),
            &directory.join("board.kicad_pcb"),
        )
        .unwrap()
        .project_rules
        .unwrap();
        let nets: Vec<String> = report.net_classes.keys().cloned().collect();
        let mut manifest = KiCadFabRelease {
            schema_version: 1,
            board_id: "board".into(),
            profile: KiCadFabProfile::Jlcpcb,
            kicad_version: None,
            released: true,
            drc: KiCadFabDrc::default(),
            erc: None,
            layers: Vec::new(),
            bom_source: Some("board".into()),
            lcsc_field: None,
            without_lcsc: Vec::new(),
            without_lcsc_lines: 0,
            zip: None,
            files: Vec::new(),
            skipped: Vec::new(),
            findings: Vec::new(),
            stackup: Some(stackup),
            controlled_impedance: crate::class_view::controlled_impedance(&report, &nets),
            polarised_parts: parts,
        };
        let lines = fab_verdict_lines(&manifest, Vec::new(), 0, false);
        assert!(
            lines.next_actions[0].starts_with(
                "order impedance control on this stackup (fab-manifest.json's stackup, the job file's) for the classes RF, USB"
            ),
            "{:#?}",
            lines.next_actions
        );
        assert!(lines.next_actions[1].starts_with("check the rotations of the 2 polarised parts"));
        let json = serde_json::to_value(&manifest).unwrap();
        assert_eq!(json["stackup"]["layers"][2]["material"], "3313");
        assert_eq!(json["controlled_impedance"]["classes"][1]["class"], "USB");
        assert_eq!(
            json["controlled_impedance"]["rules"][0]["layers"],
            serde_json::json!(["In1.Cu"])
        );
        assert_eq!(json["polarised_parts"], serde_json::json!(["D2", "D10"]));
        // A board without them: no entries.
        manifest.stackup = None;
        manifest.controlled_impedance = None;
        let json = serde_json::to_value(&manifest).unwrap();
        assert!(json.get("stackup").is_none() && json.get("controlled_impedance").is_none());
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn parts_without_an_lcsc_number_are_told_where_it_goes_by_the_boms_source() {
        // Trial 7's MIDAS-MK2 layout: no schematic beside the result, 207
        // parts on 78 lines; the action said "an LCSC field on the symbol,
        // then Update PCB from Schematic", which that release cannot do.
        let mut manifest = KiCadFabRelease {
            schema_version: 1,
            board_id: "b".into(),
            profile: KiCadFabProfile::Jlcpcb,
            kicad_version: None,
            released: true,
            drc: KiCadFabDrc::default(),
            erc: None,
            layers: Vec::new(),
            bom_source: Some("board".into()),
            lcsc_field: None,
            without_lcsc: (1..=12).map(|n| format!("R{n}")).collect(),
            without_lcsc_lines: 3,
            zip: None,
            files: Vec::new(),
            skipped: Vec::new(),
            findings: Vec::new(),
            stackup: None,
            controlled_impedance: None,
            polarised_parts: Vec::new(),
        };
        let action = lcsc_action(&manifest).expect("parts without a number");
        assert!(
            action.starts_with(
                "12 parts on 3 BOM lines (value and footprint: one number each) have no LCSC part number"
            ),
            "{action}"
        );
        assert!(
            action.contains("a property \"LCSC\" or \"LCSC Part\""),
            "{action}"
        );
        assert!(action.contains("R1, R2,"), "{action}");
        assert!(action.ends_with(", ... (fab-manifest.json's without_lcsc lists them)"));
        // From a schematic: its field, on the symbols.
        manifest.bom_source = Some("schematic".into());
        manifest.lcsc_field = Some("LCSC Part".into());
        let action = lcsc_action(&manifest).unwrap();
        assert!(
            action
                .contains("(a field \"LCSC Part\" on the symbols, then Update PCB from Schematic"),
            "{action}"
        );
        manifest.without_lcsc.clear();
        assert!(lcsc_action(&manifest).is_none());
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

    /// 2 oz outer copper is named for the order (trial 8: the
    /// 4in1-mini's release said nothing of it).
    #[test]
    fn heavy_copper_is_named_for_the_order() {
        let layer = |name: &str, kind: &str, thickness: f64| KiCadFabStackupLayer {
            name: name.into(),
            kind: kind.into(),
            thickness_mm: Some(thickness),
            ..Default::default()
        };
        let mut stackup = KiCadFabStackup {
            thickness_mm: Some(1.6),
            copper_finish: None,
            layers: vec![
                layer("F.Cu", "copper", 0.07),
                layer("dielectric 1", "prepreg", 0.2),
                layer("In1.Cu", "copper", 0.035),
                layer("B.Cu", "copper", 0.07),
            ],
        };
        assert_eq!(
            copper_weight_action(&stackup).unwrap(),
            "the stackup has copper heavier than 1 oz: F.Cu, B.Cu (0.07 mm, 2 oz); a fab makes 1 oz unless the order says otherwise, so order that copper weight (fab-manifest.json's stackup)"
        );
        stackup.layers = vec![
            layer("F.Cu", "copper", 0.035),
            layer("B.Cu", "copper", 0.035),
        ];
        assert_eq!(copper_weight_action(&stackup), None);
    }

    #[test]
    fn layers_the_board_does_not_define_are_refused() {
        let pcb = parse(
            "(kicad_pcb (layers (0 \"F.Cu\" signal) (31 \"B.Cu\" signal) (44 \"Edge.Cuts\" user)))",
        )
        .unwrap();
        let defined = defined_layers(&pcb);
        let options = KiCadFabOptions {
            layers: Some(vec!["F.Cu".into(), "In1.Cu".into()]),
            ..Default::default()
        };
        let error = fab_layers(&pcb, &defined, &options).unwrap_err();
        assert!(error.contains("\"In1.Cu\""), "{error}");
        let options = KiCadFabOptions {
            layers: Some(vec!["F.Cu".into()]),
            ..Default::default()
        };
        assert_eq!(fab_layers(&pcb, &defined, &options).unwrap(), ["F.Cu"]);
    }
}
