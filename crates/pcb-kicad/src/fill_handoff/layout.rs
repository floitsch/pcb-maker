// Copyright (C) 2026 Florian Loitsch.

//! Publish KiCad's fill data on the selected layout board.

mod layer_connections;

use super::*;
use pcb_kicad_check::net_witness::net_witness_neutral;
use pcb_kicad_check::verified_fill_cache::VerifiedFillCache;
use pcb_kicad_file::board_stats::is_rule_area;
pub(crate) use pcb_kicad_file::fs_util::project_companion_hashes as project_inputs;
use pcb_kicad_file::kicad_nets::{item_net, net_codes};
use pcb_kicad_file::kicad_sexpr::{Expr, encode, form_atom, parse};
use pcb_kicad_route::board_router::LayerTable;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::time::Instant;

#[derive(Clone, Debug, Default, Serialize)]
pub struct FillPublication {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_board_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_board_sha256: Option<String>,
    pub filled_zones: usize,
    pub filled_polygons: usize,
    pub native_checks: usize,
    pub native_checks_charged: usize,
    pub connectivity_checks: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connectivity_proof: Option<FillConnectivityProof>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_status: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refill_source: Option<&'static str>,
    pub wall_seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_as_stored: Option<FillDrcCounts>,
    /// Standalone verified fill handoff when the layout's work budget expires.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub retry_argv: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CachedFill {
    key: String,
    source_hash: String,
    board: String,
    board_hash: String,
    drc: String,
    drc_hash: String,
}

impl CachedFill {
    fn decode(bytes: &[u8], key: &str, source_hash: &str) -> Result<Self, String> {
        let entry: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        if entry.key != key
            || entry.source_hash != source_hash
            || sha256(entry.board.as_bytes()) != entry.board_hash
            || sha256(entry.drc.as_bytes()) != entry.drc_hash
        {
            return Err("cached fill content does not match its input or checksums".into());
        }
        Ok(entry)
    }

