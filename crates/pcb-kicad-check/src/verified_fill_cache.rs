// Copyright (C) 2026 Florian Loitsch.

//! Cache a native check of stored zone copper for a self-contained project.

use super::*;
use crate::drc_cache::{DrcCache, KeyContext};
use std::ffi::OsStr;
#[cfg(unix)]
use std::sync::{Mutex, OnceLock};

const MAX_ENTRIES: usize = 128;
const MAX_BYTES: u64 = 256 << 20;
pub(crate) const MAX_ENTRY_BYTES: u64 = 64 << 20;
const POLICY: &str = "pcb-maker-verified-fill-v2";
const LAYOUT_REFILL_POLICY: &str = "pcb-maker-layout-refill-v1";

pub struct VerifiedFillCache {
    cache: DrcCache,
    key: String,
    version: String,
}

impl VerifiedFillCache {
    pub fn for_board(board: &Path) -> Option<Self> {
        let directory = drc_cache::directory_from_env()?.with_file_name("verified-fills");
        let (key, version) = refill_identity(board)?;
        Some(Self {
            cache: DrcCache::keyed(directory, key.clone()),
            key,
            version,
        })
    }

    pub(crate) fn has_entry(&self) -> bool {
        self.cache.contains_bounded(MAX_ENTRY_BYTES)
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn matches_current(&self, board: &Path) -> bool {
        Self::for_board(board).is_some_and(|current| current.key == self.key)
    }

    pub fn lookup(&self) -> Option<Vec<u8>> {
        self.cache.lookup_bounded(MAX_ENTRY_BYTES)
    }

    pub fn discard(&self) {
        self.cache.discard();
    }

    pub fn store(&self, bytes: &[u8]) {
        if bytes.len() as u64 <= MAX_ENTRY_BYTES {
            self.cache.store_with_bounds(bytes, MAX_ENTRIES, MAX_BYTES);
        }
    }

    pub fn validate_stored_report(
        &self,
        bytes: &[u8],
        board: &Path,
    ) -> Result<serde_json::Value, String> {
        let report: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        validate_report(&report, ReportKind::Drc, board)?;
        if report["kicad_version"].as_str() != Some(self.version.as_str()) {
            return Err("cached fill report names a different KiCad version".into());
        }
        Ok(report)
    }
}

/// Exact proof identity even when the persistent cache is disabled.
pub(crate) fn refill_identity(board: &Path) -> Option<(String, String)> {
    if !contained_dependencies(board) {
        return None;
    }
    let arguments = native_report::exact_drc_arguments(true, true, true);
    refill_key(board, arguments, POLICY, Vec::new())
}

/// An ordinary layout receipt never leaves its checked parent directory.
/// Its sibling symbol libraries can therefore be bound to their resolved
/// paths and contents (including a verified missing file) for this run.
pub(crate) fn layout_refill_identity(board: &Path) -> Option<(String, String)> {
    let dependencies = layout_library_inputs(board)?;
    refill_key(
        board,
        native_report::ordinary_drc_arguments(true, true, true),
        LAYOUT_REFILL_POLICY,
        dependencies,
    )
}

fn refill_key<'a>(
    board: &Path,
    mut arguments: Vec<&'a str>,
    policy: &'a str,
    dependencies: Vec<(String, Vec<u8>)>,
) -> Option<(String, String)> {
    let program = native_report::kicad_cli();
    let config = kicad_config::drc_config_home(&program, board)?;
    // The save pass and stored pass must use the same engines, library
    // tables, project dependencies and native report policy.
    arguments.extend([policy, "LC_ALL=C", "stored: no refill or save"]);
    let mut context = KeyContext::current(&program, &arguments, Some(&config)).ok()?;
    if policy == LAYOUT_REFILL_POLICY {
        // KiCad writes its application settings on first use. With no
        // recorded starting settings, keep the ordinary check and leave
        // capture for a later run with an established native context.
        let settings = config.join(kicad_config::series(&context.version)?);
        if ["kicad_common.json", "pcbnew.json", "eeschema.json"]
            .into_iter()
            .any(|name| !settings.join(name).is_file())
        {
            return None;
        }
    }
    context.globals.extend(dependencies);
    context.arguments.extend(native_engine(&program)?);
    for name in ["LANG", "LANGUAGE", "LC_MESSAGES"] {
        if let Some(value) = env::var_os(name) {
            context
                .environment
                .push((name.into(), value.into_string().ok()?));
        }
    }
    context.environment.sort_unstable();
    add_native_settings(&config, &mut context)?;
    let key = drc_cache::key(&context, board).ok()?;
    Some((key, context.version))
}

