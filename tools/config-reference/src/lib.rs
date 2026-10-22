// Copyright (C) 2026 Toit contributors.

//! The configuration reference (docs/config-reference.md) and the
//! hand-over's environment index, generated from the code.
//!
//! The tables come from the config structs themselves: the field names,
//! types, doc comments and serde attributes from their source files, the
//! defaults from the structs at run time (a settings file of `{}` read
//! through serde, serialized again; the core configs' `Default` printed
//! with `{:#?}`). Which router-core and placer-core fields a setting
//! reaches is read from the functions that copy them (`core_config`,
//! `place_kicad_board`). The environment index lists every `PCB_*` name
//! the crates' sources hold, each with the effect written in
//! [`ENVIRONMENT`]; a name without one, or one the code no longer reads,
//! is a problem ([`problems`]), as is a user-facing variable that
//! `pcb-maker --help` leaves out (`ENVIRONMENT_HELP` in src/main.rs).
//!
//! ```text
//! cargo run -p config-reference      # rewrites both files
//! ```
//!
//! tests/config_reference.rs (the root package's) fails while the
//! committed files differ from what this generates.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

/// The generated reference, relative to the repository root.
pub const REFERENCE: &str = "docs/config-reference.md";

/// The hand-over, whose environment index lies between these markers.
pub const HANDOVER: &str = "docs/handover.md";
const INDEX_BEGIN: &str =
    "<!-- The environment index is generated: cargo run -p config-reference. -->";
const INDEX_END: &str = "<!-- End of the generated environment index. -->";

/// The documents whose naming of a key the reference marks.
const KEY_DOCUMENTS: [&str; 2] = ["docs/router.md", "docs/placer.md"];

/// What a variable is for, which decides its group in the index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// Changes what a run does; `pcb-maker --help` lists it.
    User,
    /// A switch back to an older default, or a tuning hook for experiments.
    Switch,
    /// Prints or writes diagnostics; the result does not change.
    Diagnostic,
    /// Used by the tests.
    Test,
}

impl Kind {
    fn heading(self) -> &'static str {
        match self {
            Kind::User => "User-facing (`pcb-maker --help` lists them)",
            Kind::Switch => "Switches and experiment hooks (change the result)",
            Kind::Diagnostic => "Diagnostics (stderr unless said; the result does not change)",
            Kind::Test => "Tests and research commands",
        }
    }
}

