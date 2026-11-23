// Copyright (C) 2026 Florian Loitsch.

//! Correlate native zone-only width findings with locations in saved copper.

use super::{
    ReportDirectory, bounded_native_board_probe, check_kept_route_input, check_zone_width_witness,
    save_kept_route_fill,
};
use pcb_kicad_file::{fs_util, kicad_sexpr};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
    fs,
    path::Path,
};

const NATIVE_WIDTH_LIMIT: usize = 199;
const MAX_BOARD_BYTES: u64 = 32 * 1024 * 1024;
const MAX_TARGETS: usize = 16;
const MAX_PROBES: usize = 2_048;
const MAX_WIDTH_REPLAYS: usize = 6;
const NM_PER_MM: f64 = 1_000_000.0;

#[derive(Serialize)]
pub struct ZoneWidthLocalization {
    pub source_board_sha256: String,
    pub native_checks: usize,
    pub native_geometry_reads: usize,
    pub probes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geometry_fill: Option<&'static str>,
    pub findings: Vec<ZoneWidthFinding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Serialize)]
pub struct ZoneWidthFinding {
    pub finding: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<ZoneWidthLocation>,
}

#[derive(Clone, Serialize)]
pub struct ZoneWidthLocation {
    pub zone_uuid: String,
    pub net: String,
    pub layer: String,
    pub actual_mm: f64,
    pub midpoint_mm: [f64; 2],
    pub chord_mm: [[f64; 2]; 2],
    pub evidence: &'static str,
}

#[derive(Serialize)]
struct Target<'a> {
    zone_uuid: &'a str,
    layer: &'a str,
    actual_nm: i64,
}

impl<'a> Target<'a> {
    fn from_finding(finding: &'a Value) -> Option<Self> {
        let item = finding["items"].as_array()?.first()?;
        let zone_uuid = item["uuid"].as_str()?;
        let (_, measured) = finding["description"].as_str()?.rsplit_once("; actual ")?;
        let (width, layer) = measured.split_once(" mm) (")?;
        // Match the precision of KiCad's reported measurement explicitly.
        // Older or unfamiliar description formats can remain unlocalized.
        let (mm, fraction) = width.split_once('.')?;
        if fraction.len() != 4 || !fraction.bytes().all(|digit| digit.is_ascii_digit()) {
            return None;
        }
        let actual_nm = mm
            .parse::<i64>()
            .ok()?
            .checked_mul(1_000_000)?
            .checked_add(fraction.parse::<i64>().ok()? * 100)?;
        (actual_nm > 0).then_some(Self {
            zone_uuid,
            layer: layer.strip_suffix(')')?,
            actual_nm,
        })
    }
}

#[derive(Deserialize)]
struct Probe {
    net: String,
    layer: String,
    actual_nm: i64,
    zone_uuids: Vec<String>,
    midpoint_nm: [i64; 2],
    chord_nm: [[i64; 2]; 2],
}

impl Probe {
    fn location(&self, target: &Target<'_>) -> Option<ZoneWidthLocation> {
        if self.layer != target.layer
            || self.actual_nm != target.actual_nm
            || !self.zone_uuids.iter().any(|uuid| uuid == target.zone_uuid)
        {
            return None;
        }
        let mm = |coord: i64| coord as f64 / NM_PER_MM;
        Some(ZoneWidthLocation {
            zone_uuid: target.zone_uuid.into(),
            net: self.net.clone(),
            layer: self.layer.clone(),
            actual_mm: mm(self.actual_nm),
            midpoint_mm: self.midpoint_nm.map(mm),
            chord_mm: self.chord_nm.map(|point| point.map(mm)),
            evidence: "native_stored_width_witness",
        })
    }
}