fn contained_dependencies(board: &Path) -> bool {
    let Some(project) = board.parent() else {
        return false;
    };
    if !local_schematic_and_rules(board) {
        return false;
    }
    ["fp-lib-table", "sym-lib-table"].into_iter().all(|name| {
        let path = project.join(name);
        if !path.exists() {
            return true;
        }
        fs::read_to_string(path)
            .ok()
            .and_then(|text| parse(&text).ok())
            .is_some_and(|table| {
                crate::verification::local_library_table(&table)
                    && table
                        .children()
                        .iter()
                        .filter(|part| part.head() == Some("lib"))
                        .all(|part| {
                            form_atom(part, "uri", 1)
                                .and_then(|uri| uri.strip_prefix("${KIPRJMOD}/"))
                                .is_some_and(|uri| {
                                    Path::new(uri).components().count()
                                        < drc_cache::MAX_PROJECT_DEPTH
                                })
                                && form_atom(part, "options", 1).is_none_or(str::is_empty)
                        })
            })
    })
}

fn local_schematic_and_rules(board: &Path) -> bool {
    !board.with_extension("sch").exists()
        && local_custom_rules(&board.with_extension("kicad_dru"))
        && fs::read_to_string(board.with_extension("kicad_sch"))
            .ok()
            .and_then(|text| parse(&text).ok())
            .is_some_and(|schematic| {
                schematic.head() == Some("kicad_sch") && schematic.child("sheet").is_none()
            })
}

fn layout_library_inputs(board: &Path) -> Option<Vec<(String, Vec<u8>)>> {
    use std::path::Component::{Normal, ParentDir};

    if !local_schematic_and_rules(board) {
        return None;
    }
    let project = fs::canonicalize(board.parent()?).ok()?;
    let parent = project.parent()?;
    // Put the stable parent in the receipt even for boards that only use
    // local libraries. The scratch project is a peer of the checked one.
    let mut inputs = vec![(
        "checked project parent".into(),
        parent.to_str()?.as_bytes().to_vec(),
    )];
    let mut sibling_bytes = 0;
    for (file, head, extension) in [
        ("fp-lib-table", "fp_lib_table", "pretty"),
        ("sym-lib-table", "sym_lib_table", "kicad_sym"),
    ] {
        let path = project.join(file);
        let table = match fs::read_to_string(path) {
            Ok(table) => parse(&table).ok()?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return None,
        };
        if table.head() != Some(head) {
            return None;
        }
        for library in table
            .children()
            .iter()
            .filter(|part| part.head() == Some("lib"))
        {
            if form_atom(library, "type", 1) != Some("KiCad")
                || !matches!(form_atom(library, "options", 1), None | Some("" | "KiCad"))
            {
                return None;
            }
            let uri = form_atom(library, "uri", 1)?.strip_prefix("${KIPRJMOD}/")?;
            if uri.contains(['$', '\\']) {
                return None;
            }
            let relative = Path::new(uri);
            if relative.extension()?.to_str()? != extension {
                return None;
            }
            let mut components = relative.components().peekable();
            let sibling = matches!(components.peek(), Some(ParentDir));
            if sibling {
                if extension != "kicad_sym" {
                    return None;
                }
                components.next();
            }
            if components.clone().count() >= drc_cache::MAX_PROJECT_DEPTH
                || !components.clone().all(
                    |part| matches!(part, Normal(name) if !name.to_string_lossy().starts_with('.')),
                )
            {
                return None;
            }
            if sibling {
                let mut path = parent.to_path_buf();
                for part in components {
                    path.push(part.as_os_str());
                }
                let key = path.to_str()?.to_owned();
                match fs::symlink_metadata(&path) {
                    Ok(metadata) if metadata.is_file() => {
                        sibling_bytes = metadata.len().checked_add(sibling_bytes)?;
                        if sibling_bytes > MAX_ENTRY_BYTES {
                            return None;
                        }
                        inputs.push((format!("symbol library {key}"), fs::read(path).ok()?));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        inputs.push((format!("missing symbol library {key}"), Vec::new()));
                    }
                    _ => return None,
                }
            }
        }
    }
    Some(inputs)
}

fn local_custom_rules(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    fs::read_to_string(path)
        .ok()
        .and_then(|source| parse(&format!("(drc-rules\n{source}\n)")).ok())
        .is_some_and(|rules| {
            rules
                .children()
                .iter()
                .skip(1)
                .all(|rule| matches!(rule.head(), Some("version" | "rule")))
        })
}

