// Copyright (C) 2026 Florian Loitsch.

//! A native connectivity fallback for different witnesses of the same open.

use super::*;

#[derive(Clone, Debug, Serialize)]
pub struct FillConnectivityProof {
    pub copper_objects: usize,
    pub components: usize,
    pub nets: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filled_islands: Option<usize>,
}

/// Gate the more expensive proof on the exact report's only permitted
/// discrepancy. The uncapped reports, including exclusions and parity,
/// still have to agree beyond the native open witnesses.
pub(super) fn eligible(before: &Value, refilled: &Value, stored: &Value) -> bool {
    same_metadata(before, refilled, stored).is_ok()
        && [refilled, stored].into_iter().all(|report| {
            counts(report).possibly_capped_types.is_empty()
                && report[crate::outline_pieces::CAPPED_KEY] != true
        })
        && keys(refilled, "violations") == keys(stored, "violations")
        && keys(refilled, "schematic_parity") == keys(stored, "schematic_parity")
        && refilled["unconnected_items"].as_array().map(Vec::len)
            == stored["unconnected_items"].as_array().map(Vec::len)
        && keys(refilled, "unconnected_items") != keys(stored, "unconnected_items")
}

type Partition = Vec<(String, Vec<String>)>;
type Membership<'a> = BTreeMap<&'a str, (&'a str, usize)>;

fn unmatched_witnesses(
    ours: &FindingCounts,
    theirs: &FindingCounts,
    membership: &Membership<'_>,
) -> Result<BTreeMap<(String, String), usize>, String> {
    let mut counts = BTreeMap::new();
    for (key, count) in ours {
        let excess = count.saturating_sub(theirs.get(key).copied().unwrap_or(0));
        if excess == 0 {
            continue;
        }
        let (kind, text) = key;
        let mut finding: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
        let changed = || "changed open is not an ordinary witness between known copper objects";
        if kind != "unconnected_items"
            || finding["severity"] != "error"
            || finding["excluded"] == true
        {
            return Err(changed().into());
        }
        let items = finding
            .as_object_mut()
            .and_then(|finding| finding.remove("items"))
            .ok_or_else(changed)?;
        let Some([a, b]) = items.as_array().map(Vec::as_slice) else {
            return Err(changed().into());
        };
        let component = |item: &Value| {
            membership
                .get(item["uuid"].as_str()?.to_ascii_lowercase().as_str())
                .copied()
        };
        let ((net, a), (other, b)) = component(a).zip(component(b)).ok_or_else(changed)?;
        if net != other || a == b {
            return Err(changed().into());
        }
        *counts.entry((net.into(), finding.to_string())).or_default() += excess;
    }
    Ok(counts)
}

fn membership(partition: &Partition) -> Result<Membership<'_>, String> {
    let mut membership = BTreeMap::new();
    for (component, (net, objects)) in partition.iter().enumerate() {
        if net.is_empty() || objects.is_empty() {
            return Err("KiCad returned an empty copper component or unnamed net".into());
        }
        for id in objects {
            if id.is_empty()
                || membership
                    .insert(id.as_str(), (net.as_str(), component))
                    .is_some()
            {
                return Err("KiCad returned duplicate or missing copper identities".into());
            }
        }
    }
    Ok(membership)
}

fn compare_witnesses(
    before: &Membership<'_>,
    after: &Membership<'_>,
    refilled_report: &Value,
    stored_report: &Value,
) -> Result<(), String> {
    let refilled = keys(refilled_report, "unconnected_items");
    let stored = keys(stored_report, "unconnected_items");
    if unmatched_witnesses(&refilled, &stored, before)?
        != unmatched_witnesses(&stored, &refilled, after)?
    {
        return Err("native open witness counts changed by net".into());
    }
    Ok(())
}