/// Every environment variable the code reads, with its effect. Names not
/// in a `PCB_*` string literal of the crates (read by a dependency, or
/// KiCad's own) are in [`EXTERNAL`].
pub const ENVIRONMENT: &[(&str, Kind, &str)] = &[
    (
        "RAYON_NUM_THREADS",
        Kind::User,
        "threads of the router's searches and of the placer's seed pool (default: every core); results do not depend on it.",
    ),
    (
        "PCB_WORK_RATE",
        Kind::User,
        "`<expansions/s>[,<placer units/s>]`: the work rates pinned instead of measured at the start (a verdict's `work_rate` and `placer_work_rate` repeat its run; `3.15e6,150e6` is the reference's, run.py's default); wins over the router settings `work_rate` and `placer_work_rate`. `pcb-maker calibrate-work-rate [rounds]` prints the measurement alone.",
    ),
    (
        "PCB_ROUTER_SEEDS",
        Kind::User,
        "`N`: with the `auto` router config, N seeds routed side by side and the best kept (the router setting `seeds`).",
    ),
    (
        "PCB_SKIP_NATIVE",
        Kind::User,
        "with any router config (a file too, since 2026-10-21), no KiCad checks (`skip_native_verification`; machines without kicad-cli): the verdict is `router_complete_unverified` at best, exit 1.",
    ),
    (
        "PCB_KICAD_CLI",
        Kind::User,
        "the kicad-cli program to run (default `kicad-cli` on the PATH; on macOS `/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli`). The agent commands ask its version before any work: missing, failing or older than KiCad 10 is an input error, exit 2 (a warning where the run needs no KiCad).",
    ),
    (
        "KICAD10_FOOTPRINT_DIR",
        Kind::User,
        "KiCad's own variable (`KICAD9_FOOTPRINT_DIR`, `KICAD8_FOOTPRINT_DIR` after it): import-kicad-netlist's folder of KiCad's footprint libraries, before the standard install folders (Linux, macOS, Windows).",
    ),
    (
        "PCB_ROUTER_NO_WALL_GUARD",
        Kind::User,
        "no wall-clock guard in the router, and the layout sets no wall-clock deadline: work alone ends runs (determinism checks, the walkthrough test).",
    ),
    (
        "PCB_PLACER_WORK_SECONDS",
        Kind::User,
        "place-kicad-board's placement work budget in seconds of work (default 600, the layout's share).",
    ),
    (
        "PCB_KICAD_TIMEOUT",
        Kind::User,
        "seconds one kicad-cli report may take before it is killed (default 900).",
    ),
    (
        "PCB_MIN_CONNECTION_FLOOR",
        Kind::User,
        "`warning`: floor the router's track widths at the board's minimum connection width also when KiCad rates connection_width a warning; `off`: never (default: only at error severity).",
    ),
    (
        "PCB_KICAD_NO_NECK_REPAIR",
        Kind::User,
        "no connection-width neck repair after the final DRC.",
    ),
    (
        "PCB_ROUTER_ISLAND_STUBS",
        Kind::User,
        "turns the router's island stubs on (`island_stubs`, off since 2026-10-21): a pad's straight track into a fill island its spokes reach, which only silences KiCad's isolated-island finding.",
    ),
    (
        "PCB_MAKER_CACHE_DIR",
        Kind::User,
        "root of the DRC cache and of the library-free kicad-cli configuration (default `$XDG_CACHE_HOME/pcb-maker`, else `~/.cache/pcb-maker`).",
    ),
    (
        "PCB_MAKER_NO_CACHE",
        Kind::User,
        "anything but `0`: no DRC cache (both sides of a CPU pair).",
    ),
    (
        "PCB_MAKER_KICAD_USER_LIBS",
        Kind::User,
        "anything but `0`: kicad-cli's DRC and ERC run with the user's KiCad configuration (global libraries) instead of empty library tables.",
    ),
    (
        "PCB_MAKER_MEMORY_MB",
        Kind::User,
        "a resident memory limit in megabytes beside the router setting `memory_mb` and the limits the process reads itself (RLIMIT_AS, the cgroup's): the ladder runs as many routers side by side as fit the tightest.",
    ),
    (
        "PCB_MAKER_LEDGER",
        Kind::User,
        "the machine's memory ledger, where routes, layouts and KiCad's checks reserve their memory before they allocate and wait for room (default `$XDG_RUNTIME_DIR/pcb-maker-ledger.json`, else `/tmp/pcb-maker-ledger.json`; `off`: none; `pcb-maker ledger` lists it). Reservations do not change results.",
    ),
    (
        "PCB_MAKER_LEDGER_MARGIN_MB",
        Kind::User,
        "the memory the ledger keeps free of reservations (default 6144): all reservations together take at most MemTotal less it.",
    ),
    (
        "PCB_MAKER_LEDGER_TOTAL_MB",
        Kind::User,
        "what the ledger's reservations may take together, in place of MemTotal less the margin (tests).",
    ),
    (
        "PCB_ROUTER_NO_SPOKELESS_TRACKS",
        Kind::Switch,
        "turns off the router's `spokeless_tracks` (a straight track into kept fill for a pad the spoke model sees without a spoke).",
    ),
    (
        "PCB_ROUTER_NO_APART_TREES",
        Kind::Switch,
        "turns off the router's `apart_trees` (terminals no path reaches from their net's tree joined among themselves).",
    ),
    (
        "PCB_ROUTER_NO_LOST_SPOKES",
        Kind::Switch,
        "turns off the router's `lost_spoke_repair` (other nets rerouted around a starved pad's cut spokes after the clean-up).",
    ),
    (
        "PCB_ROUTER_SEED",
        Kind::Switch,
        "`N`: perturbs the net order and seeds the history with noise, so that one board routes many ways.",
    ),
    (
        "PCB_NARROW_RESERVE",
        Kind::Switch,
        "`share`: the share of the ladder's first attempt's work kept back for the narrow-signal step (wins over the router setting `narrow_reserve_share`; default 0.75).",
    ),
    (
        "PCB_WALL_STRETCH_MAX",
        Kind::Switch,
        "`cap`: the most the wall-clock guards stretch for pinned rates faster than the machine measured (the pinned rate over the measured; default 4, 1: no stretch, the guards before 2026-10-21).",
    ),
    (
        "PCB_LADDER_SEARCH_ONLY",
        Kind::Switch,
        "route mode's ladder counts the attempts' search expansions alone: no manual spokes, pour analyses, lattices or lowerings, and its shares stay search work (the count before 2026-10-21; for attributing rows; was `PCB_LADDER_UNCHARGED_SPOKES`, which named only the spokes).",
    ),
    (
        "PCB_LADDER_UNCHARGED_POUR",
        Kind::Switch,
        "route mode's ladder counts the attempts' search expansions and manual spokes only, not their other pour analyses, lattice builds and lowerings, and its shares stay search work (the count before 2026-10-21, all pour work charged; for attributing rows).",
    ),
    (
        "PCB_LADDER_SEARCH_SHARES",
        Kind::Switch,
        "route mode's ladder charges all the work but gives each router its share as search work, not scaled by the routers' share of search so far (for attributing rows).",
    ),
    (
        "PCB_LADDER_NO_FIRST_ITERATION_GATE",
        Kind::Switch,
        "route mode's steps start without room for their first negotiation iteration (the rule before 2026-10-21; for attributing rows).",
    ),
    (
        "PCB_LADDER_NO_STEP_COST_GATE",
        Kind::Switch,
        "route mode's steps start with less left under the bound than a fifth of the ladder's first attempt's work (the rule before 2026-10-21, probe overhead; for attributing rows).",
    ),
    (
        "PCB_LADDER_SEARCH_LIMITS",
        Kind::Switch,
        "route mode's first attempt stops on its search alone, its pour analyses and lattice on top (the rule before 2026-10-21, probe overhead; for attributing rows).",
    ),
    (
        "PCB_LADDER_PROBES_COUNT_ALL",
        Kind::Switch,
        "route mode's rung probes stop on all their work (search, pour analyses, lattice and their part of the static maps) instead of on their search (an experiment: leaders changed; docs/changelog.md, probe overhead).",
    ),
    (
        "PCB_LADDER_CONTINUATION_LATTICE",
        Kind::Switch,
        "route mode's probed rung's continuation takes its share in search work by the routers' proportion of search with their lattices counted, as the fresh routers do (the rule before 2026-10-21, probe overhead; for attributing rows).",
    ),
    (
        "PCB_ROUTER_NO_SHARED_STATICS",
        Kind::Switch,
        "every router builds its own static maps and pour masks instead of sharing those of routers of the same board, lattice and classes (for attributing rows; the routing is the same).",
    ),
    (
        "PCB_ROUTER_LIMITS_BETWEEN_ITERATIONS",
        Kind::Switch,
        "a negotiation's work limits (its own and the work deadline) bind only after a whole iteration, not between its batches too (the rule before 2026-10-22, probe overshoot; for attributing rows).",
    ),
    (
        "PCB_PROBES_SIDE_BY_SIDE",
        Kind::Switch,
        "`N`: at most N probes of the probe ladder at once (1: one after the other).",
    ),
    (
        "PCB_ROUTER_DEBUG",
        Kind::Diagnostic,
        "the router's verbose lines (failed searches, the resolution, what blocks a stub, lattice details).",
    ),
    (
        "PCB_ROUTER_DUMP",
        Kind::Diagnostic,
        "`x0,y0,x1,y1`: prints that region of the lattice per layer (static blocks, own pads, occupancy, where vias may not go).",
    ),
    (
        "PCB_ROUTER_FINGERPRINT",
        Kind::Diagnostic,
        "a hash of the routing state at each phase (determinism and byte-identity checks; compare as a multiset when attempts run side by side).",
    ),
    (
        "PCB_ROUTER_FINGERPRINT_ALL",
        Kind::Diagnostic,
        "with `PCB_ROUTER_FINGERPRINT`, also hashes every occupancy map and pour mask into each fingerprint.",
    ),
    (
        "PCB_ROUTER_POUR_TRACE",
        Kind::Diagnostic,
        "per pour analysis its counts, modelled work and wall time (calibrating the pour work constants).",
    ),
    (
        "PCB_ROUTER_SEVER_TRACE",
        Kind::Diagnostic,
        "per refresh of what a cut severs of the pours (`pour_integrity`) its nodes, wall time and marked nodes per layer (calibrating `SEVER_NODE_NANOS`).",
    ),
    (
        "PCB_ROUTER_BUILD_TRACE",
        Kind::Diagnostic,
        "per router build and per lowering their counts, modelled work and wall time (calibrating the build and lowering work constants).",
    ),
    (
        "PCB_ROUTER_POUR_DUMP",
        Kind::Diagnostic,
        "`<dir>`: writes the pour maps at each finish.",
    ),
    (
        "PCB_ROUTER_SEARCH_RATE",
        Kind::Diagnostic,
        "the search times itself for the `profile rate:` line (calibrating `REFERENCE_EXPANSIONS_PER_SECOND`).",
    ),
    (
        "PCB_ROUTER_TIMING",
        Kind::Diagnostic,
        "where a reroute's time goes (layout moves).",
    ),
    (
        "PCB_ROUTER_SPOKE_DEBUG",
        Kind::Diagnostic,
        "the spoke model's decisions per pad.",
    ),
    (
        "PCB_ROUTER_DUMP_MANUAL_SPOKES",
        Kind::Diagnostic,
        "dump-pour-map also adds the manual spokes and says how many.",
    ),
    (
        "PCB_PLACER_DEBUG",
        Kind::Diagnostic,
        "the placer's progress lines.",
    ),
    (
        "PCB_GLOBAL_DEBUG",
        Kind::Diagnostic,
        "the global route's negotiation, a line per iteration (nets to reroute, overflow, search work; docs/global-route.md).",
    ),
    (
        "PCB_PLACER_TRACE",
        Kind::Diagnostic,
        "`<file>`: per seed the weighted wirelength after each stage and per part how far each stage moved it.",
    ),
    (
        "PCB_PLACER_DESIGNER",
        Kind::Diagnostic,
        "`<board>`: the designer's poses judged by the placer's model (their wirelength, what each illegal movable part meets), and with `PCB_PLACER_TRACE` in the trace beside ours.",
    ),
    (
        "PCB_PLACER_DESIGNER_LEVEL",
        Kind::Diagnostic,
        "`full`: judge the designer's poses at the full spacing; `tight`, `touch`: with tight bodies, and courtyards touching (default: the smallest spacing, no halos).",
    ),
    (
        "PCB_PLACER_CHECK_OVERLAP",
        Kind::Diagnostic,
        "the annealer checks its incremental overlap against a full recount.",
    ),
    (
        "PCB_LADDER_STATS",
        Kind::Diagnostic,
        "the probe ladder prints each probe's other statistics (the ladder's judge).",
    ),
    (
        "PCB_KICAD_NECKS_DEBUG",
        Kind::Diagnostic,
        "the neck finder per net and layer.",
    ),
    (
        "PCB_LABELS_DEBUG",
        Kind::Diagnostic,
        "copper texts the label mover could not place.",
    ),
    (
        "PCB_OUTLINE_DEBUG",
        Kind::Diagnostic,
        "the outline's area, vertices and cutouts.",
    ),
    (
        "PCB_TOPO_DEBUG",
        Kind::Diagnostic,
        "the topological engine's failures in short (overflowing edges by kind, failed and unrouted wires).",
    ),
    (
        "PCB_TOPO_SVG",
        Kind::Diagnostic,
        "`<dir>`: the topological engine writes an SVG picture of every stage there.",
    ),
    (
        "PCB_TOPO_TAUT",
        Kind::Diagnostic,
        "the topological engine reports taut wires that do not settle.",
    ),
    (
        "PCB_TOPO_TRACE",
        Kind::Diagnostic,
        "`N`: the topological engine traces the radii of wire N.",
    ),
    (
        "PCB_MAKER_KICAD_VERIFICATION_CACHE",
        Kind::Test,
        "`<dir>`: verify-kicad-rung caches its KiCad checks there.",
    ),
    (
        "PCB_MAKER_KICAD_VERIFICATION_CACHE_EVENTS",
        Kind::Test,
        "`<file>`: verify-kicad-rung's cache appends its hits and misses there (tests/native_verification.rs).",
    ),
    (
        "PCB_MAKER_TEST_PANIC",
        Kind::Test,
        "panic once a run has started (tests/cli_exit_codes.rs).",
    ),
    (
        "PCB_MAKER_TEST_PANIC_ATTEMPT",
        Kind::Test,
        "`<kind>[:<n>|:all]`: the n-th (default the first) attempt of the router's ladder of a kind (probe, continuation, rung, seeds, step), or every one, panics; the ladder records it in `attempts_failed` and goes on (docs/router.md, \"The ladder in code, and its faults\"; tests/cli_exit_codes.rs).",
    ),
    (
        "PCB_MAKER_TEST_DROP_DRC",
        Kind::Test,
        "the route's drc.json removed before the verdict reads it (tests/cli_exit_codes.rs).",
    ),
];

