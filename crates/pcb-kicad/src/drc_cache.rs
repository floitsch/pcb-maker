// Copyright (C) 2026 Toit contributors.

//! A disk cache of KiCad's DRC reports.
//!
//! `kicad-cli pcb drc` takes 15-25 s and 2.2 GB per call, and many calls
//! are on byte-identical inputs: the reference board of every route run, a
//! ladder attempt that ends with the same board as another, a rerun. The
//! report is a function of what kicad-cli reads, so the key hashes all of
//! it:
//!
//! - the kicad-cli version and the DRC arguments (severities, format,
//!   zone refill, schematic parity);
//! - the board's file name (the report's `source`);
//! - every KiCad design file under the board's directory (the board, the
//!   `.kicad_pro` and `.kicad_dru` rules, schematic sheets for parity,
//!   project libraries and their tables), by relative path and bytes;
//! - the global library tables (`fp-lib-table`, `sym-lib-table` under the
//!   configuration directory kicad-cli runs with: pcb-maker's library-free
//!   one, see `kicad_config`, or the user's) and every `KICAD*` environment
//!   variable kicad-cli sees (library paths, `KICAD_CONFIG_HOME`).
//!
//! Not hashed: the contents of libraries outside the project (KiCad's own,
//! which change with KiCad, whose version is in the key). They only feed
//! the `lib_footprint_*` warnings, which never count.
//!
//! The value is the report's exact bytes. A hit is validated like a fresh
//! report and published the same way (written next to the output and
//! renamed), so a caller cannot tell them apart except by the report's
//! `date`, which is the original run's. Only successful runs are stored.
//! Entries are written to a temporary file and renamed into place, so
//! concurrent processes never see half an entry; two processes that miss
//! the same key both run KiCad and the second rename wins with the same
//! bytes. After a store the oldest entries (by last use: a hit touches its
//! entry) go once the cache holds more than [`MAX_ENTRIES`] entries or
//! [`MAX_BYTES`] bytes.
//!
//! Location: `$PCB_MAKER_CACHE_DIR/drc`, else `$XDG_CACHE_HOME/pcb-maker/drc`,
//! else `~/.cache/pcb-maker/drc`. `PCB_MAKER_NO_CACHE=1` turns it off. A
//! cache that cannot be read or written is skipped with one line on stderr;
//! it never fails a DRC.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Entries kept at most; the oldest go first.
pub const MAX_ENTRIES: usize = 2000;
/// Bytes kept at most (2 GB).
pub const MAX_BYTES: u64 = 2 << 30;
/// Format of the key; bump it when the key's inputs change.
const KEY_VERSION: &str = "pcb-maker-drc-cache-v1";
/// Project trees larger than this are not hashed (the DRC runs uncached).
const MAX_PROJECT_FILES: usize = 5000;
const MAX_PROJECT_BYTES: u64 = 1 << 30;
/// Directory depth below the board's directory that is hashed.
const MAX_PROJECT_DEPTH: usize = 6;

static CALLS: AtomicU64 = AtomicU64::new(0);
static HITS: AtomicU64 = AtomicU64::new(0);
static WARNED: AtomicBool = AtomicBool::new(false);

/// DRC runs asked for by this process and how many the cache answered.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DrcCacheCounts {
    pub calls: u64,
    pub hits: u64,
}

/// This process's DRC calls and cache hits so far.
pub fn drc_cache_counts() -> DrcCacheCounts {
    DrcCacheCounts { calls: CALLS.load(Ordering::Relaxed), hits: HITS.load(Ordering::Relaxed) }
}

pub(crate) fn count_call() {
    CALLS.fetch_add(1, Ordering::Relaxed);
}

fn warn(message: &str) {
    if !WARNED.swap(true, Ordering::Relaxed) {
        eprintln!("DRC cache skipped: {message}");
    }
}

/// The cache directory, or `None` when the cache is off.
pub(crate) fn directory_from_env() -> Option<PathBuf> {
    let variable = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    if variable("PCB_MAKER_NO_CACHE").is_some_and(|value| value != "0") {
        return None;
    }
    Some(crate::kicad_config::cache_root_from_env()?.join("drc"))
}