fn width_findings(report: &Value) -> impl Iterator<Item = &Value> {
    report["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|finding| finding["type"] == "connection_width")
}

fn zone_only_error(finding: &Value) -> bool {
    finding["severity"] == "error"
        && finding["excluded"] != true
        && finding["items"].as_array().is_some_and(|items| {
            items.len() == 1
                && items[0]["description"]
                    .as_str()
                    .is_some_and(|description| description.starts_with("Zone ["))
        })
}

/// Use the entire finding including its rule text, severity, exclusion data
/// and original items. Native sometimes returns those items in another order.
fn fingerprint<'a>(
    finding: &'a Value,
    probes: &BTreeMap<String, &Probe>,
) -> (String, Vec<&'a str>) {
    let mut row = finding.clone();
    let mut witnesses = Vec::new();
    if let Some(items) = row["items"].as_array_mut() {
        for item in finding["items"].as_array().into_iter().flatten() {
            if let Some(uuid) = item["uuid"]
                .as_str()
                .filter(|uuid| probes.contains_key(*uuid))
            {
                witnesses.push(uuid);
            }
        }
        items.retain(|item| {
            !item["uuid"]
                .as_str()
                .is_some_and(|uuid| probes.contains_key(uuid))
        });
        items.sort_by_cached_key(Value::to_string);
    }
    (row.to_string(), witnesses)
}

fn width_multiset<'a>(
    rows: impl Iterator<Item = &'a Value>,
    probes: &BTreeMap<String, &Probe>,
) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for row in rows {
        *counts.entry(fingerprint(row, probes).0).or_insert(0) += 1;
    }
    counts
}

fn source_hashes(board: &Path) -> Result<Vec<Option<String>>, String> {
    let mut hashes = vec![Some(fs_util::file_sha256(board)?)];
    for extension in ["kicad_pro", "kicad_dru"] {
        let companion = board.with_extension(extension);
        hashes.push(
            companion
                .try_exists()
                .map_err(|error| error.to_string())?
                .then(|| fs_util::file_sha256(&companion))
                .transpose()?,
        );
    }
    Ok(hashes)
}

/// Return the fresh refilled DRC unchanged, with a separate optional
/// diagnostic for zone-only widths. All temporary copper lives in scratch.
pub fn localize_zone_widths(board: &Path) -> Result<(Value, ZoneWidthLocalization), String> {
    if fs::metadata(board)
        .map_err(|error| error.to_string())?
        .len()
        > MAX_BOARD_BYTES
    {
        return Err("the PCB exceeds the 32 MiB zone width diagnostic limit".into());
    }
    let source_hashes_before = source_hashes(board)?;
    let board_hash = source_hashes_before[0]
        .clone()
        .ok_or("missing source PCB hash")?;
    let scratch = ReportDirectory::new(&std::env::temp_dir())?;
    let expired = || false;
    let refilled = check_kept_route_input(
        board,
        &scratch.0.join("refilled.json"),
        false,
        true,
        expired,
    )?;
    let mut report = new_report(board_hash, &refilled);
    report.native_checks = 1;
    if !report.findings.is_empty() {
        report.native_checks += 1;
        let stored =
            check_kept_route_input(board, &scratch.0.join("stored.json"), false, false, expired);
        report.reason = stored
            .and_then(|stored| {
                let matches = |other: &Value| same_width_findings(&refilled, other);
                if matches(&stored) {
                    verify_locations(
                        board,
                        &scratch.0,
                        &refilled,
                        &stored,
                        &mut report,
                        true,
                        expired,
                    )
                } else {
                    let private = scratch.0.join("native-fill");
                    fs::create_dir(&private).map_err(|error| error.to_string())?;
                    let filled_board =
                        private.join(board.file_name().ok_or("missing PCB filename")?);
                    fs::copy(board, &filled_board).map_err(|error| error.to_string())?;
                    copy_rules(board, &filled_board)?;
                    report.native_checks += 1;
                    let filled = save_kept_route_fill(
                        &filled_board,
                        &private.join("refilled.json"),
                        false,
                        expired,
                    )?;
                    if !matches(&filled) {
                        return Err(
                            "the private refill did not reproduce the source width findings".into(),
                        );
                    }
                    report.native_checks += 1;
                    let reopened = check_kept_route_input(
                        &filled_board,
                        &private.join("stored.json"),
                        false,
                        false,
                        expired,
                    )?;
                    report.geometry_fill = Some("private_refill");
                    verify_locations(
                        &filled_board,
                        &scratch.0,
                        &refilled,
                        &reopened,
                        &mut report,
                        true,
                        expired,
                    )
                }
            })
            .err();
    }
    if source_hashes(board)? != source_hashes_before {
        return Err(
            "the source PCB or its project rules changed during the width diagnostic".into(),
        );
    }
    Ok((refilled, report))
}