/// Variables of [`ENVIRONMENT`] that are not `PCB_*` names: a dependency's
/// (`RAYON_NUM_THREADS`) and KiCad's own (`KICAD10_FOOTPRINT_DIR`).
pub const EXTERNAL: [&str; 2] = ["RAYON_NUM_THREADS", "KICAD10_FOOTPRINT_DIR"];

/// One field of a config struct, as its source declares it.
#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    pub ty: String,
    pub doc: String,
    /// `#[serde(skip)]`: set by the code, never read from a file.
    pub skip: bool,
    /// `#[serde(default)]` or `#[serde(default = "...")]`.
    pub serde_default: bool,
    /// `#[serde(flatten)]`: its fields stand in the parent's object.
    pub flatten: bool,
    /// The expression of the field in the struct's `Default` impl.
    pub default_expression: Option<String>,
}

/// Where a table's defaults come from.
enum Defaults {
    /// A settings file of `{}`, read and serialized again (the field's
    /// JSON), and the struct printed with `{:#?}` (for skipped fields).
    Json(serde_json::Value, String),
    /// The struct's `Default` printed with `{:#?}`.
    Debug(String),
    /// No default: a list entry whose fields are required or optional.
    Entry,
}

/// How a table relates to settings files.
enum Role {
    /// Its fields are keys of a settings file (`prefix` names the place).
    File,
    /// A core config: the map from its fields to the settings they come
    /// from, and the function that copies them.
    Core(BTreeMap<String, Source>, &'static str),
    /// A part of a report the command writes.
    Output,
}

/// Where a core field comes from in the KiCad layer.
#[derive(Clone, Debug, Default)]
pub struct Source {
    pub keys: BTreeSet<String>,
    /// Assigned from something other than a setting (the board, a rule).
    pub computed: bool,
}

struct Table {
    title: &'static str,
    /// The key path in a settings file, such as `topological.` (empty at
    /// the top level).
    prefix: &'static str,
    intro: &'static str,
    file: &'static str,
    name: &'static str,
    fields: Vec<Field>,
    defaults: Defaults,
    role: Role,
}

fn read(root: &Path, path: &str) -> Result<String, String> {
    fs::read_to_string(root.join(path)).map_err(|error| format!("failed to read {path}: {error}"))
}

/// The fields of `pub struct name` in `source`, in declaration order.
pub fn parse_struct(source: &str, name: &str) -> Result<Vec<Field>, String> {
    let header = format!("pub struct {name} {{");
    let mut lines = source.lines().skip_while(|line| line.trim() != header);
    if lines.next().is_none() {
        return Err(format!("no `{header}`"));
    }
    let mut fields = Vec::new();
    let mut doc: Vec<String> = Vec::new();
    let mut attributes = String::new();
    let mut attribute_depth = 0i32;
    let mut declaration = String::new();
    for line in lines {
        if line == "}" {
            break;
        }
        let trimmed = line.trim();
        if attribute_depth > 0 {
            attributes.push_str(trimmed);
            attribute_depth += depth_change(trimmed, '[', ']');
            continue;
        }
        if declaration.is_empty() {
            if let Some(text) = trimmed.strip_prefix("///") {
                doc.push(text.strip_prefix(' ').unwrap_or(text).to_string());
                continue;
            }
            if trimmed.starts_with("//") || trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with("#[") {
                attributes.push_str(trimmed);
                attribute_depth = depth_change(trimmed, '[', ']');
                continue;
            }
        }
        if !declaration.is_empty() {
            declaration.push(' ');
        }
        declaration.push_str(trimmed);
        let open = depth_change(&declaration, '<', '>') + depth_change(&declaration, '(', ')');
        if !(declaration.ends_with(',') && open == 0) {
            continue;
        }
        let text = declaration.trim_end_matches(',');
        let text = text
            .strip_prefix("pub(crate) ")
            .or_else(|| text.strip_prefix("pub "))
            .unwrap_or(text);
        let (field, ty) = text
            .split_once(':')
            .ok_or_else(|| format!("{name}: cannot read the field {declaration:?}"))?;
        let serde: Vec<String> = serde_items(&attributes);
        fields.push(Field {
            name: field.trim().to_string(),
            ty: ty.trim().to_string(),
            doc: doc.join(" ").trim().to_string(),
            skip: serde.iter().any(|item| item == "skip"),
            serde_default: serde
                .iter()
                .any(|item| item == "default" || item.starts_with("default =")),
            flatten: serde.iter().any(|item| item == "flatten"),
            default_expression: None,
        });
        doc.clear();
        attributes.clear();
        declaration.clear();
    }
    let expressions = default_expressions(source, name);
    for field in &mut fields {
        field.default_expression = expressions.get(&field.name).cloned();
    }
    Ok(fields)
}

fn depth_change(text: &str, open: char, close: char) -> i32 {
    // `->` in a type is no closing bracket.
    let text = text.replace("->", "");
    text.chars().filter(|c| *c == open).count() as i32
        - text.chars().filter(|c| *c == close).count() as i32
}

/// The items of the `#[serde(...)]` attributes in `attributes`.
fn serde_items(attributes: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut rest = attributes;
    while let Some(start) = rest.find("#[serde(") {
        let inner = &rest[start + "#[serde(".len()..];
        let end = inner.find(")]").unwrap_or(inner.len());
        items.extend(inner[..end].split(',').map(|item| item.trim().to_string()));
        rest = &inner[end..];
    }
    items
}

/// The field expressions of `impl Default for name` (or of its `new`), one
/// line each as rustfmt writes them.
fn default_expressions(source: &str, name: &str) -> BTreeMap<String, String> {
    let mut expressions = BTreeMap::new();
    let start = source
        .find(&format!("impl Default for {name} {{"))
        .or_else(|| source.find(&format!("impl {name} {{\n    pub fn new() -> Self {{")));
    let Some(start) = start else {
        return expressions;
    };
    let body = &source[start..];
    let end = body.find("\n}\n").unwrap_or(body.len());
    for line in body[..end].lines() {
        let trimmed = line.trim();
        if let Some((field, expression)) = trimmed.split_once(": ")
            && is_identifier(field)
            && let Some(expression) = expression.strip_suffix(',')
        {
            expressions.insert(field.to_string(), expression.to_string());
        }
    }
    expressions
}

fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !text.starts_with(|c: char| c.is_ascii_digit())
}