fn add_native_settings(config: &Path, context: &mut KeyContext) -> Option<()> {
    for entry in fs::read_dir(config).ok()? {
        let entry = entry.ok()?;
        if !entry.file_type().ok()?.is_dir() {
            continue;
        }
        for name in ["kicad_common.json", "pcbnew.json", "eeschema.json"] {
            let path = entry.path().join(name);
            match fs::read(&path) {
                Ok(bytes) => context.globals.push((path.to_str()?.to_string(), bytes)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return None,
            }
        }
    }
    context.globals.sort_unstable();
    Some(())
}

fn native_engine(program: &OsStr) -> Option<Vec<String>> {
    if [
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "DYLD_INSERT_LIBRARIES",
        "DYLD_LIBRARY_PATH",
    ]
    .iter()
    .any(|name| env::var_os(name).is_some_and(|value| !value.is_empty()))
    {
        return None;
    }
    let executable = resolve_program(program)?;
    // KiCad's PCB engine is a runtime-loaded module. If its install
    // cannot be identified, run the fresh native checks instead.
    engine_files(&executable)
}

fn engine_files(executable: &Path) -> Option<Vec<String>> {
    Some(vec![
        executable.to_str()?.to_string(),
        native_file_sha256(executable)?,
        native_file_sha256(&executable.with_file_name("_pcbnew.kiface"))?,
        native_file_sha256(&executable.with_file_name("_eeschema.kiface"))?,
    ])
}

#[cfg(unix)]
#[derive(Eq, PartialEq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    length: u64,
    modified: std::time::SystemTime,
    changed: (i64, i64),
}

#[cfg(unix)]
fn file_identity(path: &Path) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::metadata(path).ok()?;
    metadata.is_file().then_some(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        length: metadata.len(),
        modified: metadata.modified().ok()?,
        changed: (metadata.ctime(), metadata.ctime_nsec()),
    })
}

#[cfg(unix)]
fn native_file_sha256(path: &Path) -> Option<String> {
    static FILES: OnceLock<Mutex<BTreeMap<PathBuf, (FileIdentity, String)>>> = OnceLock::new();
    let files = FILES.get_or_init(Default::default);
    let identity = file_identity(path)?;
    if let Some((known, hash)) = files.lock().ok()?.get(path)
        && *known == identity
    {
        return Some(hash.clone());
    }
    let hash = file_sha256(path).ok()?;
    if file_identity(path)? != identity {
        return None;
    }
    files
        .lock()
        .ok()?
        .insert(path.to_path_buf(), (identity, hash.clone()));
    Some(hash)
}

#[cfg(not(unix))]
fn native_file_sha256(path: &Path) -> Option<String> {
    file_sha256(path).ok()
}