fn new_report(board_hash: String, refilled: &Value) -> ZoneWidthLocalization {
    ZoneWidthLocalization {
        source_board_sha256: board_hash,
        native_checks: 0,
        native_geometry_reads: 0,
        probes: 0,
        geometry_fill: None,
        findings: width_findings(refilled)
            .filter(|finding| zone_only_error(finding))
            .map(|finding| ZoneWidthFinding {
                finding: finding.clone(),
                location: None,
            })
            .collect(),
        reason: None,
    }
}

/// Reuse in-process source checks from a guarded fill handoff. New positions
/// still require native geometry and a fresh native witness DRC.
pub fn localize_zone_widths_from_checked_fill(
    board: &Path,
    board_hash: &str,
    refilled: &Value,
    stored: &Value,
    expired: impl Fn() -> bool + Copy,
) -> ZoneWidthLocalization {
    let mut report = new_report(board_hash.into(), refilled);
    if !report.findings.is_empty() {
        report.reason = (|| {
            let before = source_hashes(board)?;
            if before[0].as_deref() != Some(board_hash) {
                return Err("the PCB changed after the verified fill handoff".into());
            }
            let scratch = ReportDirectory::new(&std::env::temp_dir())?;
            let result = verify_locations(
                board,
                &scratch.0,
                refilled,
                stored,
                &mut report,
                false,
                expired,
            );
            if source_hashes(board)? != before {
                report
                    .findings
                    .iter_mut()
                    .for_each(|finding| finding.location = None);
                return Err(
                    "the source PCB or its project rules changed during the width diagnostic"
                        .into(),
                );
            }
            result
        })()
        .err();
    }
    report
}

fn verify_locations(
    board: &Path,
    scratch: &Path,
    refilled: &Value,
    stored: &Value,
    report: &mut ZoneWidthLocalization,
    retry_separate_widths: bool,
    expired: impl Fn() -> bool + Copy,
) -> Result<(), String> {
    if report.findings.len() > MAX_TARGETS || width_findings(refilled).count() >= NATIVE_WIDTH_LIMIT
    {
        return Err("too many native width findings for this bounded diagnostic".into());
    }
    let version = refilled["kicad_version"]
        .as_str()
        .ok_or("missing native DRC version")?;
    if !same_width_findings(refilled, stored) {
        return Err("saved copper does not reproduce the refilled width findings; check a verified --save-filled copy".into());
    }
    let targets: Vec<_> = report
        .findings
        .iter()
        .filter_map(|row| Target::from_finding(&row.finding))
        .collect();
    if targets.is_empty() {
        return Err(
            "KiCad's zone width descriptions could not be read as a layer and precise width".into(),
        );
    }
    let request = serde_json::to_value(targets).map_err(|error| error.to_string())?;
    if expired() {
        return Err("the width diagnostic deadline expired before reading saved copper".into());
    }
    let evidence_hash = match report.geometry_fill {
        Some("private_refill") => fs_util::file_sha256(board)?,
        _ => {
            report.geometry_fill = Some("saved_source");
            report.source_board_sha256.clone()
        }
    };
    report.native_geometry_reads += 1;
    let (geometry, board_hash) = bounded_native_board_probe(
        board,
        version,
        &request,
        include_str!("zone_width_geometry.py"),
        "zone width geometry",
        true,
        expired,
    )?;
    if board_hash != evidence_hash {
        return Err("the source PCB changed before reading width geometry".into());
    }
    let candidates: Vec<Probe> =
        serde_json::from_value(geometry).map_err(|error| error.to_string())?;
    if candidates.is_empty() || candidates.len() > MAX_PROBES {
        return Err("no width-matching chord within the bounded search of saved copper".into());
    }
    report.probes = candidates.len();
    let staged = scratch.join(board.file_name().ok_or("missing PCB filename")?);
    copy_rules(board, &staged)?;
    let source = fs::read_to_string(board).map_err(|error| error.to_string())?;
    let prefix = source
        .trim_end()
        .strip_suffix(')')
        .ok_or("missing closing PCB form")?;
    let source_lower = source.to_ascii_lowercase();
    let source_ids: BTreeSet<_> = source_lower
        .split(|character: char| character != '-' && !character.is_ascii_hexdigit())
        .filter(|token| token.len() == 36)
        .collect();
    let witness = WitnessBoard {
        staged: &staged,
        hash: &board_hash,
        prefix,
        source_ids,
        refilled,
    };
    witness.check(candidates.iter(), report, expired)?;
    if retry_separate_widths && report.findings.iter().any(|row| row.location.is_none()) {
        let widths: BTreeSet<_> = report
            .findings
            .iter()
            .filter(|row| row.location.is_none())
            .filter_map(|row| Target::from_finding(&row.finding).map(|target| target.actual_nm))
            .collect();
        if widths.len() > 1
            || candidates
                .iter()
                .any(|probe| !widths.contains(&probe.actual_nm))
        {
            for width in widths.into_iter().take(MAX_WIDTH_REPLAYS) {
                if expired() {
                    break;
                }
                witness.check(
                    candidates.iter().filter(|probe| probe.actual_nm == width),
                    report,
                    expired,
                )?;
            }
        }
    }
    if report.findings.iter().any(|row| row.location.is_none()) {
        return Err(
            "KiCad did not uniquely name a scratch witness for every zone width finding".into(),
        );
    }
    Ok(())
}