/// The top-level fields of a `{:#?}` print, each value on one line.
pub fn debug_fields(debug: &str) -> BTreeMap<String, String> {
    let mut fields: Vec<(String, String)> = Vec::new();
    for line in debug.lines().skip(1) {
        let indent = line.len() - line.trim_start().len();
        let field = (indent == 4)
            .then(|| line.trim().split_once(": "))
            .flatten()
            .filter(|(name, _)| is_identifier(name));
        if let Some((name, value)) = field {
            fields.push((name.to_string(), value.to_string()));
        } else if indent >= 4
            && let Some((_, value)) = fields.last_mut()
        {
            value.push(' ');
            value.push_str(line.trim());
        }
    }
    fields
        .into_iter()
        .map(|(name, value)| {
            let value = value
                .trim_end_matches(',')
                .replace("[ ", "[")
                .replace("( ", "(")
                .replace("{ ", "{")
                .replace(", ]", "]")
                .replace(", )", ")")
                .replace(", }", "}");
            (name, value)
        })
        .collect()
}

/// The fields of `target` (dotted after the variable, as `global.seed`)
/// that the function of `source` starting at `start` assigns, with the
/// fields of `config` each assignment reads (or the `if let` around it
/// reads), and the fields a struct literal sets as `field: config.key`.
pub fn assignments(
    source: &str,
    start: &str,
    target: &str,
) -> Result<BTreeMap<String, Source>, String> {
    let at = source
        .find(start)
        .ok_or_else(|| format!("no {start:?} to read the settings from"))?;
    let body = &source[at..];
    let body = &body[..body.find("\n}\n").unwrap_or(body.len())];
    let body: String = body
        .lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let mut found: BTreeMap<String, Source> = BTreeMap::new();
    // The `if let` keys in force at each brace depth.
    let mut stack: Vec<Option<String>> = Vec::new();
    let mut segment = String::new();
    let mut literal_depth = None;
    for c in body.chars() {
        match c {
            '{' => {
                let key = (segment.contains("if let") && segment.contains('='))
                    .then(|| config_keys(segment.split_once('=').map_or("", |(_, r)| r)))
                    .and_then(|keys| keys.into_iter().next());
                if literal_depth.is_none()
                    && segment.contains('=')
                    && segment.trim_end().ends_with("core::Config")
                {
                    literal_depth = Some(stack.len() + 1);
                }
                stack.push(key);
                segment.clear();
            }
            '}' | ';' | ',' => {
                if c == ',' && literal_depth != Some(stack.len()) {
                    segment.push(c);
                    continue;
                }
                if literal_depth == Some(stack.len()) {
                    // A field of the core::Config literal: `name: value`.
                    if let Some((field, value)) = segment.trim().split_once(':')
                        && is_identifier(field.trim())
                    {
                        let entry = found.entry(field.trim().to_string()).or_default();
                        let keys = config_keys(value);
                        entry.computed |= keys.is_empty() && !value.contains("..");
                        entry.keys.extend(keys);
                    }
                } else if let Some((field, value)) = assignment(&segment, target) {
                    let entry = found.entry(field).or_default();
                    let mut keys = config_keys(value);
                    if keys.is_empty()
                        && let Some(key) = stack.iter().rev().flatten().next()
                    {
                        keys.insert(key.clone());
                    }
                    entry.computed |= keys.is_empty();
                    entry.keys.extend(keys);
                }
                segment.clear();
                if c == '}' {
                    if literal_depth == Some(stack.len()) {
                        literal_depth = None;
                    }
                    stack.pop();
                }
            }
            _ => segment.push(c),
        }
    }
    Ok(found)
}

/// `target.path = value` in `segment`: the path and the value.
fn assignment<'a>(segment: &'a str, target: &str) -> Option<(String, &'a str)> {
    let prefix = format!("{target}.");
    let start = segment.find(&prefix)?;
    if segment[..start]
        .chars()
        .last()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return None;
    }
    let rest = &segment[start + prefix.len()..];
    let (path, value) = rest.split_once('=')?;
    if value.starts_with('=') || path.ends_with(['!', '<', '>']) {
        return None;
    }
    let path = path.trim();
    path.split('.')
        .all(is_identifier)
        .then(|| (path.to_string(), value))
}