fn resolve_program(program: &OsStr) -> Option<PathBuf> {
    let program = Path::new(program);
    if program.components().count() != 1 {
        return program.canonicalize().ok();
    }
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|directory| directory.join(program))
        .find(|entry| {
            let Ok(metadata) = entry.metadata() else {
                return false;
            };
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            metadata.is_file()
        })?
        .canonicalize()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "pcb-maker-fill-cache-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn equivalent_relative_projects_share_a_key_and_design_edits_change_it() {
        let root = scratch();
        let left = root.join("one");
        let right = root.join("two");
        let table = |uri| {
            format!(
                r#"(fp_lib_table (lib (name "Lib") (type "KiCad") (uri "{uri}") (options "")))"#
            )
        };
        let custom = "(version 1)\n(rule \"Example\" (constraint clearance (min 0.2mm)))";
        for project in [&left, &right] {
            fs::create_dir_all(project.join("Lib.pretty")).unwrap();
            fs::write(project.join("board.kicad_pcb"), "(kicad_pcb)").unwrap();
            fs::write(project.join("board.kicad_sch"), "(kicad_sch)").unwrap();
            fs::write(project.join("board.kicad_dru"), custom).unwrap();
            fs::write(
                project.join("fp-lib-table"),
                table("${KIPRJMOD}/Lib.pretty"),
            )
            .unwrap();
            fs::write(project.join("Lib.pretty/C.kicad_mod"), "(footprint C)").unwrap();
        }
        let left_board = left.join("board.kicad_pcb");
        let right_board = right.join("board.kicad_pcb");
        assert!(contained_dependencies(&left_board));
        assert!(contained_dependencies(&right_board));
        let context = KeyContext {
            version: "10.0.8".into(),
            arguments: vec![POLICY.into()],
            globals: Vec::new(),
            environment: Vec::new(),
        };
        let original = drc_cache::key(&context, &left_board).unwrap();
        let target_key = || drc_cache::key(&context, &right_board).unwrap();
        assert_eq!(target_key(), original);
        fs::write(&right_board, "(kicad_pcb (version 2))").unwrap();
        assert_ne!(target_key(), original);
        fs::write(&right_board, "(kicad_pcb)").unwrap();
        fs::write(right.join("board.kicad_dru"), custom.replace("0.2", "0.3")).unwrap();
        assert_ne!(target_key(), original);
        fs::write(right.join("board.kicad_dru"), custom).unwrap();
        fs::write(right.join("Lib.pretty/C.kicad_mod"), "(footprint C2)").unwrap();
        assert_ne!(target_key(), original);

        for external in [
            "${KIPRJMOD}/../Lib.pretty",
            "/absolute/Lib.pretty",
            "${KICAD}/Lib.pretty",
        ] {
            fs::write(right.join("fp-lib-table"), table(external)).unwrap();
            assert!(!contained_dependencies(&right_board), "{external}");
        }
        fs::write(right.join("fp-lib-table"), table("${KIPRJMOD}/Lib.pretty")).unwrap();
        fs::write(right.join("board.kicad_sch"), "(kicad_sch (sheet))").unwrap();
        assert!(!contained_dependencies(&right_board));
        fs::write(right.join("board.kicad_sch"), "(kicad_sch)").unwrap();
        fs::write(
            right.join("board.kicad_dru"),
            r#"(include "../outside.kicad_dru")"#,
        )
        .unwrap();
        assert!(!contained_dependencies(&right_board));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn layout_capture_binds_one_parent_symbol_dependencies_without_caching_them() {
        let root = scratch();
        let route = root.join("result");
        let stage = root.join("stage");
        let symbol_table = |uri| {
            format!(
                r#"(sym_lib_table (lib (name "Lib") (type "KiCad") (options "") (uri "{uri}")))"#
            )
        };
        for directory in [&route, &stage] {
            fs::create_dir(directory).unwrap();
            fs::write(directory.join("board.kicad_pcb"), "(kicad_pcb)").unwrap();
            fs::write(directory.join("board.kicad_sch"), "(kicad_sch)").unwrap();
            fs::write(
                directory.join("sym-lib-table"),
                symbol_table("${KIPRJMOD}/../shared/local.kicad_sym"),
            )
            .unwrap();
        }
        let route_board = route.join("board.kicad_pcb");
        let stage_board = stage.join("board.kicad_pcb");
        assert!(!contained_dependencies(&route_board));
        let absent = layout_library_inputs(&route_board).unwrap();
        assert_eq!(layout_library_inputs(&stage_board).unwrap(), absent);
        let shared = root.join("shared");
        fs::create_dir(&shared).unwrap();
        let local = shared.join("local.kicad_sym");
        fs::write(&local, "(kicad_symbol_lib)").unwrap();
        let present = layout_library_inputs(&route_board).unwrap();
        assert_ne!(present, absent);
        assert_eq!(layout_library_inputs(&stage_board).unwrap(), present);
        fs::write(&local, "(kicad_symbol_lib (symbol X))").unwrap();
        assert_ne!(layout_library_inputs(&route_board).unwrap(), present);
        for uri in [
            "${KIPRJMOD}/../../shared/local.kicad_sym",
            "/shared/local.kicad_sym",
            "${UNKNOWN}/shared/local.kicad_sym",
        ] {
            fs::write(route.join("sym-lib-table"), symbol_table(uri)).unwrap();
            assert!(layout_library_inputs(&route_board).is_none(), "{uri}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replaces_a_memoized_tool_identity_when_the_native_engine_changes() {
        let root = scratch();
        let executable = root.join("kicad-cli");
        let pcb = root.join("_pcbnew.kiface");
        let schematic = root.join("_eeschema.kiface");
        fs::write(&executable, "cli").unwrap();
        fs::write(&pcb, "pcb-a").unwrap();
        fs::write(&schematic, "sch-a").unwrap();
        let original = engine_files(&executable).unwrap();
        fs::write(&pcb, "pcb-b").unwrap();
        let pcb_edit = engine_files(&executable).unwrap();
        assert_ne!(original, pcb_edit);
        fs::write(&schematic, "sch-b").unwrap();
        assert_ne!(engine_files(&executable).unwrap(), pcb_edit);
        fs::remove_file(schematic).unwrap();
        assert!(engine_files(&executable).is_none());
        fs::remove_dir_all(root).unwrap();
    }
}