/// Describe changed open witnesses that the ordinary copper proof cannot
/// identify. The native item label carries the net and layer of fill copper.
fn fill_open_witnesses(
    ours: &FindingCounts,
    theirs: &FindingCounts,
    membership: &Membership<'_>,
    units: &str,
) -> Option<String> {
    let mut shown = Vec::new();
    let mut omitted = 0;
    for (key, count) in ours {
        let (kind, text) = key;
        let excess = count.saturating_sub(theirs.get(key).copied().unwrap_or(0));
        if excess == 0 || kind != "unconnected_items" {
            continue;
        }
        let finding: Value = serde_json::from_str(text).ok()?;
        let Some([a, b]) = finding["items"].as_array().map(Vec::as_slice) else {
            continue;
        };
        let is_unmapped_fill = |item: &Value| {
            item["description"].as_str().is_some_and(|name| {
                (name.starts_with("Zone ") || name.starts_with("Teardrop "))
                    && item["uuid"]
                        .as_str()
                        .is_none_or(|id| !membership.contains_key(id.to_ascii_lowercase().as_str()))
            })
        };
        if finding["severity"] != "error"
            || finding["excluded"] == true
            || ![a, b].into_iter().any(is_unmapped_fill)
        {
            continue;
        }
        if shown.len() >= 2 {
            omitted += excess;
            continue;
        }
        let label = |item: &Value| {
            let description = item["description"]
                .as_str()
                .unwrap_or("Unknown native item");
            let id = item["uuid"].as_str().unwrap_or("no UUID");
            match item["pos"]["x"].as_f64().zip(item["pos"]["y"].as_f64()) {
                Some((x, y)) => format!("{description} ({id}, {x:.6}, {y:.6} {units})"),
                None => format!("{description} ({id})"),
            }
        };
        let mut example = format!("{} to {}", label(a), label(b));
        if excess > 1 {
            example.push_str(&format!(" ({excess} occurrences)"));
        }
        shown.push(example);
    }
    if omitted > 0 {
        shown.push(format!("{omitted} more changed open witnesses"));
    }
    (!shown.is_empty()).then(|| shown.join(" / "))
}

pub(super) fn has_changed_fill_witnesses(first: &Value, second: &Value) -> bool {
    let a = keys(first, "unconnected_items");
    let b = keys(second, "unconnected_items");
    let unknown = BTreeMap::new();
    fill_open_witnesses(&a, &b, &unknown, "").is_some()
        || fill_open_witnesses(&b, &a, &unknown, "").is_some()
}

fn compare_fill_witnesses(
    membership: &Membership<'_>,
    refilled_report: &Value,
    stored_report: &Value,
) -> Result<(), String> {
    compare_witnesses(membership, membership, refilled_report, stored_report).map_err(|reason| {
        let refilled = keys(refilled_report, "unconnected_items");
        let stored = keys(stored_report, "unconnected_items");
        let units = refilled_report["coordinate_units"]
            .as_str()
            .unwrap_or("native units");
        let evidence: Vec<_> = [
            ("Refilled check", fill_open_witnesses(&refilled, &stored, membership, units)),
            ("Stored check", fill_open_witnesses(&stored, &refilled, membership, units)),
        ]
        .into_iter()
        .filter_map(|(scope, evidence)| evidence.map(|text| format!("{scope} named {text}.")))
        .collect();
        if evidence.is_empty() {
            return reason;
        }
        format!(
            "Native pad, track and via connectivity matched. The changed open witnesses include zone or teardrop copper that the partition cannot identify. {} Inspect the named nets and locations in a private KiCad copy, including the filled islands and their pad or via attachments. Resolve any real breaks, check the saved board and a fresh refill, then retry --save-filled",
            evidence.join(" "),
        )
    })
}

/// Compare changed ordinary open witnesses against each board's native
/// partition. The caller must separately prove the surviving partition.
pub(crate) fn verify_open_witnesses(
    original: &Partition,
    candidate: &Partition,
    before: &Value,
    after: &Value,
) -> Result<(), String> {
    compare_witnesses(
        &membership(original)?,
        &membership(candidate)?,
        before,
        after,
    )
}