/// The `config.<key>` fields `text` reads (not `router_config.` and the
/// like).
fn config_keys(text: &str) -> BTreeSet<String> {
    // `config\n    .key` as rustfmt breaks a chain.
    let mut joined = String::new();
    for piece in text.split('\n') {
        let piece = if joined.is_empty() {
            piece
        } else {
            piece.trim_start()
        };
        if !(piece.starts_with('.') || joined.is_empty()) {
            joined.push(' ');
        }
        joined.push_str(piece);
    }
    let mut keys = BTreeSet::new();
    let mut rest = joined.as_str();
    while let Some(at) = rest.find("config.") {
        let before = rest[..at].chars().last();
        let after = &rest[at + "config.".len()..];
        if !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
            let key: String = after
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !key.is_empty() {
                keys.insert(key);
            }
        }
        rest = after;
    }
    keys
}

/// The identifiers inside a Markdown file's code spans and code blocks.
pub fn code_identifiers(markdown: &str) -> BTreeSet<String> {
    let mut code = String::new();
    let mut fenced = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            code.push_str(line);
            code.push('\n');
            continue;
        }
        for (index, piece) in line.split('`').enumerate() {
            if index % 2 == 1 {
                code.push_str(piece);
                code.push('\n');
            }
        }
    }
    code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|word| is_identifier(word))
        .map(str::to_string)
        .collect()
}

fn json_defaults<T>() -> Result<Defaults, String>
where
    T: serde::de::DeserializeOwned + serde::Serialize + std::fmt::Debug,
{
    let value: T = serde_json::from_value(serde_json::json!({}))
        .map_err(|error| format!("a settings file of {{}}: {error}"))?;
    let json = serde_json::to_value(&value).map_err(|error| error.to_string())?;
    Ok(Defaults::Json(json, format!("{value:#?}")))
}

fn tables(root: &Path) -> Result<Vec<Table>, String> {
    let router_file = "crates/pcb-kicad/src/board_router/config.rs";
    let placer_file = "crates/pcb-kicad/src/board_placer/config.rs";
    let layout_file = "crates/pcb-kicad/src/board_layout.rs";
    let core_router_file = "crates/pcb-router/src/router/config.rs";
    let core_placer_file = "crates/pcb-placer/src/lib.rs";
    let global_file = "crates/pcb-placer/src/global.rs";
    let anneal_file = "crates/pcb-placer/src/anneal.rs";
    let pin_swap_file = "crates/pcb-kicad/src/pin_swap.rs";
    let pour_file = "crates/pcb-kicad/src/board_router/pour_plan.rs";
    let class_file = "crates/pcb-kicad/src/net_classes.rs";
    let rules_file = "crates/pcb-kicad/src/connection_rules.rs";

    let router = json_defaults::<pcb_kicad::KiCadBoardRouterConfig>()?;
    let layout = json_defaults::<pcb_kicad::KiCadBoardLayoutConfig>()?;
    let nested = |defaults: &Defaults, key: &str| match defaults {
        Defaults::Json(json, _) => Defaults::Json(json[key].clone(), String::new()),
        _ => Defaults::Entry,
    };
    let topological = nested(&router, "topological");
    let pin_swap = nested(&router, "pin_swap");

    // Which settings reach the core configs.
    let core_config_file = find_file(root, "crates/pcb-kicad/src", "fn core_config(")?;
    let router_sources = assignments(
        &read(root, &core_config_file)?,
        "fn core_config(",
        "router_config",
    )?;
    let place_file = find_file(root, "crates/pcb-kicad/src", "pub fn place_kicad_board(")?;
    let placer_sources = assignments(
        &read(root, &place_file)?,
        "let mut placer_config = core::Config::new();",
        "placer_config",
    )?;
    let part = |prefix: &str| -> BTreeMap<String, Source> {
        placer_sources
            .iter()
            .filter_map(|(path, source)| match path.split_once('.') {
                Some((head, field)) if head == prefix => Some((field.to_string(), source.clone())),
                None if prefix.is_empty() => Some((path.clone(), source.clone())),
                _ => None,
            })
            .collect()
    };

    let parse = |file: &str, name: &str| -> Result<Vec<Field>, String> {
        parse_struct(&read(root, file)?, name).map_err(|error| format!("{file}: {error}"))
    };
    Ok(vec![
        Table {
            title: "Router settings",
            prefix: "",
            intro: "The router config file of `route-kicad-board` and `layout-kicad-board` (`auto` resolves it from the project's `.kicad_pro`; a file's keys replace what `auto` would give only where a file sets them, `connection_rules` and `default_rules` together as the classes; docs/router.md). Unknown keys are rejected. `unset` options take the default the doc comment names, mostly the router core's (the next table).",
            file: router_file,
            name: "KiCadBoardRouterConfig",
            fields: parse(router_file, "KiCadBoardRouterConfig")?,
            defaults: router,
            role: Role::File,
        },
        Table {
            title: "Router settings: topological",
            prefix: "topological.",
            intro: "The `topological` object of the router settings (the `engine: \"topological\"` engine).",
            file: router_file,
            name: "KiCadTopologicalConfig",
            fields: parse(router_file, "KiCadTopologicalConfig")?,
            defaults: topological,
            role: Role::File,
        },
        Table {
            title: "Router settings: pin_swap",
            prefix: "pin_swap.",
            intro: "The `pin_swap` object of the router and the layout settings (the pin-swap estimate, docs/pin-swap.md).",
            file: pin_swap_file,
            name: "KiCadPinSwapConfig",
            fields: parse(pin_swap_file, "KiCadPinSwapConfig")?,
            defaults: pin_swap,
            role: Role::File,
        },
        Table {
            title: "Router settings: add_pours entries",
            prefix: "add_pours[].",
            intro: "One entry of `add_pours`.",
            file: pour_file,
            name: "KiCadPourRequest",
            fields: parse(pour_file, "KiCadPourRequest")?,
            defaults: Defaults::Entry,
            role: Role::File,
        },
        Table {
            title: "Router settings: net_classes entries",
            prefix: "net_classes[].",
            intro: "One entry of `net_classes`.",
            file: class_file,
            name: "KiCadNetClassRequest",
            fields: parse(class_file, "KiCadNetClassRequest")?,
            defaults: Defaults::Entry,
            role: Role::File,
        },
        Table {
            title: "Router settings: connection rules",
            prefix: "connection_rules.<net>.",
            intro: "The value of `default_rules` and of each `connection_rules` entry.",
            file: rules_file,
            name: "KiCadConnectionRoutingRules",
            fields: parse(rules_file, "KiCadConnectionRoutingRules")?,
            defaults: Defaults::Entry,
            role: Role::File,
        },
        Table {
            title: "Router report: rules",
            prefix: "rules.",
            intro: "Not a setting: the `rules` object of the router's report (`board-router.json`), what the router made of the project's rules.",
            file: "crates/pcb-kicad/src/board_router/result.rs",
            name: "KiCadRulesSummary",
            fields: parse(
                "crates/pcb-kicad/src/board_router/result.rs",
                "KiCadRulesSummary",
            )?,
            defaults: Defaults::Entry,
            role: Role::Output,
        },
        Table {
            title: "Router report: rules from the project",
            prefix: "rules.",
            intro: "The fields `project` flattens into `rules`; each is left out when empty.",
            file: "crates/pcb-kicad/src/project_rules.rs",
            name: "KiCadProjectRulesReport",
            fields: parse(
                "crates/pcb-kicad/src/project_rules.rs",
                "KiCadProjectRulesReport",
            )?,
            defaults: Defaults::Entry,
            role: Role::Output,
        },
        Table {
            title: "Router report: custom_rules entries",
            prefix: "rules.custom_rules[].",
            intro: "One constraint of one rule of the project's `.kicad_dru`.",
            file: "crates/pcb-kicad/src/project_rules/dru.rs",
            name: "KiCadCustomRuleReport",
            fields: parse(
                "crates/pcb-kicad/src/project_rules/dru.rs",
                "KiCadCustomRuleReport",
            )?,
            defaults: Defaults::Entry,
            role: Role::Output,
        },
        Table {
            title: "Router core",
            prefix: "",
            intro: "`pcb_router::Config`, the router itself: not a file. The KiCad router builds it from the router settings in `core_config` (the column names the settings each field is copied from there); the ladder and the layout change more fields per attempt.",
            file: core_router_file,
            name: "Config",
            fields: parse(core_router_file, "Config")?,
            defaults: Defaults::Debug(format!("{:#?}", pcb_router::Config::default())),
            role: Role::Core(router_sources, "`core_config`"),
        },
        Table {
            title: "Placer settings",
            prefix: "",
            intro: "The placer config file of `place-kicad-board`, and the `placer` object of the layout settings. Unknown keys are rejected; a missing key takes its default.",
            file: placer_file,
            name: "KiCadBoardPlacerConfig",
            fields: parse(placer_file, "KiCadBoardPlacerConfig")?,
            defaults: json_defaults::<pcb_kicad::KiCadBoardPlacerConfig>()?,
            role: Role::File,
        },
        Table {
            title: "Layout settings",
            prefix: "",
            intro: "The layout config file of `layout-kicad-board`. Unknown keys are rejected; a missing key takes its default.",
            file: layout_file,
            name: "KiCadBoardLayoutConfig",
            fields: parse(layout_file, "KiCadBoardLayoutConfig")?,
            defaults: layout,
            role: Role::File,
        },
        Table {
            title: "Layout settings: swappable entries",
            prefix: "swappable[].",
            intro: "One entry of `swappable`.",
            file: layout_file,
            name: "KiCadSwappable",
            fields: parse(layout_file, "KiCadSwappable")?,
            defaults: Defaults::Entry,
            role: Role::File,
        },
        Table {
            title: "Layout settings: free_dimension",
            prefix: "free_dimension.",
            intro: "The `free_dimension` object of the layout settings: one edge of a rectangular outline that `layout-kicad-board` moves to find the smallest clean board (docs/constraints.md, \"A free dimension\").",
            file: "crates/pcb-kicad/src/free_dimension.rs",
            name: "KiCadFreeDimension",
            fields: parse(
                "crates/pcb-kicad/src/free_dimension.rs",
                "KiCadFreeDimension",
            )?,
            defaults: Defaults::Entry,
            role: Role::File,
        },
        Table {
            title: "Layout settings: silkscreen",
            prefix: "silkscreen.",
            intro: "The `silkscreen` object of the layout settings: the footprints' Reference and Value fields, placed after routing where KiCad's silkscreen checks find nothing (docs/placer.md, \"The fields on the silkscreen\").",
            file: layout_file,
            name: "KiCadSilkscreenConfig",
            fields: parse(layout_file, "KiCadSilkscreenConfig")?,
            defaults: json_defaults::<pcb_kicad::KiCadSilkscreenConfig>()?,
            role: Role::File,
        },
        Table {
            title: "Placer core",
            prefix: "",
            intro: "`pcb_placer::Config` (its defaults are `Config::new()`'s): not a file. The KiCad placer builds it from the placer settings in `place_kicad_board`.",
            file: core_placer_file,
            name: "Config",
            fields: parse(core_placer_file, "Config")?,
            defaults: Defaults::Debug(format!("{:#?}", pcb_placer::Config::new())),
            role: Role::Core(part(""), "`place_kicad_board`"),
        },
        Table {
            title: "Placer core: global",
            prefix: "global.",
            intro: "`pcb_placer::GlobalConfig`, the global (electrostatic) placement.",
            file: global_file,
            name: "GlobalConfig",
            fields: parse(global_file, "GlobalConfig")?,
            defaults: Defaults::Debug(format!("{:#?}", pcb_placer::GlobalConfig::default())),
            role: Role::Core(part("global"), "`place_kicad_board`"),
        },
        Table {
            title: "Placer core: anneal",
            prefix: "anneal.",
            intro: "`pcb_placer::anneal::AnnealConfig`, the annealer.",
            file: anneal_file,
            name: "AnnealConfig",
            fields: parse(anneal_file, "AnnealConfig")?,
            defaults: Defaults::Debug(format!(
                "{:#?}",
                pcb_placer::anneal::AnnealConfig::default()
            )),
            role: Role::Core(part("anneal"), "`place_kicad_board`"),
        },
    ])
}