    fn check_unchanged_items(&self, original: &Expr) -> Result<(usize, usize), String> {
        let cached_board = parse(&self.board)?;
        let mut current = original.clone();
        let count = copy_fill_with_native_teardrops(&mut current, cached_board)?;
        if format!("{}\n", encode(&current)) != self.board {
            return Err("cached fill changes content outside the current board's fill".into());
        }
        Ok(count)
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct SavedFill {
    board: String,
    board_hash: String,
    drc: String,
    findings: FillDrcCounts,
    zones: usize,
    polygons: usize,
}

impl SavedFill {
    fn load(
        cache: &VerifiedFillCache,
        bytes: &[u8],
        board: &Path,
        original: &Expr,
        source_hash: &str,
    ) -> Result<Self, String> {
        let entry = CachedFill::decode(bytes, cache.key(), source_hash)?;
        let report = cache.validate_stored_report(entry.drc.as_bytes(), board)?;
        let findings = counts(&report);
        check_finding_caps(&report, &findings, original)?;
        let (zones, polygons) = entry.check_unchanged_items(original)?;
        Ok(Self {
            board: entry.board,
            board_hash: entry.board_hash,
            drc: entry.drc,
            findings,
            zones,
            polygons,
        })
    }

    fn cache_entry(&self, key: &str, source_hash: &str) -> Option<Vec<u8>> {
        serde_json::to_vec(&CachedFill {
            key: key.into(),
            source_hash: source_hash.into(),
            board: self.board.clone(),
            board_hash: self.board_hash.clone(),
            drc_hash: sha256(self.drc.as_bytes()),
            drc: self.drc.clone(),
        })
        .ok()
    }

    fn publish(self, board: &Path, report: &mut FillPublication) -> Result<(), String> {
        let stored_report = board.with_file_name("drc-saved-fill.json");
        fs_util::write_atomically(&stored_report, self.drc.as_bytes())?;
        if let Err(error) = fs_util::write_atomically(board, self.board.as_bytes()) {
            let _ = fs::remove_file(stored_report);
            return Err(error);
        }
        report.status = "saved";
        report.saved_board_sha256 = Some(self.board_hash);
        report.filled_zones = self.zones;
        report.filled_polygons = self.polygons;
        report.saved_as_stored = Some(self.findings);
        Ok(())
    }
}

impl FillPublication {
    pub(crate) fn skipped(reason: impl Into<String>) -> Self {
        Self {
            status: "skipped",
            reason: Some(reason.into()),
            ..Self::default()
        }
    }
}

fn layer_names(zone: &Expr) -> impl Iterator<Item = &str> {
    let layer = zone.child("layers").or_else(|| zone.child("layer"));
    layer
        .into_iter()
        .flat_map(|layer| layer.children().iter().skip(1).filter_map(Expr::atom))
}

fn semantic_layers<'a>(zone: &'a Expr, table: &'a LayerTable) -> Result<BTreeSet<&'a str>, String> {
    let mut names = BTreeSet::new();
    for name in layer_names(zone) {
        match name {
            "F&B.Cu" => names.extend(["F.Cu", "B.Cu"]),
            "*.Cu" => names.extend(table.names.iter().map(String::as_str)),
            name => {
                if name.ends_with(".Cu") && table.index(name).is_none() {
                    return Err(format!("zone names inactive copper layer {name}"));
                }
                names.insert(name);
            }
        }
    }
    Ok(names)
}

fn fillable(zone: &Expr) -> bool {
    zone.head() == Some("zone")
        && !is_rule_area(zone)
        && form_atom(zone, "fill", 1) != Some("no")
        && layer_names(zone).any(|layer| layer.ends_with(".Cu"))
}

fn zones(board: &Expr) -> impl Iterator<Item = &Expr> {
    board.children().iter().chain(
        board
            .children()
            .iter()
            .filter(|part| matches!(part.head(), Some("footprint" | "module")))
            .flat_map(Expr::children),
    )
}

pub(crate) fn has_fillable_zone(board: &Expr) -> bool {
    zones(board).any(fillable)
}

/// These two KiCad 10.0 providers inspect unchanged footprint-library
/// inputs and non-copper artwork. The copied fill is copper only. The
/// full reported finding multiset is still compared before publication.
fn check_finding_caps(
    report: &Value,
    findings: &FillDrcCounts,
    original: &Expr,
) -> Result<(), String> {
    for (field, kinds) in &findings.possibly_capped_types {
        for kind in kinds {
            let static_warning = *field == "violations"
                && report["kicad_version"]
                    .as_str()
                    .is_some_and(|version| version.starts_with("10.0."))
                && match kind.as_str() {
                    "lib_footprint_issues" => true,
                    "silk_overlap" => zones(original)
                        .filter(|zone| zone.head() == Some("zone") && !is_rule_area(zone))
                        .all(|zone| {
                            let mut layers = layer_names(zone).peekable();
                            layers.peek().is_some() && layers.all(|layer| layer.ends_with(".Cu"))
                        }),
                    _ => false,
                }
                && report["violations"]
                    .as_array()
                    .expect("validated findings")
                    .iter()
                    .filter(|finding| finding["type"] == kind.as_str())
                    .all(|finding| finding["severity"] == "warning" && finding["excluded"] != true);
            if !static_warning {
                return Err(format!(
                    "KiCad's {field} report may have reached a finding cap for {kind}"
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn has_fillable_zones(board: &Expr) -> bool {
    zones(board).any(fillable)
}

fn zone_id(zone: &Expr) -> Result<&str, String> {
    form_atom(zone, "uuid", 1)
        .or_else(|| form_atom(zone, "tstamp", 1))
        .ok_or_else(|| "a fillable zone has no stable native UUID".into())
}

fn outline(zone: &Expr) -> impl Iterator<Item = &Expr> {
    zone.children()
        .iter()
        .filter(|part| part.head() == Some("polygon"))
}

fn coordinate_nm(expr: &Expr) -> Option<i32> {
    let Expr::Atom(value) = expr else {
        return None;
    };
    let nm = (value.parse::<f64>().ok()? * 1_000_000.0).round();
    (f64::from(i32::MIN)..=f64::from(i32::MAX))
        .contains(&nm)
        .then_some(nm as i32)
}

fn outline_eq(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::List(xs), Expr::List(ys)) => {
            // KiCad reads and saves vertices in integer nanometres. Compare
            // only coordinate pairs on that lattice and keep the authored
            // polygon itself when copying the fill.
            if let ([head, ax, ay], [other, bx, by]) = (xs.as_slice(), ys.as_slice())
                && head == other
                && matches!(head.atom(), Some("xy" | "start" | "mid" | "end"))
            {
                return coordinate_nm(ax).is_some_and(|x| Some(x) == coordinate_nm(bx))
                    && coordinate_nm(ay).is_some_and(|y| Some(y) == coordinate_nm(by));
            }
            xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| outline_eq(x, y))
        }
        _ => a == b,
    }
}

fn fill_payload(item: &Expr) -> bool {
    matches!(item.head(), Some("filled_polygon" | "fill_segments"))
}

pub(crate) fn native_pad_via_teardrop(zone: &Expr) -> bool {
    fillable(zone)
        && zone.child("uuid").is_none()
        && zone.child("tstamp").is_none()
        && zone.child("layers").is_none()
        && form_atom(zone, "layer", 1).is_some_and(|layer| !layer.contains('*'))
        && form_atom(zone, "fill", 1) == Some("yes")
        && zone
            .child("attr")
            .and_then(|attr| attr.child("teardrop"))
            .is_some_and(|teardrop| form_atom(teardrop, "type", 1) == Some("padvia"))
}

pub(crate) fn copy_fill_with_native_teardrops(
    original: &mut Expr,
    mut native: Expr,
) -> Result<(usize, usize), String> {
    let Expr::List(native_items) = &mut native else {
        return Err("native PCB root is not a list".into());
    };
    // KiCad rebuilds these UUID-free pad/via teardrops during each refill.
    // Treat only those explicitly tagged native forms as derived copper.
    let teardrops: Vec<_> = native_items
        .extract_if(.., |zone| native_pad_via_teardrop(zone))
        .collect();
    let Expr::List(items) = original else {
        return Err("PCB root is not a list".into());
    };
    items.retain(|zone| !native_pad_via_teardrop(zone));
    let (mut zones, mut polygons) = copy_identity_matched_fill(original, &native)?;
    let Expr::List(items) = original else {
        unreachable!("copy_fill_payload retains the PCB root");
    };
    for teardrop in &teardrops {
        let count = teardrop
            .children()
            .iter()
            .filter(|part| fill_payload(part))
            .count();
        zones += usize::from(count > 0);
        polygons += count;
    }
    items.extend(teardrops);
    nonempty_fill((zones, polygons))
}

fn apply_zone_payload(zone: &mut Expr, filled: &Expr) -> Result<usize, String> {
    let polygons = filled
        .children()
        .iter()
        .filter(|part| part.head() == Some("filled_polygon"))
        .count();
    let filled_marker = form_atom(filled, "fill", 1) == Some("yes");
    if polygons > 0 && !filled_marker {
        return Err("KiCad saved zone polygons without its filled marker".into());
    }
    let Expr::List(parts) = zone else {
        unreachable!("zone is a list");
    };
    parts.retain(|part| !fill_payload(part));
    if filled_marker {
        if let Some(Expr::List(fill)) = parts.iter_mut().find(|part| part.head() == Some("fill")) {
            if fill.get(1).and_then(Expr::atom) != Some("yes") {
                fill.insert(1, Expr::Atom("yes".into()));
            }
        } else {
            parts.push(Expr::List(vec![
                Expr::Atom("fill".into()),
                Expr::Atom("yes".into()),
            ]));
        }
    }
    parts.extend(
        filled
            .children()
            .iter()
            .filter(|part| fill_payload(part))
            .cloned(),
    );
    Ok(polygons)
}

pub(crate) fn copy_fill_payload(
    original: &mut Expr,
    native: &Expr,
) -> Result<(usize, usize), String> {
    nonempty_fill(copy_identity_matched_fill(original, native)?)
}

fn nonempty_fill(copied: (usize, usize)) -> Result<(usize, usize), String> {
    if copied.1 == 0 {
        return Err("KiCad's refill produced no zone copper polygons".into());
    }
    Ok(copied)
}

fn copy_identity_matched_fill(
    original: &mut Expr,
    native: &Expr,
) -> Result<(usize, usize), String> {
    let mut fills = BTreeMap::new();
    for zone in zones(native).filter(|zone| fillable(zone)) {
        let id = zone_id(zone)?;
        if fills.insert(id, zone).is_some() {
            return Err(format!("KiCad saved more than one zone with UUID {id}"));
        }
    }
    let old_nets = net_codes(original);
    let new_nets = net_codes(native);
    let old_stack = LayerTable::from_pcb(original)?;
    let new_stack = LayerTable::from_pcb(native)?;
    if old_stack.names == new_stack.names {
        layer_connections::copy(original, native, &old_nets, &new_nets, &old_stack);
    }
    let mut copied = (0, 0);
    let mut apply = |zone: &mut Expr| -> Result<(), String> {
        if !fillable(zone) {
            return Ok(());
        }
        let id = zone_id(zone)?;
        let filled = fills
            .remove(id)
            .ok_or_else(|| format!("no unambiguous native refill for zone {id}"))?;
        let old_layers = semantic_layers(zone, &old_stack)?;
        let new_layers = semantic_layers(filled, &new_stack)?;
        let old_outline: Vec<_> = outline(zone).collect();
        let new_outline: Vec<_> = outline(filled).collect();
        if old_layers != new_layers
            || item_net(zone, &old_nets) != item_net(filled, &new_nets)
            || old_outline.is_empty()
            || old_outline.len() != new_outline.len()
            || !old_outline
                .iter()
                .zip(new_outline)
                .all(|(a, b)| outline_eq(a, b))
        {
            return Err(format!(
                "KiCad changed zone {id}'s net, layers or outline while saving"
            ));
        }
        let polygons = apply_zone_payload(zone, filled)?;
        copied.0 += usize::from(polygons > 0);
        copied.1 += polygons;
        Ok(())
    };
    let Expr::List(items) = original else {
        return Err("PCB root is not a list".into());
    };
    for item in items {
        if matches!(item.head(), Some("footprint" | "module")) {
            if let Expr::List(parts) = item {
                for child in parts {
                    apply(child)?;
                }
            }
        } else {
            apply(item)?;
        }
    }
    if !fills.is_empty() {
        return Err("KiCad added fillable zones that have no original board identity".into());
    }
    Ok(copied)
}

pub(crate) enum FillRefillEvidence<'a> {
    Layout(Option<&'a pcb_kicad_check::verification::CheckedRefill>),
    CheckedLayout(&'a pcb_kicad_check::verification::CheckedRefill),
    AcceptedRepair(&'a Value),
    GuardedTrial(&'a dyn Fn(&Value) -> Result<(), String>),
    CompletedRoute(&'a Value),
}

/// Work only on a private sibling. The caller charges each native check
/// and receives a usable layout even when this optional handoff fails.
pub(crate) fn publish_layout_fill(
    project: &Path,
    board_id: &str,
    mut original: Expr,
    source_hash: String,
    evidence: FillRefillEvidence<'_>,
    allow_connectivity_proof: bool,
    wall_passed: impl Fn() -> bool,
) -> FillPublication {
    let started = Instant::now();
    let stage = fs_util::temporary_next_to(project);
    let mut report = FillPublication {
        status: "skipped",
        source_board_sha256: Some(source_hash.clone()),
        ..FillPublication::default()
    };
    let publication = (|| {
        let context = project_inputs(project)?;
        let board = project.join(format!("{board_id}.kicad_pcb"));
        let current_input = || -> Result<bool, String> {
            Ok(project_inputs(project)? == context && fs_util::file_sha256(&board)? == source_hash)
        };
        let checked_layout = matches!(evidence, FillRefillEvidence::CheckedLayout(_));
        let (reused_refill, expected_report, guarded, trial_gate) = match evidence {
            FillRefillEvidence::Layout(refill) => (refill, None, false, None),
            FillRefillEvidence::CheckedLayout(refill) => (Some(refill), None, true, None),
            FillRefillEvidence::AcceptedRepair(report) => (None, Some(report), false, None),
            FillRefillEvidence::GuardedTrial(gate) => (None, None, true, Some(gate)),
            FillRefillEvidence::CompletedRoute(report) => (None, Some(report), true, None),
        };
        let cache = (!checked_layout && expected_report.is_none() && trial_gate.is_none())
            .then(|| VerifiedFillCache::for_board(&board))
            .flatten();
        report.cache_status = Some(if cache.is_some() { "miss" } else { "bypassed" });
        if let Some(cache) = &cache
            && let Some(bytes) = cache.lookup()
        {
            match SavedFill::load(cache, &bytes, &board, &original, &source_hash) {
                Ok(saved) => {
                    if !current_input()? {
                        return Err(
                            "the selected board changed while checking its cached fill".into()
                        );
                    }
                    if cache.matches_current(&board) {
                        saved.publish(&board, &mut report)?;
                        report.cache_status = Some("hit");
                        report.refill_source = Some("verified_fill_cache");
                        report.native_checks_charged = 2;
                        return Ok(());
                    }
                }
                Err(_) => cache.discard(),
            }
        }
        fs_util::copy_directory_tree(project, &stage)?;
        let staged = stage.join(format!("{board_id}.kicad_pcb"));
        if project_inputs(&stage)? != context || fs_util::file_sha256(&staged)? != source_hash {
            return Err("the selected board changed while staging the refill".into());
        }
        let parity =
            staged.with_extension("kicad_sch").is_file() || staged.with_extension("sch").is_file();
        if wall_passed() {
            return Err("the wall-clock guard expired before filling".into());
        }
        let reused_refill = reused_refill.filter(|refill| {
            refill.matches(&board) && (!checked_layout || refill.matches(&staged))
        });
        if checked_layout && reused_refill.is_none() {
            return Err(
                "the selected board, report or project no longer matches its exact layout refill"
                    .into(),
            );
        }
        // The ordinary layout already booked its captured refill. Retain
        // the established accounting on the independent reuse/cache paths.
        report.native_checks_charged += usize::from(!checked_layout);
        let (refilled, native) = if let Some(refill) = reused_refill {
            report.refill_source = Some(if checked_layout {
                "fresh_layout_drc"
            } else {
                "fresh_route_reuse_drc"
            });
            (refill.drc().clone(), parse(refill.board())?)
        } else {
            report.refill_source = Some("fresh_fill_drc");
            report.native_checks += 1;
            let report_path = stage.join("drc-refilled-for-publication.json");
            let refilled = if guarded {
                let report = native_report::save_kept_route_fill(
                    &staged,
                    &report_path,
                    parity,
                    &wall_passed,
                )?;
                native_report::validate_drc_report(&report, &staged)?;
                report
            } else {
                native_report::run_fill_handoff_drc(&staged, &report_path, parity, true)?
            };
            let native = parse(&fs::read_to_string(&staged).map_err(|error| error.to_string())?)?;
            (refilled, native)
        };
        check_finding_caps(&refilled, &counts(&refilled), &original)?;
        if let Some(gate) = trial_gate {
            gate(&refilled)?;
        }
        // Compared without the items of one-net findings (`net_witness`):
        // which of a joint's tracks KiCad names follows the checked path.
        let expected_report =
            expected_report.map(|expected| net_witness_neutral(&original, expected));
        let refilled = net_witness_neutral(&original, &refilled);
        if guarded && let Some(expected) = &expected_report {
            compare_reports(expected, expected, &refilled).map_err(|error| {
                format!("the staged refill differs from the accepted native report: {error}")
            })?;
        }
        let (zones, polygons) = copy_fill_with_native_teardrops(&mut original, native)?;
        let text = format!("{}\n", encode(&original));
        fs_util::write_atomically(&staged, text.as_bytes())?;
        let saved_hash = fs_util::file_sha256(&staged)?;
        if project_inputs(&stage)? != context {
            return Err(
                "KiCad changed staged project rules or schematic while saving its fill".into(),
            );
        }
        if wall_passed() {
            return Err("the wall-clock guard expired before reopening the saved fill".into());
        }
        report.native_checks += 1;
        report.native_checks_charged += 1;
        let stored_path = stage.join("drc-saved-fill.json");
        let stored = if guarded {
            let report = native_report::check_kept_route_input(
                &staged,
                &stored_path,
                parity,
                false,
                &wall_passed,
            )?;
            native_report::validate_drc_report(&report, &staged)?;
            report
        } else {
            native_report::run_fill_handoff_drc(&staged, &stored_path, parity, false)?
        };
        let findings = counts(&stored);
        check_finding_caps(&stored, &findings, &original)?;
        let stored = net_witness_neutral(&original, &stored);
        if let Some(expected) = &expected_report {
            compare_reports(expected, expected, &stored).map_err(|error| {
                format!("the saved board differs from its accepted refill: {error}")
            })?;
        }
        if let Err(error) = compare_reports(&refilled, &refilled, &stored) {
            if !connectivity::eligible(&refilled, &refilled, &stored) {
                return Err(explain_cap(error, &refilled, &stored));
            }
            if !allow_connectivity_proof {
                return Err(format!(
                    "{error}. The layout has insufficient work for two native connectivity checks."
                ));
            }
            let original_bytes = fs::read(&board).map_err(|error| error.to_string())?;
            if sha256(&original_bytes) != source_hash {
                return Err("the selected layout changed before checking connectivity".into());
            }
            let proof = connectivity::prove(
                &staged,
                &original_bytes,
                &refilled,
                &stored,
                &mut report.connectivity_checks,
                &wall_passed,
                None,
            );
            report.native_checks_charged += report.connectivity_checks;
            report.connectivity_proof =
                Some(proof.map_err(|why| format!("{error}. Connectivity proof failed: {why}"))?);
        }
        if project_inputs(&stage)? != context
            || fs_util::file_sha256(&staged)? != saved_hash
            || !current_input()?
            || reused_refill.is_some_and(|refill| !refill.matches(&board))
        {
            return Err("the saved board or its project changed during fill verification".into());
        }
        let saved = SavedFill {
            board: text,
            board_hash: saved_hash,
            drc: fs::read_to_string(stored_path).map_err(|error| error.to_string())?,
            findings,
            zones,
            polygons,
        };
        if let Some(cache) = cache
            && report.connectivity_proof.is_none()
            && cache.matches_current(&board)
            && let Some(entry) = saved.cache_entry(cache.key(), &source_hash)
        {
            cache.store(&entry);
        }
        saved.publish(&board, &mut report)
    })();
    let _ = fs::remove_dir_all(stage);
    report.reason = publication.err();
    report.wall_seconds = started.elapsed().as_secs_f64();
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOARD: &str = r#"(kicad_pcb
        (generator "pcb-maker")
        (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
        (net 1 "GND")
        (segment (start 1 1) (end 4 1) (width 0.3) (layer "F.Cu") (net 1) (uuid "track"))
        (zone (net 1) (layer "F.Cu") (uuid "plane") (priority 2)
            (fill yes (thermal_gap 0.4))
            (polygon (pts (xy 0 0) (xy 5.0 0) (xy 5 5))))
        (zone (layer "F.Cu") (uuid "rule-area") (keepout (tracks not_allowed))
            (polygon (pts (xy 2 2) (xy 3 2) (xy 3 3)))))"#;
    const FILL: &str = r#"(filled_polygon (layer "F.Cu")
        (pts (xy 0.1 0.1) (xy 4.9 0.1) (xy 4.9 4.9)))"#;

    fn native(outline: &str) -> Expr {
        parse(&format!(
            r#"(kicad_pcb
                (generator "pcbnew")
                (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
                (segment (start 1 1) (end 2 2) (layer "B.Cu") (net "GND"))
                (zone (net "GND") (layer "F.Cu") (uuid "plane")
                    (fill yes (thermal_gap 0.8))
                    (polygon (pts {outline})) {FILL}))"#
        ))
        .unwrap()
    }

    #[test]
    fn a_layout_copies_only_fill_data_from_a_native_save() {
        let original = parse(BOARD).unwrap();
        assert!(has_fillable_zones(&original));
        let mut saved = original.clone();
        let refill = native("(xy 0 0) (xy 5 0) (xy 5.00 5)");
        assert_eq!(copy_fill_payload(&mut saved, &refill).unwrap(), (1, 1));
        let mut removed_payload = saved.clone();
        let Expr::List(items) = &mut removed_payload else {
            unreachable!();
        };
        for item in items {
            if fillable(item)
                && let Expr::List(parts) = item
            {
                parts.retain(|part| !fill_payload(part));
            }
        }
        assert_eq!(removed_payload, original);
    }

    #[test]
    fn native_fill_replaces_only_uuid_free_pad_via_teardrops() {
        let teardrop = |x: f64| {
            parse(&format!(
                r#"(zone (net "GND") (layer "F.Cu") (attr (teardrop (type padvia)))
                    (fill yes) (polygon (pts (xy {x} 0) (xy 3 0) (xy 3 1))) {FILL})"#
            ))
            .unwrap()
        };
        let mut original = parse(BOARD).unwrap();
        let Expr::List(items) = &mut original else {
            unreachable!()
        };
        items.push(teardrop(1.0));
        let scratch_native = || {
            let mut refill = native("(xy 0 0) (xy 5 0) (xy 5 5)");
            let Expr::List(items) = &mut refill else {
                unreachable!()
            };
            items.push(teardrop(2.0));
            refill
        };
        assert!(copy_fill_payload(&mut original.clone(), &scratch_native()).is_err());
        let mut unknown = scratch_native();
        let Expr::List(items) = &mut unknown else {
            unreachable!()
        };
        items.push(parse(r#"(zone (layer "F.Cu") (fill yes))"#).unwrap());
        assert!(copy_fill_with_native_teardrops(&mut original.clone(), unknown).is_err());
        let no_ordinary_pour = |mut board: Expr| {
            let Expr::List(items) = &mut board else {
                unreachable!()
            };
            items.retain(|item| !fillable(item) || native_pad_via_teardrop(item));
            board
        };
        assert_eq!(
            copy_fill_with_native_teardrops(
                &mut no_ordinary_pour(original.clone()),
                no_ordinary_pour(scratch_native())
            )
            .unwrap(),
            (1, 1)
        );
        assert_eq!(
            copy_fill_with_native_teardrops(&mut original, scratch_native()).unwrap(),
            (2, 2)
        );
        let generated: Vec<_> = original
            .children()
            .iter()
            .filter(|zone| native_pad_via_teardrop(zone))
            .collect();
        assert_eq!(generated, vec![&teardrop(2.0)]);
        assert!(
            original
                .children()
                .iter()
                .any(|item| form_atom(item, "uuid", 1) == Some("rule-area"))
        );
    }

    const FLASH_PAD: &str = r#"(pad "13" thru_hole oval (at -2.5 -2.15 90) (size 1 1.7)
        (drill oval 0.6 1.3) (layers "*.Cu" "*.Mask") (remove_unused_layers yes)
        (keep_end_layers yes) (zone_layer_connections "In2.Cu") (zone_connect 3)
        (uuid "pad") (net "GND"))"#;

    const FLASH_VIA: &str = r#"(via blind (at 3.1234561 4) (size 0.6) (drill 0.3)
        (layers "F.Cu" "In1.Cu") (remove_unused_layers) (net 1) (uuid "via"))"#;

    fn flash_board(pad: &str, via: &str) -> String {
        BOARD
            .replace(
                r#"(2 "B.Cu" signal)"#,
                r#"(4 "In1.Cu" signal) (6 "In2.Cu" signal) (2 "B.Cu" signal)"#,
            )
            .replace(
                "(net 1 \"GND\")",
                &format!(
                    r#"(net 1 "GND") (footprint "connector" (layer "F.Cu")
                        (at 3 4 0) (uuid "connector") {pad}) {via}"#,
                ),
            )
    }

    fn flash_native(pad: &str, via: &str) -> Expr {
        parse(
            &flash_board(pad, via)
                .replace("(at 3 4 0)", "(at 3 4)")
                .replace(
                    "(fill yes (thermal_gap 0.4))",
                    &format!("(fill yes) {FILL}"),
                ),
        )
        .unwrap()
    }

    #[test]
    fn layout_copies_explicit_empty_and_added_pad_and_via_layer_connections() {
        let board = flash_board(FLASH_PAD, FLASH_VIA);
        let mut saved = parse(&board).unwrap();
        let pad = FLASH_PAD.replace(
            r#"(zone_layer_connections "In2.Cu")"#,
            "(zone_layer_connections)",
        );
        let via = FLASH_VIA
            .replace(
                "(remove_unused_layers)",
                r#"(remove_unused_layers yes) (zone_layer_connections "In1.Cu")"#,
            )
            .replace("(at 3.1234561 4)", "(at 3.123456 4)")
            .replace("(net 1)", r#"(net "GND")"#);
        copy_fill_payload(&mut saved, &flash_native(&pad, &via)).unwrap();

        let expected = board
            .replace(FLASH_PAD, &pad)
            .replace(
                "(remove_unused_layers) (net 1)",
                r#"(remove_unused_layers) (zone_layer_connections "In1.Cu") (net 1)"#,
            )
            .replace("(xy 5 5))))", &format!("(xy 5 5))) {FILL})"));
        assert_eq!(saved, parse(&expected).unwrap());

        let stored = format!("{}\n", encode(&saved));
        let mut entry = CachedFill {
            key: "current key".into(),
            source_hash: sha256(board.as_bytes()),
            board_hash: sha256(stored.as_bytes()),
            board: stored,
            drc: "{}".into(),
            drc_hash: sha256(b"{}"),
        };
        let original = parse(&board).unwrap();
        assert_eq!(entry.check_unchanged_items(&original).unwrap(), (1, 1));
        entry.board = entry.board.replace("(drill 0.3)", "(drill 0.2)");
        entry.board_hash = sha256(entry.board.as_bytes());
        assert!(entry.check_unchanged_items(&original).is_err());
    }

    #[test]
    fn layer_connections_require_an_unambiguous_matching_plated_item() {
        let original = parse(&flash_board(FLASH_PAD, FLASH_VIA)).unwrap();
        let pad = FLASH_PAD.replace("In2.Cu", "In1.Cu");
        let via = FLASH_VIA.replace("(net 1)", r#"(zone_layer_connections "In1.Cu") (net 1)"#);
        let refill = flash_native(&pad, &via);
        let baseline = encode(&refill);
        for (from, to) in [
            (r#"(uuid "pad")"#, r#"(uuid "another-pad")"#),
            (r#"(uuid "pad")"#, r#"(uuid "pad") (tstamp "pad")"#),
            (r#"(uuid "connector")"#, r#"(uuid "other-connector")"#),
            ("(at 3 4)", "(at 3.1 4)"),
            ("(at -2.5 -2.15 90)", "(at -2.5 -2.15 91)"),
            ("(size 1 1.7)", "(size 1 1.8)"),
            ("(keep_end_layers yes)", "(keep_end_layers no)"),
            ("thru_hole oval", "np_thru_hole oval"),
            (
                r#"(zone_layer_connections "In1.Cu")"#,
                r#"(zone_layer_connections "In1.Cu" "In1.Cu")"#,
            ),
            (
                r#"(zone_layer_connections "In1.Cu")"#,
                r#"(zone_layer_connections "F.Mask")"#,
            ),
            (r#"(zone_layer_connections "In1.Cu")"#, ""),
        ] {
            let mut saved = original.clone();
            let altered = parse(&baseline.replace(from, to)).unwrap();
            copy_fill_payload(&mut saved, &altered).unwrap();
            assert_eq!(
                saved.child("footprint"),
                original.child("footprint"),
                "{to}"
            );
        }
        for changed_via in [
            via.replace(
                r#"(zone_layer_connections "In1.Cu")"#,
                r#"(zone_layer_connections "In2.Cu")"#,
            ),
            via.replace("(drill 0.3)", "(drill 0.2)"),
            via.replace("(net 1)", r#"(net "/GND")"#),
            format!("{via} {via}"),
        ] {
            let mut saved = original.clone();
            copy_fill_payload(&mut saved, &flash_native(&pad, &changed_via)).unwrap();
            assert_eq!(saved.child("via"), original.child("via"), "{changed_via}");
        }
    }

    #[test]
    fn a_native_save_may_expand_copper_layer_aliases_without_changing_zone_scope() {
        let two_layers = r#"(layers (0 "F.Cu" signal) (2 "B.Cu" signal))"#;
        let four_layers = r#"(layers (0 "F.Cu" signal) (4 "In1.Cu" signal)
            (6 "In2.Cu" signal) (2 "B.Cu" signal))"#;
        let single = r#"(zone (net 1) (layer "F.Cu")"#;
        let mk_original = |alias| {
            parse(
                &BOARD
                    .replace(single, &format!(r#"(zone (net 1) (layers "{alias}")"#))
                    .replace(two_layers, four_layers),
            )
            .unwrap()
        };
        let mk_native = |layers: &str, members| {
            parse(&format!(
                r#"(kicad_pcb {layers}
                    (zone (net "GND") (layers {members}) (uuid "plane") (fill yes)
                    (polygon (pts (xy 0 0) (xy 5 0) (xy 5 5))) {FILL}))"#
            ))
            .unwrap()
        };
        let mut outer = mk_original("F&B.Cu");
        let external = mk_native(four_layers, r#""B.Cu" "F.Cu""#);
        assert_eq!(copy_fill_payload(&mut outer, &external).unwrap(), (1, 1));
        assert_eq!(
            layer_names(outer.child("zone").unwrap()).collect::<Vec<_>>(),
            ["F&B.Cu"]
        );
        let mut all = mk_original("*.Cu");
        let explicit = mk_native(four_layers, r#""F.Cu" "B.Cu" "In1.Cu" "In2.Cu""#);
        assert_eq!(copy_fill_payload(&mut all, &explicit).unwrap(), (1, 1));
        for changed in [
            external,
            mk_native(two_layers, r#""*.Cu""#),
            mk_native(four_layers, r#""F.Cu" "B.Cu" "In1.Cu" "In3.Cu""#),
            mk_native(
                four_layers,
                r#""F.Cu" "B.Cu" "In1.Cu" "In2.Cu" "Eco1.User""#,
            ),
        ] {
            assert!(copy_fill_payload(&mut mk_original("*.Cu"), &changed).is_err());
        }
    }

    #[test]
    fn a_native_round_trip_matches_the_zone_vertices_at_exact_nanometres() {
        let authored = "(xy 135.62263812943408 97.65385958952493)";
        let rounded = "(xy 135.622638 97.65386)";
        let outline = format!("(xy 0 0) (xy 5 0) {rounded}");
        let mut board = parse(&BOARD.replace("(xy 5 5)", authored)).unwrap();
        let original = board.clone();
        let refill = native(&outline);
        assert_eq!(copy_fill_payload(&mut board, &refill).unwrap(), (1, 1));
        let before = original.child("zone").unwrap().child("polygon").unwrap();
        let after = board.child("zone").unwrap().child("polygon").unwrap();
        assert_eq!(before, after);
        assert!(encode(after).contains(authored));

        for changed in [
            "(xy 135.622639 97.65386)",
            "(xy 135.622638 97.653859)",
            "(xy NaN 97.65386)",
            "(xy 135.622638 3000.0)",
            "(xy \"135.622638\" 97.65386)",
        ] {
            let refill = native(&outline.replace(rounded, changed));
            assert!(
                copy_fill_payload(&mut original.clone(), &refill).is_err(),
                "{changed}"
            );
        }
    }

    #[test]
    fn a_fill_only_normalizes_coordinate_atoms() {
        let parse_form = |form| parse(form).unwrap();
        let a = parse_form("(polygon (pts (arc (start 1.0000001 2) (mid 3 4) (end 5 6))) (tag 1))");
        let b = parse_form("(polygon (pts (arc (start 1 2) (mid 3 4.0) (end 5 6))) (tag 1))");
        let c = parse_form("(polygon (pts (arc (start 1 2) (mid 3 4.0) (end 5 6))) (tag 1.0))");
        assert!(outline_eq(&a, &b));
        assert!(!outline_eq(&a, &c));
    }

    #[test]
    fn a_fill_with_a_changed_outline_is_refused_and_rule_areas_need_no_fill() {
        let mut board = parse(BOARD).unwrap();
        let refill = native("(xy 0 0) (xy 5 0) (xy 4 5)");
        assert!(
            copy_fill_payload(&mut board, &refill)
                .unwrap_err()
                .contains("changed zone plane's net, layers or outline")
        );
        let rules = parse(
            r#"(kicad_pcb
            (zone (layer "F.Cu") (keepout (footprints not_allowed)))
            (zone (layer "User.1"))
            (zone (layer "B.Cu") (fill no)))"#,
        )
        .unwrap();
        assert!(!has_fillable_zones(&rules));
    }

    #[test]
    fn only_unchanged_static_warning_caps_allow_publication() {
        let copper = parse(BOARD).unwrap();
        let other_layers = parse("(kicad_pcb (zone (layer F.Cu)) (zone (layer F.SilkS)))").unwrap();
        let repeated = |kind: &str, severity: &str, count| {
            vec![
                serde_json::json!({
                    "type": kind,
                    "severity": severity,
                    "items": [{"uuid": "example"}],
                });
                count
            ]
        };
        let capped = |kind: &str, severity: &str| {
            repeated(kind, severity, crate::verdict::KICAD_FINDING_CAP)
        };
        let report = |violations| {
            serde_json::json!({
                "kicad_version": "10.0.8",
                "violations": violations,
                "schematic_parity": [],
                "unconnected_items": [],
            })
        };
        let mut warnings = capped("silk_overlap", "warning");
        warnings.extend(capped("lib_footprint_issues", "warning"));
        let before = report(warnings);
        let check = |drc: &Value, board| check_finding_caps(drc, &counts(drc), board);
        check(&before, &copper).unwrap();
        assert!(check(&before, &other_layers).is_err());

        let mut saved = before.clone();
        saved["violations"][0]["items"][0]["uuid"] = serde_json::json!("changed");
        assert!(
            compare_reports(&before, &before, &saved)
                .unwrap_err()
                .contains("silk_overlap")
        );

        for (kind, severity) in [
            ("silk_overlap", "error"),
            ("lib_footprint_issues", "exclusion"),
            ("silk_over_copper", "warning"),
            ("connection_width", "warning"),
            ("clearance", "error"),
        ] {
            let capped = report(capped(kind, severity));
            assert!(check(&capped, &copper).unwrap_err().contains(kind));
        }
        let mut saved = before.clone();
        saved["violations"][0]["excluded"] = serde_json::json!(true);
        assert!(check(&saved, &copper).is_err());
        let mut unknown_version = before.clone();
        unknown_version["kicad_version"] = Value::Null;
        assert!(check(&unknown_version, &copper).is_err());
        let mut opens = before.clone();
        opens["unconnected_items"] = serde_json::json!(repeated("unconnected_items", "error", 499));
        assert!(check(&opens, &copper).is_err());
        let mut parity = before;
        parity["schematic_parity"] = serde_json::json!(capped("net_conflict", "error"));
        assert!(check(&parity, &copper).is_err());
    }

    #[test]
    fn a_cached_fill_rejects_corruption_and_unrelated_board_changes() {
        let input = parse(BOARD).unwrap();
        let mut filled = input.clone();
        let mut refilled = native("(xy 0 0) (xy 5 0) (xy 5 5)");
        let Expr::List(items) = &mut refilled else {
            unreachable!();
        };
        items.push(
            parse(&format!(
                r#"(zone (net "GND") (layer "F.Cu") (attr (teardrop (type padvia)))
                     (fill yes) (polygon (pts (xy 1 0) (xy 3 0) (xy 3 1))) {FILL})"#
            ))
            .unwrap(),
        );
        copy_fill_with_native_teardrops(&mut filled, refilled).unwrap();
        let saved = format!("{}\n", encode(&filled));
        let mut entry = CachedFill {
            key: "current key".into(),
            source_hash: sha256(BOARD.as_bytes()),
            board_hash: sha256(saved.as_bytes()),
            board: saved,
            drc: "{}".into(),
            drc_hash: sha256(b"{}"),
        };
        let decode = |entry: &CachedFill| {
            let bytes = serde_json::to_vec(entry).unwrap();
            CachedFill::decode(&bytes, "current key", &sha256(BOARD.as_bytes()))
        };
        assert_eq!(
            decode(&entry)
                .unwrap()
                .check_unchanged_items(&input)
                .unwrap(),
            (2, 2)
        );
        entry.drc.push('!');
        assert!(decode(&entry).is_err());
        entry.drc.pop();
        entry.board = entry
            .board
            .replace("(uuid \"track\")", "(uuid \"wrong-track\")");
        assert!(decode(&entry).is_err());
        entry.board_hash = sha256(entry.board.as_bytes());
        assert!(
            decode(&entry)
                .unwrap()
                .check_unchanged_items(&input)
                .is_err()
        );
        entry.board = entry
            .board
            .replace("(uuid \"wrong-track\")", "(uuid \"track\")")
            .replace("(type padvia)", "(type track_end)");
        entry.board_hash = sha256(entry.board.as_bytes());
        assert!(
            decode(&entry)
                .unwrap()
                .check_unchanged_items(&input)
                .is_err()
        );
    }
}