/// `program version`, asked once per program and process.
pub(crate) fn tool_version(program: &OsStr) -> Option<String> {
    static VERSIONS: OnceLock<Mutex<HashMap<OsString, Option<String>>>> = OnceLock::new();
    let versions = VERSIONS.get_or_init(Default::default);
    if let Some(version) = versions.lock().ok()?.get(program) {
        return version.clone();
    }
    let version = Command::new(program)
        .arg("version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());
    versions.lock().ok()?.insert(program.to_os_string(), version.clone());
    version
}

/// What a DRC's result depends on besides the project: the tool, its
/// arguments, the global library tables and the KiCad environment.
#[derive(Debug, Clone)]
pub(crate) struct KeyContext {
    pub version: String,
    pub arguments: Vec<String>,
    /// (name, bytes) of global files, sorted by name.
    pub globals: Vec<(String, Vec<u8>)>,
    /// `KICAD*` environment variables, sorted.
    pub environment: Vec<(String, String)>,
}

impl KeyContext {
    /// `config_home` is the `KICAD_CONFIG_HOME` kicad-cli runs with when it
    /// is not the process's own (see `kicad_config`): its tables are hashed.
    fn current(program: &OsStr, arguments: &[&str], config_home: Option<&Path>) -> Result<Self, String> {
        let version = tool_version(program).ok_or("kicad-cli version unknown")?;
        let mut environment: Vec<(String, String)> = std::env::vars_os()
            .filter_map(|(name, value)| Some((name.into_string().ok()?, value.to_string_lossy().into_owned())))
            .filter(|(name, _)| name.starts_with("KICAD"))
            .filter(|(name, _)| config_home.is_none() || name != "KICAD_CONFIG_HOME")
            .collect();
        if let Some(home) = config_home {
            environment.push(("KICAD_CONFIG_HOME".into(), home.display().to_string()));
        }
        environment.sort();
        let mut globals = Vec::new();
        let variable = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
        let config = config_home
            .map(Path::to_path_buf)
            .or_else(|| variable("KICAD_CONFIG_HOME").map(PathBuf::from))
            .or_else(|| variable("XDG_CONFIG_HOME").map(|base| PathBuf::from(base).join("kicad")))
            .or_else(|| variable("HOME").map(|home| PathBuf::from(home).join(".config/kicad")));
        if let Some(config) = config.filter(|path| path.is_dir()) {
            let mut candidates = vec![config.clone()];
            if let Ok(entries) = fs::read_dir(&config) {
                candidates.extend(entries.flatten().map(|entry| entry.path()).filter(|path| path.is_dir()));
            }
            for directory in candidates {
                for table in ["fp-lib-table", "sym-lib-table"] {
                    let path = directory.join(table);
                    if let Ok(bytes) = fs::read(&path) {
                        globals.push((path.display().to_string(), bytes));
                    }
                }
            }
        }
        globals.sort();
        Ok(Self { version, arguments: arguments.iter().map(|argument| argument.to_string()).collect(), globals, environment })
    }
}

fn is_design_file(name: &str) -> bool {
    matches!(name, "fp-lib-table" | "sym-lib-table")
        || [".kicad_pcb", ".kicad_pro", ".kicad_dru", ".kicad_sch", ".kicad_sym", ".kicad_mod"]
            .iter()
            .any(|extension| name.ends_with(extension))
}

/// The KiCad design files under `root` (hidden entries skipped, symbolic
/// links followed to a bounded depth), as relative paths, sorted.
pub(crate) fn project_files(root: &Path) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    fn walk(root: &Path, directory: &Path, depth: usize, files: &mut Vec<(PathBuf, PathBuf)>, bytes: &mut u64) -> Result<(), String> {
        let entries = fs::read_dir(directory).map_err(|error| format!("{}: {error}", directory.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            // Follows symbolic links; a dangling one is no input.
            let Ok(metadata) = fs::metadata(&path) else { continue };
            if metadata.is_dir() {
                if depth < MAX_PROJECT_DEPTH {
                    walk(root, &path, depth + 1, files, bytes)?;
                }
            } else if metadata.is_file() && is_design_file(name) {
                *bytes += metadata.len();
                let relative = path.strip_prefix(root).map_err(|error| error.to_string())?.to_path_buf();
                files.push((relative, path));
                if files.len() > MAX_PROJECT_FILES || *bytes > MAX_PROJECT_BYTES {
                    return Err(format!("{} holds too many KiCad files to hash", root.display()));
                }
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(root, root, 0, &mut files, &mut 0)?;
    files.sort();
    Ok(files)
}

/// The key of a DRC of `input` under `context`.
pub(crate) fn key(context: &KeyContext, input: &Path) -> Result<String, String> {
    let mut hash = Sha256::new();
    let mut field = |bytes: &[u8]| {
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    };
    field(KEY_VERSION.as_bytes());
    field(context.version.as_bytes());
    field(&(context.arguments.len() as u64).to_le_bytes());
    for argument in &context.arguments {
        field(argument.as_bytes());
    }
    field(&(context.environment.len() as u64).to_le_bytes());
    for (name, value) in &context.environment {
        field(name.as_bytes());
        field(value.as_bytes());
    }
    field(&(context.globals.len() as u64).to_le_bytes());
    for (name, bytes) in &context.globals {
        field(name.as_bytes());
        field(bytes);
    }
    let name = input.file_name().ok_or_else(|| format!("{} names no file", input.display()))?;
    field(name.as_encoded_bytes());
    let root = match input.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let files = project_files(root)?;
    if !files.iter().any(|(relative, _)| relative.as_os_str() == name) {
        return Err(format!("{} does not exist", input.display()));
    }
    field(&(files.len() as u64).to_le_bytes());
    for (relative, path) in files {
        let bytes = fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        field(relative.as_os_str().as_encoded_bytes());
        field(&bytes);
    }
    drop(field);
    Ok(format!("{:x}", hash.finalize()))
}

/// A unique name for a temporary file next to `path`.
fn temporary_next_to(path: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).map(|time| time.as_nanos()).unwrap_or(0);
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!(".{name}.{}.{nonce}.{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)))
}

/// Writes `bytes` to `path` so that readers see the old file or the whole
/// new one: a temporary file in the same directory, renamed into place.
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = temporary_next_to(path);
    let written = fs::write(&temporary, bytes)
        .map_err(|error| format!("failed to write {}: {error}", temporary.display()))
        .and_then(|()| fs::rename(&temporary, path).map_err(|error| format!("failed to rename to {}: {error}", path.display())));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// A DRC cache in `directory`.
pub(crate) struct DrcCache {
    directory: PathBuf,
    key: String,
}

impl DrcCache {
    /// The cache entry for a DRC of `input`, or `None` when the cache is
    /// off or the inputs cannot be hashed.
    pub(crate) fn for_run(
        directory: PathBuf,
        program: &OsStr,
        arguments: &[&str],
        input: &Path,
        config_home: Option<&Path>,
    ) -> Option<Self> {
        let derived = KeyContext::current(program, arguments, config_home).and_then(|context| key(&context, input));
        match derived {
            Ok(key) => Some(Self { directory, key }),
            Err(error) => {
                warn(&error);
                None
            }
        }
    }

    fn entry(&self) -> PathBuf {
        self.directory.join(format!("{}.json", self.key))
    }

    /// The cached report's bytes, if any; a hit marks the entry as used.
    pub(crate) fn lookup(&self) -> Option<Vec<u8>> {
        let entry = self.entry();
        let bytes = fs::read(&entry).ok()?;
        if let Ok(file) = fs::File::options().write(true).open(&entry) {
            let _ = file.set_modified(SystemTime::now());
        }
        Some(bytes)
    }

    /// Drops an entry that did not validate.
    pub(crate) fn discard(&self) {
        let _ = fs::remove_file(self.entry());
    }

    pub(crate) fn count_hit(&self) {
        HITS.fetch_add(1, Ordering::Relaxed);
    }

    /// Stores a fresh report's bytes and keeps the cache within its bounds.
    pub(crate) fn store(&self, bytes: &[u8]) {
        let stored = fs::create_dir_all(&self.directory)
            .map_err(|error| format!("failed to create {}: {error}", self.directory.display()))
            .and_then(|()| write_atomically(&self.entry(), bytes));
        match stored {
            Ok(()) => evict(&self.directory, MAX_ENTRIES, MAX_BYTES),
            Err(error) => warn(&error),
        }
    }
}

/// Removes the least recently used entries until at most nine tenths of
/// `max_entries` and `max_bytes` remain, once either is exceeded. Also
/// removes temporary files a killed process left (older than an hour).
/// Other processes may evict at the same time: what is gone is gone.
pub(crate) fn evict(directory: &Path, max_entries: usize, max_bytes: u64) {
    let Ok(entries) = fs::read_dir(directory) else { return };
    let mut kept = Vec::new();
    let mut total = 0u64;
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else { continue };
        if !metadata.is_file() {
            continue;
        }
        let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            if name.ends_with(".tmp") && now.duration_since(modified).unwrap_or_default() > Duration::from_secs(3600) {
                let _ = fs::remove_file(entry.path());
            }
            continue;
        }
        if !name.ends_with(".json") {
            continue;
        }
        total += metadata.len();
        kept.push((modified, metadata.len(), entry.path()));
    }
    if kept.len() <= max_entries && total <= max_bytes {
        return;
    }
    kept.sort();
    let (entry_goal, byte_goal) = (max_entries * 9 / 10, max_bytes / 10 * 9);
    let mut count = kept.len();
    for (_, size, path) in kept {
        if count <= entry_goal && total <= byte_goal {
            break;
        }
        let _ = fs::remove_file(&path);
        count -= 1;
        total = total.saturating_sub(size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let directory = temporary_next_to(&std::env::temp_dir().join(format!("drc-cache-test-{label}")));
            fs::create_dir_all(&directory).unwrap();
            Self(directory)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn context() -> KeyContext {
        KeyContext {
            version: "10.0.7".into(),
            arguments: ["pcb", "drc", "--format", "json", "--severity-all", "--output"].map(String::from).to_vec(),
            globals: vec![("fp-lib-table".into(), b"(fp_lib_table)".to_vec())],
            environment: vec![("KICAD10_FOOTPRINT_DIR".into(), "/usr/share/kicad/footprints".into())],
        }
    }

    fn project(directory: &Path) -> PathBuf {
        fs::write(directory.join("board.kicad_pcb"), "(kicad_pcb (version 20240108))\n").unwrap();
        fs::write(directory.join("board.kicad_pro"), "{\"board\": {}}\n").unwrap();
        fs::write(directory.join("notes.txt"), "not a design file").unwrap();
        directory.join("board.kicad_pcb")
    }

    #[test]
    fn the_key_follows_every_input_and_nothing_else() {
        let scratch = Scratch::new("key");
        let board = project(&scratch.0);
        let base = key(&context(), &board).unwrap();
        assert_eq!(base, key(&context(), &board).unwrap(), "the key is deterministic");
        assert_eq!(base.len(), 64);

        // Files kicad-cli does not read, and hidden ones (our temporary
        // report directories), leave the key alone.
        fs::write(scratch.0.join("notes.txt"), "changed").unwrap();
        fs::write(scratch.0.join("drc.json"), "{}").unwrap();
        fs::create_dir(scratch.0.join(".kicad-report-1")).unwrap();
        fs::write(scratch.0.join(".kicad-report-1/x.kicad_pcb"), "x").unwrap();
        assert_eq!(base, key(&context(), &board).unwrap());

        // One byte of the board, the rules, a new rule file, a schematic or a
        // project library changes it.
        let changes: [(&str, &str); 5] = [
            ("board.kicad_pcb", "(kicad_pcb (version 20240109))\n"),
            ("board.kicad_pro", "{\"board\": {}} \n"),
            ("board.kicad_dru", "(version 1)\n"),
            ("board.kicad_sch", "(kicad_sch)\n"),
            ("lib/parts.pretty/R.kicad_mod", "(footprint R)\n"),
        ];
        let mut seen = vec![base.clone()];
        for (file, contents) in changes {
            let path = scratch.0.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let before = fs::read(&path).ok();
            fs::write(&path, contents).unwrap();
            let changed = key(&context(), &board).unwrap();
            assert!(!seen.contains(&changed), "{file} did not change the key");
            seen.push(changed);
            match before {
                Some(bytes) => fs::write(&path, bytes).unwrap(),
                None => fs::remove_file(&path).unwrap(),
            }
        }
        let _ = fs::remove_dir_all(scratch.0.join("lib"));
        assert_eq!(base, key(&context(), &board).unwrap(), "restored inputs give the old key");

        // So do the tool, its arguments, the global tables and environment.
        let mut other = context();
        other.version = "10.0.8".into();
        let mut variants = vec![other];
        let mut other = context();
        other.arguments.insert(5, "--refill-zones".into());
        variants.push(other);
        let mut other = context();
        other.arguments.insert(5, "--schematic-parity".into());
        variants.push(other);
        let mut other = context();
        other.globals[0].1.push(b' ');
        variants.push(other);
        let mut other = context();
        other.environment.clear();
        variants.push(other);
        for variant in variants {
            let changed = key(&variant, &board).unwrap();
            assert!(!seen.contains(&changed), "{variant:?} did not change the key");
            seen.push(changed);
        }

        // The same bytes under another file name: the report's source differs.
        fs::copy(&board, scratch.0.join("other.kicad_pcb")).unwrap();
        let renamed = key(&context(), &scratch.0.join("other.kicad_pcb")).unwrap();
        assert!(!seen.contains(&renamed));

        // A missing board has no key.
        assert!(key(&context(), &scratch.0.join("missing.kicad_pcb")).is_err());
    }

    #[test]
    fn fields_cannot_run_into_each_other() {
        let scratch = Scratch::new("fields");
        let board = project(&scratch.0);
        let mut first = context();
        first.arguments = vec!["ab".into(), "c".into()];
        let mut second = context();
        second.arguments = vec!["a".into(), "bc".into()];
        assert_ne!(key(&first, &board).unwrap(), key(&second, &board).unwrap());
    }

    #[test]
    fn atomic_writes_replace_whole_files_and_leave_no_temporaries() {
        let scratch = Scratch::new("atomic");
        let path = scratch.0.join("entry.json");
        write_atomically(&path, b"first").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first");
        write_atomically(&path, b"second, longer").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second, longer");
        let names: Vec<_> = fs::read_dir(&scratch.0).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        assert_eq!(names, vec![OsString::from("entry.json")]);
        // A failed write (no such directory) leaves nothing behind either.
        assert!(write_atomically(&scratch.0.join("missing/entry.json"), b"x").is_err());

        // Writers side by side: every read sees one whole version.
        let threads: Vec<_> = (0..8)
            .map(|index| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let contents = vec![b'a' + index as u8; 64 * 1024];
                    for _ in 0..20 {
                        write_atomically(&path, &contents).unwrap();
                        let read = fs::read(&path).unwrap();
                        assert_eq!(read.len(), 64 * 1024);
                        assert!(read.iter().all(|byte| *byte == read[0]));
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 1);
    }

    #[test]
    fn eviction_drops_the_least_recently_used_entries() {
        let scratch = Scratch::new("evict");
        let base = SystemTime::now() - Duration::from_secs(1000);
        for index in 0..20u64 {
            let path = scratch.0.join(format!("{index:02}.json"));
            fs::write(&path, vec![b'x'; 100]).unwrap();
            fs::File::options().write(true).open(&path).unwrap().set_modified(base + Duration::from_secs(index)).unwrap();
        }
        let stale = scratch.0.join(".00.json.1.2.3.tmp");
        fs::write(&stale, "half").unwrap();
        fs::File::options().write(true).open(&stale).unwrap().set_modified(base - Duration::from_secs(7200)).unwrap();
        evict(&scratch.0, 30, 1 << 20);
        assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 20, "within bounds: only the stale temporary goes");
        // Entry 00 was used last: it stays.
        fs::File::options().write(true).open(scratch.0.join("00.json")).unwrap().set_modified(SystemTime::now()).unwrap();
        evict(&scratch.0, 10, 1 << 20);
        let mut left: Vec<_> = fs::read_dir(&scratch.0).unwrap().map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, vec!["00.json", "12.json", "13.json", "14.json", "15.json", "16.json", "17.json", "18.json", "19.json"]);
        // The byte bound alone.
        evict(&scratch.0, 100, 500);
        assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 4);
    }

    #[test]
    fn the_cache_directory_follows_the_environment_rules() {
        // Only reads the environment; the variables are not changed here
        // (tests run side by side).
        if std::env::var_os("PCB_MAKER_NO_CACHE").is_some_and(|value| !value.is_empty() && value != "0") {
            assert!(directory_from_env().is_none());
        } else if let Some(directory) = directory_from_env() {
            assert!(directory.ends_with("drc"));
        }
    }
}
