// Copyright (C) 2026 Toit contributors.

//! KiCad's own verification of a board directory (ERC, DRC, schematic
//! parity) and its cache.

use super::*;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct VerificationReport {
    pub board_id: String,
    pub complete: bool,
    pub erc_violations: usize,
    pub drc_design_violations: usize,
    pub schematic_parity_issues: usize,
    pub selected_net_unconnected_items: usize,
    pub intentional_no_connect_groups: usize,
    pub library_metadata_warnings: usize,
    /// ERC's library warnings by type (`is_erc_library_warning`), not in
    /// `erc_violations`.
    #[serde(default)]
    pub erc_library_warnings: BTreeMap<String, usize>,
}

pub const KICAD_VERIFICATION_CACHE_ENV: &str = "PCB_MAKER_KICAD_VERIFICATION_CACHE";

pub const KICAD_VERIFICATION_CACHE_EVENTS_ENV: &str = "PCB_MAKER_KICAD_VERIFICATION_CACHE_EVENTS";

const KICAD_VERIFICATION_CACHE_SCHEMA_VERSION: u32 = 1;

const KICAD_VERIFICATION_POLICY: &str = "verify-materialized-rung-v2:erc-without-library-warnings:drc-all:schematic-parity:intentional-no-connect-groups";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadVerificationCacheStatus {
    Hit,
    Miss,
    Bypassed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadVerificationCacheTrace {
    pub schema_version: u32,
    pub status: KiCadVerificationCacheStatus,
    pub key: Option<String>,
    pub input_sha256: Option<String>,
    pub kicad_cli_version: String,
    pub verifier_sha256: String,
    pub policy: String,
    pub refill_zones: bool,
    pub cache_root: PathBuf,
    pub cache_entry: Option<PathBuf>,
    pub elapsed_micros: u64,
    pub detail: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct KiCadVerificationCacheEntry {
    schema_version: u32,
    key: String,
    input_sha256: String,
    kicad_cli_version: String,
    verifier_sha256: String,
    policy: String,
    refill_zones: bool,
    erc_sha256: String,
    drc_sha256: String,
    report: VerificationReport,
}

pub fn verify_materialized_rung(
    directory: &Path,
    board_id: &str,
) -> Result<VerificationReport, String> {
    verification_preview::capture(directory, board_id, || {
        remove_if_present(&directory.join("verification.json"))?;
        let pcb_path = directory.join(format!("{board_id}.kicad_pcb"));
        let pcb_source = fs::read_to_string(&pcb_path)
            .map_err(|error| format!("failed to read {}: {error}", pcb_path.display()))?;
        let pcb = parse(&pcb_source)?;
        verify_materialized_rung_with_options(directory, board_id, count_head(&pcb, "zone") > 0)
    })
}

fn verify_materialized_rung_with_options(
    directory: &Path,
    board_id: &str,
    refill_zones: bool,
) -> Result<VerificationReport, String> {
    if !directory.join(format!("{board_id}.kicad_sch")).exists() {
        return verify_board_only(directory, board_id, refill_zones);
    }
    if let Some(cache_root) = env::var_os(KICAD_VERIFICATION_CACHE_ENV) {
        verify_materialized_rung_cached(directory, board_id, refill_zones, Path::new(&cache_root))
    } else {
        verify_materialized_rung_uncached(directory, board_id, refill_zones)
    }
}

fn verify_materialized_rung_uncached(
    directory: &Path,
    board_id: &str,
    refill_zones: bool,
) -> Result<VerificationReport, String> {
    remove_if_present(&directory.join("verification.json"))?;
    let schematic = directory.join(format!("{board_id}.kicad_sch"));
    let pcb = directory.join(format!("{board_id}.kicad_pcb"));
    let erc_path = directory.join("erc.json");
    let drc_path = directory.join("drc.json");
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
    let mut drc_arguments = vec![
        "pcb",
        "drc",
        "--format",
        "json",
        "--severity-all",
        "--schematic-parity",
    ];
    if refill_zones {
        drc_arguments.push("--refill-zones");
    }
    drc_arguments.push("--output");
    run_kicad_report(&drc_arguments, &drc_path, &pcb)?;

    let erc: serde_json::Value = read_json(&erc_path)?;
    let drc: serde_json::Value = read_json(&drc_path)?;
    write_verification_report(directory, board_id, &erc, &drc)
}

/// A project without a schematic (a bare board, as in board-only benchmark
/// sets): DRC without schematic parity, and an empty ERC report in its place.
fn verify_board_only(
    directory: &Path,
    board_id: &str,
    refill_zones: bool,
) -> Result<VerificationReport, String> {
    remove_if_present(&directory.join("verification.json"))?;
    let pcb = directory.join(format!("{board_id}.kicad_pcb"));
    let drc_path = directory.join("drc.json");
    let mut drc_arguments = vec!["pcb", "drc", "--format", "json", "--severity-all"];
    if refill_zones {
        drc_arguments.push("--refill-zones");
    }
    drc_arguments.push("--output");
    run_kicad_report(&drc_arguments, &drc_path, &pcb)?;
    let drc: serde_json::Value = read_json(&drc_path)?;
    let erc = serde_json::json!({
        "source": format!("{board_id}.kicad_sch"),
        "date": drc["date"].as_str().unwrap_or("unknown"),
        "kicad_version": "none: board-only project, no schematic",
        "coordinate_units": "mm",
        "sheets": [],
    });
    write_typed_json(&directory.join("erc.json"), &erc)?;
    write_verification_report(directory, board_id, &erc, &drc)
}

pub(crate) fn write_verification_report(
    directory: &Path,
    board_id: &str,
    erc: &serde_json::Value,
    drc: &serde_json::Value,
) -> Result<VerificationReport, String> {
    remove_if_present(&directory.join("verification.json"))?;
    let report = assess_verification_reports(board_id, erc, drc)?;
    write_typed_json(&directory.join("verification.json"), &report)?;
    Ok(report)
}

pub(crate) fn assess_verification_reports(
    board_id: &str,
    erc: &serde_json::Value,
    drc: &serde_json::Value,
) -> Result<VerificationReport, String> {
    validate_report(
        erc,
        ReportKind::Erc,
        Path::new(&format!("{board_id}.kicad_sch")),
    )?;
    validate_report(
        drc,
        ReportKind::Drc,
        Path::new(&format!("{board_id}.kicad_pcb")),
    )?;
    let erc_violations = erc_design_issues(erc).len();
    let mut erc_library_warnings = BTreeMap::<String, usize>::new();
    for finding in erc["sheets"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|sheet| sheet["violations"].as_array().into_iter().flatten())
        .filter(|finding| is_erc_library_warning(finding))
    {
        *erc_library_warnings
            .entry(finding["type"].as_str().unwrap_or("").to_string())
            .or_default() += 1;
    }
    let schematic_parity_issues = drc["schematic_parity"].as_array().map_or(0, Vec::len);
    let mut drc_design_violations = 0;
    let mut library_metadata_warnings = 0;
    for violation in drc["violations"].as_array().into_iter().flatten() {
        if is_library_metadata_warning(violation) {
            library_metadata_warnings += 1;
        } else {
            drc_design_violations += 1;
        }
    }
    let mut intentional_no_connect_groups = 0;
    let mut selected_net_unconnected_items = 0;
    for violation in drc["unconnected_items"].as_array().into_iter().flatten() {
        if is_intentional_no_connect_group(violation) {
            intentional_no_connect_groups += 1;
        } else {
            selected_net_unconnected_items += 1;
        }
    }
    let complete = erc_violations == 0
        && drc_design_violations == 0
        && schematic_parity_issues == 0
        && selected_net_unconnected_items == 0;
    let report = VerificationReport {
        board_id: board_id.to_string(),
        complete,
        erc_violations,
        drc_design_violations,
        schematic_parity_issues,
        selected_net_unconnected_items,
        intentional_no_connect_groups,
        library_metadata_warnings,
        erc_library_warnings,
    };
    Ok(report)
}

enum KiCadCacheInput {
    Cacheable(String),
    Bypassed(String),
}

fn verify_materialized_rung_cached(
    directory: &Path,
    board_id: &str,
    refill_zones: bool,
    cache_root: &Path,
) -> Result<VerificationReport, String> {
    remove_if_present(&directory.join("verification.json"))?;
    let started = Instant::now();
    let kicad_cli_version = kicad_cli_version()?;
    let verifier_sha256 = kicad_verifier_implementation_sha256();
    let input_sha256 = match kicad_cache_input_sha256(directory)? {
        KiCadCacheInput::Cacheable(hash) => hash,
        KiCadCacheInput::Bypassed(detail) => {
            let report = verify_materialized_rung_uncached(directory, board_id, refill_zones)?;
            write_kicad_cache_trace(
                directory,
                &KiCadVerificationCacheTrace {
                    schema_version: KICAD_VERIFICATION_CACHE_SCHEMA_VERSION,
                    status: KiCadVerificationCacheStatus::Bypassed,
                    key: None,
                    input_sha256: None,
                    kicad_cli_version,
                    verifier_sha256,
                    policy: KICAD_VERIFICATION_POLICY.into(),
                    refill_zones,
                    cache_root: cache_root.to_path_buf(),
                    cache_entry: None,
                    elapsed_micros: started.elapsed().as_micros() as u64,
                    detail,
                },
            )?;
            return Ok(report);
        }
    };
    let key = {
        let mut hash = Sha256::new();
        hash.update(KICAD_VERIFICATION_CACHE_SCHEMA_VERSION.to_le_bytes());
        hash.update(KICAD_VERIFICATION_POLICY.as_bytes());
        hash.update([u8::from(refill_zones)]);
        hash.update((kicad_cli_version.len() as u64).to_le_bytes());
        hash.update(kicad_cli_version.as_bytes());
        hash.update(verifier_sha256.as_bytes());
        hash.update(input_sha256.as_bytes());
        format!("{:x}", hash.finalize())
    };
    fs::create_dir_all(cache_root).map_err(|error| {
        format!(
            "failed to create KiCad verification cache {}: {error}",
            cache_root.display()
        )
    })?;
    let cache_entry = cache_root.join(&key);
    if cache_entry.is_dir() {
        let entry: KiCadVerificationCacheEntry =
            read_typed_json(&cache_entry.join("cache-entry.json"))?;
        if entry.schema_version != KICAD_VERIFICATION_CACHE_SCHEMA_VERSION
            || entry.key != key
            || entry.input_sha256 != input_sha256
            || entry.kicad_cli_version != kicad_cli_version
            || entry.verifier_sha256 != verifier_sha256
            || entry.policy != KICAD_VERIFICATION_POLICY
            || entry.refill_zones != refill_zones
            || entry.report.board_id != board_id
        {
            return Err(format!(
                "KiCad verification cache entry {} does not match its key",
                cache_entry.display()
            ));
        }
        let cached_erc = cache_entry.join("erc.json");
        let cached_drc = cache_entry.join("drc.json");
        if file_sha256(&cached_erc)? != entry.erc_sha256
            || file_sha256(&cached_drc)? != entry.drc_sha256
        {
            return Err(format!(
                "KiCad verification cache entry {} has corrupted reports",
                cache_entry.display()
            ));
        }
        let assessed = assess_verification_reports(
            board_id,
            &read_json(&cached_erc)?,
            &read_json(&cached_drc)?,
        )?;
        if assessed != entry.report {
            return Err(format!(
                "KiCad verification cache entry {} has an inconsistent assessment",
                cache_entry.display()
            ));
        }
        fs::copy(&cached_erc, directory.join("erc.json"))
            .map_err(|error| format!("failed to restore cached ERC report: {error}"))?;
        fs::copy(&cached_drc, directory.join("drc.json"))
            .map_err(|error| format!("failed to restore cached DRC report: {error}"))?;
        write_typed_json(&directory.join("verification.json"), &entry.report)?;
        write_kicad_cache_trace(
            directory,
            &KiCadVerificationCacheTrace {
                schema_version: KICAD_VERIFICATION_CACHE_SCHEMA_VERSION,
                status: KiCadVerificationCacheStatus::Hit,
                key: Some(key),
                input_sha256: Some(input_sha256),
                kicad_cli_version,
                verifier_sha256,
                policy: KICAD_VERIFICATION_POLICY.into(),
                refill_zones,
                cache_root: cache_root.to_path_buf(),
                cache_entry: Some(cache_entry),
                elapsed_micros: started.elapsed().as_micros() as u64,
                detail: "restored exact ERC, DRC, and interpreted verification reports".into(),
            },
        )?;
        return Ok(entry.report);
    }

    let report = verify_materialized_rung_uncached(directory, board_id, refill_zones)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock precedes UNIX epoch: {error}"))?
        .as_nanos();
    let temporary = cache_root.join(format!(".{key}.{}.{}.tmp", std::process::id(), nonce));
    fs::create_dir(&temporary).map_err(|error| {
        format!(
            "failed to create temporary cache entry {}: {error}",
            temporary.display()
        )
    })?;
    let erc = directory.join("erc.json");
    let drc = directory.join("drc.json");
    fs::copy(&erc, temporary.join("erc.json"))
        .map_err(|error| format!("failed to cache ERC report: {error}"))?;
    fs::copy(&drc, temporary.join("drc.json"))
        .map_err(|error| format!("failed to cache DRC report: {error}"))?;
    fs::copy(
        directory.join("verification.json"),
        temporary.join("verification.json"),
    )
    .map_err(|error| format!("failed to cache verification report: {error}"))?;
    let entry = KiCadVerificationCacheEntry {
        schema_version: KICAD_VERIFICATION_CACHE_SCHEMA_VERSION,
        key: key.clone(),
        input_sha256: input_sha256.clone(),
        kicad_cli_version: kicad_cli_version.clone(),
        verifier_sha256: verifier_sha256.clone(),
        policy: KICAD_VERIFICATION_POLICY.into(),
        refill_zones,
        erc_sha256: file_sha256(&erc)?,
        drc_sha256: file_sha256(&drc)?,
        report: report.clone(),
    };
    write_typed_json(&temporary.join("cache-entry.json"), &entry)?;
    if let Err(error) = fs::rename(&temporary, &cache_entry) {
        if cache_entry.is_dir() {
            fs::remove_dir_all(&temporary).map_err(|cleanup| {
                format!(
                    "cache entry race ({error}) and failed to remove {}: {cleanup}",
                    temporary.display()
                )
            })?;
        } else {
            return Err(format!(
                "failed to publish cache entry {}: {error}",
                cache_entry.display()
            ));
        }
    }
    write_kicad_cache_trace(
        directory,
        &KiCadVerificationCacheTrace {
            schema_version: KICAD_VERIFICATION_CACHE_SCHEMA_VERSION,
            status: KiCadVerificationCacheStatus::Miss,
            key: Some(key),
            input_sha256: Some(input_sha256),
            kicad_cli_version,
            verifier_sha256,
            policy: KICAD_VERIFICATION_POLICY.into(),
            refill_zones,
            cache_root: cache_root.to_path_buf(),
            cache_entry: Some(cache_entry),
            elapsed_micros: started.elapsed().as_micros() as u64,
            detail: "ran native KiCad ERC/DRC and published an immutable cache entry".into(),
        },
    )?;
    Ok(report)
}

/// Identifies the native-verification implementation independently of the
/// router executable that happens to call it.
///
/// The cache intentionally invalidates conservatively after any change to this
/// crate or the resolved Rust dependency graph. Changes confined to placement
/// and routing crates retain native KiCad verification hits when their emitted
/// KiCad inputs are byte-identical.
pub fn kicad_verifier_implementation_sha256() -> String {
    let mut hash = Sha256::new();
    hash.update(b"pcb-maker-kicad-verifier-implementation-v1");
    // lib.rs held all of these until 2026-10-21.
    hash.update(include_bytes!("lib.rs"));
    hash.update(include_bytes!("kicad_sexpr.rs"));
    hash.update(include_bytes!("kicad_nets.rs"));
    hash.update(include_bytes!("verification.rs"));
    hash.update(include_bytes!("fs_util.rs"));
    hash.update(include_bytes!("native_report.rs"));
    hash.update(include_bytes!("../../../Cargo.lock"));
    format!("{:x}", hash.finalize())
}

fn kicad_cli_version() -> Result<String, String> {
    let output = Command::new(crate::native_report::kicad_cli())
        .arg("version")
        .output()
        .map_err(|error| format!("failed to identify kicad-cli: {error}"))?;
    if !output.status.success() {
        return Err(format!("kicad-cli version exited with {}", output.status));
    }
    String::from_utf8(output.stdout)
        .map(|version| version.trim().to_string())
        .map_err(|error| format!("kicad-cli version was not UTF-8: {error}"))
}

fn kicad_cache_input_sha256(directory: &Path) -> Result<KiCadCacheInput, String> {
    for table in ["sym-lib-table", "fp-lib-table"] {
        let path = directory.join(table);
        if !path.is_file() {
            continue;
        }
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        for tail in source.split("(uri \"").skip(1) {
            let Some(uri) = tail.split('"').next() else {
                return Ok(KiCadCacheInput::Bypassed(format!(
                    "could not parse library URI in {table}"
                )));
            };
            if !uri.starts_with("${KIPRJMOD}/") {
                return Ok(KiCadCacheInput::Bypassed(format!(
                    "{table} references external library {uri:?}"
                )));
            }
        }
    }
    let mut files = Vec::new();
    collect_kicad_cache_inputs(directory, directory, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    if files.is_empty() {
        return Err(format!(
            "{} contains no KiCad design inputs",
            directory.display()
        ));
    }
    let mut hash = Sha256::new();
    hash.update(b"pcb-maker-kicad-verification-input-v1");
    for (relative, absolute) in files {
        let relative = relative.as_os_str().as_encoded_bytes();
        let contents = fs::read(&absolute)
            .map_err(|error| format!("failed to read {}: {error}", absolute.display()))?;
        hash.update((relative.len() as u64).to_le_bytes());
        hash.update(relative);
        hash.update((contents.len() as u64).to_le_bytes());
        hash.update(contents);
    }
    Ok(KiCadCacheInput::Cacheable(format!("{:x}", hash.finalize())))
}

fn collect_kicad_cache_inputs(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<(), String> {
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("failed to read {}: {error}", directory.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", entry.path().display()))?;
        if file_type.is_symlink() {
            return Err(format!(
                "KiCad verification cache rejects symlink {}",
                entry.path().display()
            ));
        }
        if file_type.is_dir() {
            collect_kicad_cache_inputs(root, &entry.path(), files)?;
            continue;
        }
        if !file_type.is_file() || !is_kicad_cache_input(&entry.path()) {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_path_buf();
        files.push((relative, entry.path()));
    }
    Ok(())
}

fn is_kicad_cache_input(path: &Path) -> bool {
    let name = path.file_name().and_then(|name| name.to_str());
    if matches!(name, Some("sym-lib-table" | "fp-lib-table")) {
        return true;
    }
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("kicad_sch" | "kicad_pcb" | "kicad_pro" | "kicad_sym" | "kicad_mod" | "kicad_dru")
    )
}

fn write_kicad_cache_trace(
    directory: &Path,
    trace: &KiCadVerificationCacheTrace,
) -> Result<(), String> {
    write_typed_json(&directory.join("verification-cache.json"), trace)?;
    if let Some(event_path) = env::var_os(KICAD_VERIFICATION_CACHE_EVENTS_ENV) {
        let event_path = PathBuf::from(event_path);
        if let Some(parent) = event_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create cache event directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let mut events = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&event_path)
            .map_err(|error| {
                format!(
                    "failed to open cache event log {}: {error}",
                    event_path.display()
                )
            })?;
        let serialized = serde_json::to_string(trace).map_err(|error| error.to_string())?;
        writeln!(events, "{serialized}").map_err(|error| {
            format!(
                "failed to append cache event log {}: {error}",
                event_path.display()
            )
        })?;
    }
    Ok(())
}

fn is_intentional_no_connect_group(violation: &serde_json::Value) -> bool {
    let Some(items) = violation["items"].as_array() else {
        return false;
    };
    if items.is_empty() {
        return false;
    }
    let mut component = None;
    for item in items {
        let Some(description) = item["description"].as_str() else {
            return false;
        };
        let Some(net_start) = description.find("[unconnected-(") else {
            return false;
        };
        let Some(net_end) = description[net_start..].find("] of ") else {
            return false;
        };
        let after_net = &description[net_start + net_end + 5..];
        let Some(reference) = after_net.split(" on ").next() else {
            return false;
        };
        match component {
            None => component = Some(reference),
            Some(expected) if expected == reference => {}
            Some(_) => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_cache_hashes_design_inputs_and_ignores_reports() {
        assert_eq!(kicad_verifier_implementation_sha256().len(), 64);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-kicad-cache-input-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("board.kicad_pcb"), "(kicad_pcb)\n").unwrap();
        fs::write(directory.join("board.kicad_pro"), "{}\n").unwrap();
        fs::write(
            directory.join("sym-lib-table"),
            r#"(sym_lib_table (lib (uri "${KIPRJMOD}/Generated.kicad_sym")))"#,
        )
        .unwrap();
        let digest = match kicad_cache_input_sha256(&directory).unwrap() {
            KiCadCacheInput::Cacheable(digest) => digest,
            KiCadCacheInput::Bypassed(reason) => panic!("unexpected bypass: {reason}"),
        };
        fs::write(directory.join("erc.json"), "generated report\n").unwrap();
        let after_report = match kicad_cache_input_sha256(&directory).unwrap() {
            KiCadCacheInput::Cacheable(digest) => digest,
            KiCadCacheInput::Bypassed(reason) => panic!("unexpected bypass: {reason}"),
        };
        assert_eq!(digest, after_report);
        fs::write(directory.join("board.kicad_pcb"), "(kicad_pcb (zone))\n").unwrap();
        let after_board = match kicad_cache_input_sha256(&directory).unwrap() {
            KiCadCacheInput::Cacheable(digest) => digest,
            KiCadCacheInput::Bypassed(reason) => panic!("unexpected bypass: {reason}"),
        };
        assert_ne!(digest, after_board);

        fs::write(
            directory.join("fp-lib-table"),
            r#"(fp_lib_table (lib (uri "/external/library.pretty")))"#,
        )
        .unwrap();
        assert!(matches!(
            kicad_cache_input_sha256(&directory).unwrap(),
            KiCadCacheInput::Bypassed(reason) if reason.contains("external library")
        ));
        fs::remove_dir_all(directory).unwrap();
    }
}