fn verify(
    original: &Partition,
    saved: &Partition,
    refilled_report: &Value,
    stored_report: &Value,
) -> Result<FillConnectivityProof, String> {
    if original != saved {
        return Err("saving the fill changed KiCad's copper component membership".into());
    }
    let membership = membership(saved)?;
    compare_fill_witnesses(&membership, refilled_report, stored_report)?;
    let nets: std::collections::BTreeSet<_> = saved.iter().map(|(net, _)| net).collect();
    Ok(FillConnectivityProof {
        copper_objects: membership.len(),
        components: saved.len(),
        nets: nets.len(),
        filled_islands: None,
    })
}

pub(super) fn prove_saved_islands(
    staged: &Path,
    native_fill: &[u8],
    refilled: &Value,
    stored: &Value,
    checks: &mut usize,
) -> Result<FillConnectivityProof, String> {
    let version = refilled["kicad_version"]
        .as_str()
        .ok_or("missing version for native island proof")?;
    let candidate = fs::read(staged).map_err(|error| error.to_string())?;
    // Read the exact snapshot emitted by the first native refill DRC.
    fs_util::write_atomically(staged, native_fill)?;
    *checks += 1;
    let before = native_report::connectivity_islands(staged, version, false, || false);
    let unchanged = fs::read(staged).is_ok_and(|bytes| bytes == native_fill);
    fs_util::write_atomically(staged, &candidate)?;
    if !unchanged {
        return Err("checking refilled islands changed the staged board".into());
    }
    let before = before?;
    *checks += 1;
    let after = native_report::connectivity_islands(staged, version, false, || false)?;
    if !fs::read(staged).is_ok_and(|bytes| bytes == candidate) {
        return Err("checking stored islands changed the staged board".into());
    }
    let original_opens = before.open_counts(refilled);
    if before.ordinary != after.ordinary
        || before.shapes != after.shapes
        || !after.keeps_island_attachments(&before)
        || original_opens.is_none()
        || original_opens != after.open_counts(stored)
    {
        return Err("the saved board did not reproduce the exact island shapes, copper attachments and counts of native open evidence by net".into());
    }
    Ok(FillConnectivityProof {
        copper_objects: membership(&after.ordinary)?.len(),
        components: after.ordinary.len(),
        nets: after
            .ordinary
            .iter()
            .map(|(net, _)| net)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        filled_islands: Some(after.islands.len()),
    })
}