struct WitnessBoard<'a> {
    staged: &'a Path,
    hash: &'a str,
    prefix: &'a str,
    source_ids: BTreeSet<&'a str>,
    refilled: &'a Value,
}

impl WitnessBoard<'_> {
    fn check<'a>(
        &self,
        candidates: impl Iterator<Item = &'a Probe>,
        report: &mut ZoneWidthLocalization,
        expired: impl Fn() -> bool + Copy,
    ) -> Result<(), String> {
        let mut text = self.prefix.to_owned();
        let mut probes = BTreeMap::new();
        let board_hash = self.hash;
        for (index, probe) in candidates.enumerate() {
            let uuid = kicad_sexpr::deterministic_uuid_for(&format!(
                "zone-width-probe:{board_hash}:{index}"
            ));
            if self.source_ids.contains(uuid.as_str()) || probes.contains_key(&uuid) {
                return Err("scratch width witness has a conflicting PCB item UUID".into());
            }
            let [x, y] = probe.midpoint_nm.map(|value| value as f64 / NM_PER_MM);
            let layer = kicad_sexpr::escape(&probe.layer);
            let net = kicad_sexpr::escape(&probe.net);
            // A micron-wide witness makes KiCad name the pinch without replacing
            // the source copper. It must never be included in the returned PCB.
            writeln!(
            &mut text,
            "(segment (start {x:.6} {y:.6}) (end {:.6} {y:.6}) (width 0.001) (layer \"{layer}\") (net \"{net}\") (uuid \"{uuid}\"))",
            x + 0.0005
        )
        .map_err(|error| error.to_string())?;
            probes.insert(uuid, probe);
        }
        if probes.is_empty() {
            return Ok(());
        }
        text.push_str(")\n");
        fs::write(self.staged, text).map_err(|error| error.to_string())?;
        if expired() {
            return Err(
                "the width diagnostic deadline expired before checking scratch witnesses".into(),
            );
        }
        report.native_checks += 1;
        let replay = check_zone_width_witness(
            self.staged,
            &self.staged.with_extension("witnesses.json"),
            expired,
        )?;
        let empty = BTreeMap::new();
        let expected = width_multiset(width_findings(self.refilled), &empty);
        if !same_width_policy(self.refilled, &replay)
            || width_findings(&replay).count() >= NATIVE_WIDTH_LIMIT
            || width_multiset(width_findings(&replay), &probes) != expected
        {
            // The added probes can replace a source zone witness. This pass
            // establishes no locations when any original width is changed.
            return Ok(());
        }
        let mut witnessed = BTreeMap::<String, Vec<_>>::new();
        for width in width_findings(&replay) {
            let (key, ids) = fingerprint(width, &probes);
            if !ids.is_empty() {
                witnessed
                    .entry(key)
                    .or_default()
                    .push(ids.into_iter().map(|id| probes[id]).collect::<Vec<_>>());
            }
        }
        for row in report
            .findings
            .iter_mut()
            .filter(|row| row.location.is_none())
        {
            let key = fingerprint(&row.finding, &empty).0;
            // Several byte-identical source findings name the same zone and
            // numeric width. Expose only independently witnessed locations.
            if let Some(target) = Target::from_finding(&row.finding)
                && let Some(found) = witnessed.get_mut(&key)
            {
                while let Some(witnesses) = found.pop() {
                    let mut scoped = witnesses.iter().filter_map(|probe| {
                        probe.location(&target).map(|location| (probe, location))
                    });
                    if let (Some((probe, location)), None) = (scoped.next(), scoped.next()) {
                        found.retain(|others| {
                            !others
                                .iter()
                                .any(|other| other.midpoint_nm == probe.midpoint_nm)
                        });
                        row.location = Some(location);
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

fn copy_rules(board: &Path, staged: &Path) -> Result<(), String> {
    for extension in ["kicad_pro", "kicad_dru"] {
        let source = board.with_extension(extension);
        if source.is_file() {
            fs::copy(source, staged.with_extension(extension))
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn same_width_policy(before: &Value, other: &Value) -> bool {
    [
        "kicad_version",
        "coordinate_units",
        "included_severities",
        "ignored_checks",
    ]
    .into_iter()
    .all(|field| before[field] == other[field])
}

fn same_width_findings(before: &Value, other: &Value) -> bool {
    let no_probes = BTreeMap::new();
    same_width_policy(before, other)
        && width_multiset(width_findings(before), &no_probes)
            == width_multiset(width_findings(other), &no_probes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn width(description: &str, item_ids: &[&str]) -> Value {
        json!({
            "type": "connection_width",
            "severity": "error",
            "description": description,
            "items": item_ids.iter().map(|id| json!({
                "uuid": id, "description": "Zone [GND] on F.Cu", "pos": {"x": 0, "y": 0}
            })).collect::<Vec<_>>()
        })
    }

    #[test]
    fn a_witness_can_add_an_item_but_not_replace_the_zone_or_move_the_measurement() {
        let before = width(
            "minimum connection width 0.1270 mm; actual 0.0774 mm) (F.Cu)",
            &["zone"],
        );
        let same = width(before["description"].as_str().unwrap(), &["probe", "zone"]);
        let replaced = width(before["description"].as_str().unwrap(), &["probe"]);
        let probe = Probe {
            net: "GND".into(),
            layer: "F.Cu".into(),
            actual_nm: 77_400,
            zone_uuids: vec!["zone".into()],
            midpoint_nm: [90, 100],
            chord_nm: [[0, 0], [180, 200]],
        };
        let target = Target::from_finding(&before).unwrap();
        assert_eq!(probe.location(&target).unwrap().zone_uuid, "zone");
        let mut unrelated = Target::from_finding(&before).unwrap();
        unrelated.zone_uuid = "different-zone";
        assert!(probe.location(&unrelated).is_none());
        let probes = BTreeMap::from([("probe".into(), &probe)]);
        let key = fingerprint(&before, &probes).0;
        let (replayed, witnesses) = fingerprint(&same, &probes);
        assert_eq!(key, replayed);
        assert_eq!(witnesses, vec!["probe"]);
        assert_ne!(key, fingerprint(&replaced, &probes).0);
        let mut changed = same;
        changed["description"] =
            json!("minimum connection width 0.1270 mm; actual 0.0773 mm) (F.Cu)");
        assert_ne!(key, fingerprint(&changed, &probes).0);
    }

    #[test]
    fn localized_scope_excludes_already_named_tracks_and_exclusions() {
        let mut finding = width(
            "minimum connection width 0.1270 mm; actual 0.0774 mm) (F.Cu)",
            &["zone"],
        );
        assert!(zone_only_error(&finding));
        finding["excluded"] = json!(true);
        assert!(!zone_only_error(&finding));
        finding["excluded"] = json!(false);
        finding["items"][0]["description"] = json!("Track [GND] on F.Cu");
        assert!(!zone_only_error(&finding));
    }
}