/// The first `.rs` file under `directory` (sorted) holding `text`.
fn find_file(root: &Path, directory: &str, text: &str) -> Result<String, String> {
    let mut files = Vec::new();
    rust_files(&root.join(directory), &mut files);
    files.sort();
    for file in files {
        if fs::read_to_string(&file).is_ok_and(|source| source.contains(text)) {
            return Ok(relative(root, &file));
        }
    }
    Err(format!("no file under {directory} holds {text:?}"))
}

fn rust_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// A table cell: one line, `|` escaped.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

/// A default as the reference shows it.
fn shown_default(field: &Field, defaults: &Defaults, debug: &BTreeMap<String, String>) -> String {
    // A default read from the environment: the value without it, and with.
    if let Some(expression) = &field.default_expression
        && let Some(start) = expression.find("env::var_os(\"")
    {
        let rest = &expression[start + "env::var_os(\"".len()..];
        let name = &rest[..rest.find('"').unwrap_or(rest.len())];
        if expression.ends_with(".is_some()") {
            return format!("`false` (`true` with `{name}`)");
        }
        if expression.ends_with(".is_none()") {
            return format!("`true` (`false` with `{name}`)");
        }
        return format!("`{}`", cell(expression));
    }
    match defaults {
        Defaults::Json(json, _) if !field.skip => match json.get(&field.name) {
            None | Some(serde_json::Value::Null) => "unset".into(),
            Some(value @ serde_json::Value::Object(map)) if !map.is_empty() => {
                if is_nested(&field.ty) {
                    "its own table".into()
                } else {
                    format!("`{}`", cell(&value.to_string()))
                }
            }
            Some(serde_json::Value::Array(items)) => format!(
                "`[{}]`",
                cell(
                    &items
                        .iter()
                        .map(serde_json::Value::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            ),
            Some(value) => format!("`{}`", cell(&value.to_string())),
        },
        Defaults::Entry => {
            if field.serde_default {
                "optional".into()
            } else {
                "required".into()
            }
        }
        _ => match debug.get(&field.name).map(String::as_str) {
            None => "?".into(),
            Some("None") => "unset".into(),
            Some(value) if is_nested(&field.ty) => {
                let _ = value;
                "its own table".into()
            }
            Some(value) => format!("`{}`", cell(value)),
        },
    }
}

/// Types with a table of their own.
fn is_nested(ty: &str) -> bool {
    matches!(
        ty,
        "KiCadTopologicalConfig"
            | "KiCadPinSwapConfig"
            | "KiCadSilkscreenConfig"
            | "KiCadBoardPlacerConfig"
            | "GlobalConfig"
            | "anneal::AnnealConfig"
    )
}

/// The counts of one table, for the summary and the report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub fields: usize,
    pub internal: usize,
    pub with_doc: usize,
    /// File keys (fields not skipped) named in docs/router.md or
    /// docs/placer.md; none for the core tables.
    pub named: usize,
    pub file_keys: usize,
}

/// The reference's text and its tables' counts.
pub fn reference(root: &Path) -> Result<(String, Vec<(String, Counts)>), String> {
    let tables = tables(root)?;
    let named: Vec<(&str, BTreeSet<String>)> = KEY_DOCUMENTS
        .iter()
        .map(|path| Ok((*path, code_identifiers(&read(root, path)?))))
        .collect::<Result<_, String>>()?;
    let mut out = String::new();
    out.push_str("# Configuration reference\n\n");
    out.push_str(
        "Generated from the config structs by `cargo run -p config-reference`\n\
         (tools/config-reference); `cargo test --test config_reference` fails\n\
         while this file is stale. Do not edit it by hand: the descriptions are\n\
         the fields' doc comments, the defaults the structs' own (a settings\n\
         file of `{}` read through serde; the core configs' `Default`).\n\
         \n\
         - *Docs*: the key is named (in code) in [router.md](router.md) or\n\
         \x20 [placer.md](placer.md), where the settings are explained with\n\
         \x20 their measurements.\n\
         - *internal*: `#[serde(skip)]`, set by the code (the ladder, the\n\
         \x20 layout), never read from a file.\n\
         - The core tables are not files: their *Set from* column names the\n\
         \x20 settings the KiCad layer copies into each field (*the KiCad layer*:\n\
         \x20 from the board or a computed value; *-*: the default stands there).\n\
         - The *Router report* tables describe an output, `board-router.json`'s\n\
         \x20 `rules`, not a setting.\n\
         \n\
         The environment variables are indexed in the hand-over\n\
         ([handover.md](handover.md#switches)).\n\n",
    );
    let mut counts = Vec::new();
    let mut sections = String::new();
    for table in &tables {
        let (json_debug, debug) = match &table.defaults {
            Defaults::Json(_, debug) => (true, debug_fields(debug)),
            Defaults::Debug(debug) => (false, debug_fields(debug)),
            Defaults::Entry => (false, BTreeMap::new()),
        };
        let _ = json_debug;
        let mut count = Counts::default();
        let _ = writeln!(sections, "## {}\n", table.title);
        let _ = writeln!(
            sections,
            "{} Source: `{}` in [{}](../{}).\n",
            table.intro, table.name, table.file, table.file
        );
        match &table.role {
            Role::File => {
                sections.push_str("| Key | Type | Default | Docs | Description |\n");
                sections.push_str("|---|---|---|---|---|\n");
            }
            Role::Core(_, function) => {
                let _ = writeln!(
                    sections,
                    "| Field | Type | Default | Set from ({function}) | Description |"
                );
                sections.push_str("|---|---|---|---|---|\n");
            }
            Role::Output => {
                sections.push_str("| Key | Type | Description |\n");
                sections.push_str("|---|---|---|\n");
            }
        }
        for field in &table.fields {
            count.fields += 1;
            count.internal += field.skip as usize;
            count.with_doc += !field.doc.is_empty() as usize;
            let default = shown_default(field, &table.defaults, &debug);
            let doc = if field.doc.is_empty() {
                "(no doc comment)".to_string()
            } else {
                cell(&field.doc)
            };
            match &table.role {
                Role::File => {
                    let documents: Vec<&str> = named
                        .iter()
                        .filter(|(_, names)| names.contains(&field.name))
                        .map(|(path, _)| path.trim_start_matches("docs/"))
                        .collect();
                    let key = if field.skip {
                        format!("`{}{}` (internal)", table.prefix, field.name)
                    } else {
                        count.file_keys += 1;
                        count.named += !documents.is_empty() as usize;
                        format!("`{}{}`", table.prefix, field.name)
                    };
                    let documents = if field.skip || documents.is_empty() {
                        "-".to_string()
                    } else {
                        documents.join(", ")
                    };
                    let default = if field.skip {
                        format!("{default} (internal)")
                    } else {
                        default
                    };
                    let _ = writeln!(
                        sections,
                        "| {key} | `{}` | {default} | {documents} | {doc} |",
                        cell(&field.ty)
                    );
                }
                Role::Output => {
                    let key = if field.flatten {
                        format!("`{}` (flattened: the next table's keys)", field.name)
                    } else {
                        format!("`{}{}`", table.prefix, field.name)
                    };
                    let _ = writeln!(sections, "| {key} | `{}` | {doc} |", cell(&field.ty));
                }
                Role::Core(sources, _) => {
                    let from = match sources.get(&field.name) {
                        Some(source) if !source.keys.is_empty() => source
                            .keys
                            .iter()
                            .map(|key| format!("`{key}`"))
                            .collect::<Vec<_>>()
                            .join(", "),
                        Some(_) => "the KiCad layer".into(),
                        None => "-".into(),
                    };
                    let _ = writeln!(
                        sections,
                        "| `{}{}` | `{}` | {default} | {from} | {doc} |",
                        table.prefix,
                        field.name,
                        cell(&field.ty)
                    );
                }
            }
        }
        sections.push('\n');
        counts.push((table.title.to_string(), count));
    }
    out.push_str("## Summary\n\n");
    out.push_str(
        "| Table | Fields | File keys | Internal | With a doc comment | Named in router.md or placer.md |\n",
    );
    out.push_str("|---|---|---|---|---|---|\n");
    for ((title, count), table) in counts.iter().zip(&tables) {
        let (file_keys, named) = match table.role {
            Role::File => (count.file_keys.to_string(), count.named.to_string()),
            Role::Core(..) | Role::Output => ("-".into(), "-".into()),
        };
        let _ = writeln!(
            out,
            "| [{title}](#{}) | {} | {file_keys} | {} | {} | {named} |",
            anchor(title),
            count.fields,
            count.internal,
            count.with_doc
        );
    }
    out.push('\n');
    out.push_str(&sections);
    let out = out.trim_end().to_string() + "\n";
    Ok((out, counts))
}

/// GitHub's anchor of a heading.
fn anchor(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

/// The environment variables the crates' sources name (`PCB_*` string
/// literals outside tests), each with the files naming it.
pub fn environment_in_code(root: &Path) -> BTreeMap<String, BTreeSet<String>> {
    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files);
    rust_files(&root.join("src"), &mut files);
    let mut found: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for file in files {
        let path = relative(root, &file);
        if path.ends_with("/tests.rs") || path.contains("/tests/") {
            continue;
        }
        let Ok(source) = fs::read_to_string(&file) else {
            continue;
        };
        let source = match source.find("\n#[cfg(test)]\nmod tests") {
            Some(end) => &source[..end],
            None => &source[..],
        };
        for line in source.lines() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            let mut rest = line;
            while let Some(at) = rest.find("\"PCB_") {
                let name: String = rest[at + 1..]
                    .chars()
                    .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                    .collect();
                if rest[at + 1 + name.len()..].starts_with('"') {
                    found.entry(name.clone()).or_default().insert(path.clone());
                }
                rest = &rest[at + 1 + name.len()..];
            }
        }
    }
    found
}