/// Use the same staged filename for each probe, so KiCad sees the
/// same .pro, .dru, schematic, relative libraries and zone configuration.
/// Restore the candidate even if the first native load fails.
pub(super) fn prove(
    staged: &Path,
    original: &[u8],
    refilled: &Value,
    stored: &Value,
    checks: &mut usize,
    wall_passed: impl Fn() -> bool,
    candidate_refill: Option<&Value>,
) -> Result<FillConnectivityProof, String> {
    let version = refilled["kicad_version"]
        .as_str()
        .ok_or("missing version for native connectivity proof")?;
    let candidate = fs::read(staged).map_err(|error| error.to_string())?;
    if wall_passed() {
        return Err("layout wall guard expired before native connectivity proof".into());
    }
    fs_util::write_atomically(staged, original)?;
    *checks += 1;
    let before = native_report::connectivity_partition(staged, version, true, &wall_passed);
    let before_unchanged = fs::read(staged).is_ok_and(|bytes| bytes == original);
    fs_util::write_atomically(staged, &candidate)?;
    if !before_unchanged {
        return Err("checking refilled connectivity changed the staged board".into());
    }
    let before = before?;
    if wall_passed() {
        return Err("layout wall guard expired before checking the stored partition".into());
    }
    *checks += 1;
    let after = native_report::connectivity_partition(staged, version, false, &wall_passed)?;
    if fs::read(staged).map_err(|error| error.to_string())? != candidate {
        return Err("checking stored connectivity changed the staged board".into());
    }
    if let Some(reopened) = candidate_refill {
        *checks += 1;
        let reopened_partition =
            native_report::connectivity_partition(staged, version, true, wall_passed)?;
        if reopened_partition != after
            || fs::read(staged).map_err(|error| error.to_string())? != candidate
        {
            return Err(
                "refilling the ordered board changed its stored copper partition or file".into(),
            );
        }
        compare_fill_witnesses(&membership(&after)?, reopened, stored)?;
    }
    verify(&before, &after, refilled, stored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn witness(a: &str, b: &str) -> Value {
        json!({
            "type": "unconnected_items",
            "severity": "error",
            "description": "Missing connection between items",
            "items": [{"uuid": a}, {"uuid": b}],
        })
    }

    fn report(a: &str, b: &str) -> Value {
        json!({
            "kicad_version": "10.0.8",
            "coordinate_units": "mm",
            "violations": [],
            "unconnected_items": [witness(a, b)],
            "schematic_parity": [],
        })
    }

    #[test]
    fn another_pair_between_the_same_copper_components_is_a_witness() {
        let original = vec![
            ("SIGNAL".into(), vec!["a".into(), "b".into()]),
            ("SIGNAL".into(), vec!["c".into(), "d".into()]),
        ];
        let before = report("a", "c");
        let reopened = report("b", "d");
        assert!(eligible(&before, &before, &reopened));
        let proof = verify(&original, &original, &before, &reopened).unwrap();
        assert_eq!(proof.copper_objects, 4);
        assert_eq!(proof.components, 2);
        assert_eq!(proof.nets, 1);

        let damaged = vec![
            ("SIGNAL".into(), vec!["a".into(), "c".into()]),
            ("SIGNAL".into(), vec!["b".into(), "d".into()]),
        ];
        let error = verify(&original, &damaged, &before, &report("c", "d")).unwrap_err();
        assert!(error.contains("component membership"));
        assert!(verify(&original, &original, &before, &report("a", "b")).is_err());
        assert!(verify(&original, &original, &before, &report("a", "zone")).is_err());
    }

    #[test]
    fn changed_fill_witnesses_show_both_modes_without_waiving_missing_island_proof() {
        let partition = vec![
            ("GND".into(), vec!["pad-a".into()]),
            ("GND".into(), vec!["pad-b".into()]),
        ];
        let mut refilled = report("pad-a", "zone-uuid");
        let mut stored = report("pad-b", "teardrop-uuid");
        refilled["unconnected_items"][0]["items"][1]["description"] =
            json!("Zone [GND] on In1.Cu, priority 0");
        stored["unconnected_items"][0]["items"][1]["description"] =
            json!("Teardrop [GND] on In1.Cu");
        stored["unconnected_items"][0]["items"][1]["pos"] = json!({"x":217.4,"y":106.566601});
        assert!(eligible(&refilled, &refilled, &stored));
        let refused = verify(&partition, &partition, &refilled, &stored).unwrap_err();
        for evidence in [
            "Refilled check",
            "Stored check",
            "Zone [GND] on In1.Cu",
            "zone-uuid",
            "Teardrop [GND] on In1.Cu",
            "teardrop-uuid, 217.400000, 106.566601 mm",
            "private KiCad copy",
            "--save-filled",
        ] {
            assert!(refused.contains(evidence), "{refused}");
        }
        assert!(
            verify(&partition, &partition, &refilled, &refilled).is_ok(),
            "an unchanged fill witness is covered by the exact report comparison"
        );
        let unknown = report("pad-b", "unclassified");
        let ordinary = report("pad-a", "pad-b");
        let refusal = verify(&partition, &partition, &ordinary, &unknown).unwrap_err();
        assert_eq!(
            refusal,
            "changed open is not an ordinary witness between known copper objects"
        );
    }

    #[test]
    fn unrelated_findings_and_capped_reports_keep_the_exact_gate() {
        let before = report("a", "c");
        let mut reopened = report("b", "d");
        reopened["schematic_parity"] = json!([{"type": "net_conflict", "items": []}]);
        assert!(!eligible(&before, &before, &reopened));
        reopened["schematic_parity"] = json!([]);
        reopened["violations"] = json!([{
            "type": "copper_edge_clearance",
            "severity": "warning",
            "excluded": true,
            "items": [],
        }]);
        assert!(!eligible(&before, &before, &reopened));
        reopened["violations"] = json!([]);
        reopened["unconnected_items"] = json!(vec![witness("a", "c"); 499]);
        assert!(!eligible(&reopened, &reopened, &reopened));
    }
}