/// The hand-over's environment index (the lines between the markers).
pub fn environment_index() -> String {
    let mut out = String::new();
    let mut kinds: Vec<Kind> = ENVIRONMENT.iter().map(|(_, kind, _)| *kind).collect();
    kinds.sort();
    kinds.dedup();
    for kind in kinds {
        let _ = writeln!(out, "{}:\n", kind.heading());
        for (name, _, effect) in ENVIRONMENT.iter().filter(|(_, k, _)| *k == kind) {
            let _ = writeln!(out, "- `{name}`: {effect}");
        }
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// The hand-over with its environment index replaced by the generated one.
pub fn handover(root: &Path) -> Result<String, String> {
    let text = read(root, HANDOVER)?;
    let begin = text
        .find(INDEX_BEGIN)
        .ok_or_else(|| format!("{HANDOVER}: no {INDEX_BEGIN:?}"))?;
    let end = text
        .find(INDEX_END)
        .ok_or_else(|| format!("{HANDOVER}: no {INDEX_END:?}"))?;
    if end < begin {
        return Err(format!(
            "{HANDOVER}: the index's end comes before its start"
        ));
    }
    Ok(format!(
        "{}{INDEX_BEGIN}\n\n{}\n\n{}",
        &text[..begin],
        environment_index(),
        &text[end..]
    ))
}

/// What is wrong beyond a stale file: variables the code names without an
/// effect in [`ENVIRONMENT`], effects of variables the code no longer
/// names, and user-facing variables `pcb-maker --help` leaves out.
pub fn problems(root: &Path) -> Result<Vec<String>, String> {
    let in_code = environment_in_code(root);
    let described: BTreeSet<&str> = ENVIRONMENT.iter().map(|(name, _, _)| *name).collect();
    let mut problems = Vec::new();
    for (name, files) in &in_code {
        if !described.contains(name.as_str()) {
            problems.push(format!(
                "{name} (in {}) has no effect in tools/config-reference's ENVIRONMENT",
                files.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    for name in &described {
        if !in_code.contains_key(*name) && !EXTERNAL.contains(name) {
            problems.push(format!(
                "{name} is in tools/config-reference's ENVIRONMENT, but no source names it"
            ));
        }
    }
    let main = read(root, "src/main.rs")?;
    let help = main
        .find("const ENVIRONMENT_HELP")
        .map(|start| &main[start..])
        .and_then(|help| help.find("\n];\n").map(|end| &help[..end]))
        .ok_or("src/main.rs: no ENVIRONMENT_HELP")?;
    for (name, kind, _) in ENVIRONMENT {
        if *kind == Kind::User && !help.contains(&format!("\"{name}\"")) {
            problems.push(format!(
                "{name} is user-facing, but pcb-maker --help (ENVIRONMENT_HELP in src/main.rs) does not list it"
            ));
        }
    }
    Ok(problems)
}

/// Every generated file, relative to `root`, with its text.
pub fn generate(root: &Path) -> Result<Vec<(String, String)>, String> {
    Ok(vec![
        (REFERENCE.to_string(), reference(root)?.0),
        (HANDOVER.to_string(), handover(root)?),
    ])
}

/// The repository root (this crate lives in tools/config-reference).
pub fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_struct_with_attributes_and_long_types_parses() {
        let source = "\
pub struct Example {
    /// First line,
    /// second line.
    #[serde(default)]
    pub alpha: Option<f64>,
    #[serde(skip)]
    pub beta: Option<(Measure, (usize, usize))>,
    // A plain comment.
    pub gamma: BTreeMap<String, f64>,
}

impl Default for Example {
    fn default() -> Self {
        Self {
            alpha: None,
            beta: std::env::var_os(\"PCB_X\").is_some(),
            gamma: BTreeMap::new(),
        }
    }
}
";
        let fields = parse_struct(source, "Example").unwrap();
        let names: Vec<&str> = fields.iter().map(|field| field.name.as_str()).collect();
        assert_eq!(names, ["alpha", "beta", "gamma"]);
        assert_eq!(fields[0].doc, "First line, second line.");
        assert!(fields[0].serde_default && !fields[0].skip);
        assert!(fields[1].skip);
        assert_eq!(fields[1].ty, "Option<(Measure, (usize, usize))>");
        assert_eq!(
            fields[1].default_expression.as_deref(),
            Some("std::env::var_os(\"PCB_X\").is_some()")
        );
        let debug = BTreeMap::new();
        assert_eq!(
            shown_default(&fields[1], &Defaults::Entry, &debug),
            "`false` (`true` with `PCB_X`)"
        );
    }

    #[test]
    fn debug_prints_flatten_to_one_line_per_field() {
        let debug = "Config {\n    pitches: [\n        0.127,\n        0.1,\n    ],\n    seed: None,\n    work: Some(\n        3.0,\n    ),\n}";
        let fields = debug_fields(debug);
        assert_eq!(fields["pitches"], "[0.127, 0.1]");
        assert_eq!(fields["seed"], "None");
        assert_eq!(fields["work"], "Some(3.0)");
    }

    #[test]
    fn assignments_follow_if_let_keys_and_literals() {
        let source = "\
fn core_config(config: &Settings) -> core::Config {
    let mut router_config = core::Config {
        verbose: true,
        deadline: config.deadline,
        ..core::Config::default()
    };
    if let Some(cost) = config.via_cost_mm {
        router_config.via_cost = cost;
    }
    router_config.work = config
        .work_seconds
        .map(|seconds| seconds * RATE);
    if let Some(seconds) = config.negotiation_seconds {
        // config.not_this one
        router_config.expansions = (seconds * RATE) as u64;
    } else {
        router_config.expansions = 3;
    }
    router_config.run_work = config.run_work;
    router_config
}
";
        let found = assignments(source, "fn core_config(", "router_config").unwrap();
        let keys = |field: &str| found[field].keys.iter().cloned().collect::<Vec<_>>();
        assert_eq!(keys("deadline"), ["deadline"]);
        assert!(found["verbose"].computed && found["verbose"].keys.is_empty());
        assert_eq!(keys("via_cost"), ["via_cost_mm"]);
        assert_eq!(keys("expansions"), ["negotiation_seconds"]);
        assert_eq!(keys("run_work"), ["run_work"]);
        assert_eq!(keys("work"), ["work_seconds"]);
    }

    #[test]
    fn code_identifiers_come_from_code_spans_and_blocks_only() {
        let names = code_identifiers("A `seeds` key, not seeds.\n```\n{\"tighten\": true}\n```\n");
        assert!(names.contains("seeds") && names.contains("tighten"));
        assert!(!names.contains("not"));
    }
}
