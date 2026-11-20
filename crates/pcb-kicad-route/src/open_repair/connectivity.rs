// Copyright (C) 2026 Florian Loitsch.

//! Verify additive repairs when KiCad reports different ratsnest witnesses.

use super::*;
use pcb_kicad_file::fs_util::write_atomically;

type Partition = Vec<(String, Vec<String>)>;

#[derive(Serialize)]
pub struct OpenConnectivityProof {
    pub prior_copper_objects: usize,
    pub added_copper_objects: usize,
    pub target_components_before: usize,
    pub target_components_after: usize,
}

/// Confirms that only ordinary native open witnesses differ before an
/// electrical partition proof is attempted.
pub fn eligible_open_partition(before: &Value, after: &Value) -> Result<bool, String> {
    let regular_witnesses = |report: &Value| {
        report["unconnected_items"].as_array().is_some_and(|items| {
            items.iter().all(|finding| {
                finding["type"] == "unconnected_items"
                    && finding["severity"] == "error"
                    && finding["excluded"] != true
                    && finding["items"]
                        .as_array()
                        .is_some_and(|items| items.len() == 2)
            })
        })
    };
    if !same_check_policy(before, after)
        || [before, after].into_iter().any(|report| {
            report_capped(report)
                || !finding_types_at_cap(report, "schematic_parity").is_empty()
                || !regular_witnesses(report)
        })
    {
        return Ok(false);
    }
    for key in ["violations", "schematic_parity", ACROSS_KEY] {
        if finding_counts(before, key)? != finding_counts(after, key)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) struct Trial<'a> {
    pub board: &'a Path,
    pub original: &'a str,
    pub additions: &'a [Expr],
    pub previous: usize,
    pub target: &'a str,
    pub refill: bool,
}

pub(super) enum ExpectedWitnesses<'a> {
    PadBridge(&'a SameFootprintPadBridge),
    WidthReplay {
        original: &'a Value,
        candidate: &'a Value,
    },
}

type OpenPairs = BTreeMap<([usize; 2], String), usize>;

fn open_pairs(partition: &Partition, report: &Value) -> Option<OpenPairs> {
    let memberships = partition
        .iter()
        .enumerate()
        .flat_map(|(index, (_, members))| members.iter().map(move |id| (id.as_str(), index)))
        .collect::<BTreeMap<_, _>>();
    let mut pairs = BTreeMap::new();
    for finding in report["unconnected_items"].as_array()? {
        if finding["type"] != "unconnected_items"
            || finding["severity"] != "error"
            || finding["excluded"] == true
        {
            return None;
        }
        let mut metadata = finding.clone();
        let items = metadata.as_object_mut()?.remove("items")?;
        let [first, second] = items.as_array()?.as_slice() else {
            return None;
        };
        let membership = |item: &Value| {
            memberships
                .get(item["uuid"].as_str()?.to_ascii_lowercase().as_str())
                .copied()
        };
        let mut endpoints = [membership(first)?, membership(second)?];
        if partition[endpoints[0]].0 != partition[endpoints[1]].0 {
            return None;
        }
        endpoints.sort_unstable();
        *pairs.entry((endpoints, metadata.to_string())).or_default() += 1;
    }
    Some(pairs)
}

fn verify_replayed_open_pairs(
    source: &Partition,
    saved: &Partition,
    original: &Value,
    replay: &Value,
    candidate: &Value,
) -> Option<()> {
    let expected = open_pairs(source, original)?;
    if expected.keys().any(|(pair, _)| pair[0] == pair[1])
        || open_pairs(source, replay)? != expected
    {
        return None;
    }
    // Evaluate both original witnesses on the candidate partition. Every
    // suppressed original open must now connect on its exact original IDs.
    let connected = open_pairs(saved, original)?;
    let remaining = open_pairs(saved, candidate)?;
    let still_separate = connected
        .into_iter()
        .filter(|((pair, _), _)| pair[0] != pair[1])
        .collect::<OpenPairs>();
    (remaining == still_separate).then_some(())
}

impl Trial<'_> {
    pub(super) fn prove(
        &self,
        before: &Value,
        reduction: usize,
        budget: &mut Option<&mut LayoutRepairBudget<'_>>,
        expected: Option<ExpectedWitnesses<'_>>,
    ) -> Result<OpenConnectivityProof, String> {
        if budget
            .as_ref()
            .is_some_and(|budget| !budget.can_check_trials(2))
        {
            return Err(
                "remaining layout work does not cover two native connectivity checks".into(),
            );
        }
        let version = before["kicad_version"]
            .as_str()
            .ok_or("missing native version")?;
        let wall = Instant::now() + std::time::Duration::from_secs(20);
        let wall = budget
            .as_ref()
            .and_then(|budget| budget.router.deadline)
            .map_or(wall, |limit| limit.min(wall));
        let expired = || Instant::now() >= wall;
        let candidate = fs::read(self.board).map_err(|error| error.to_string())?;
        write_additions(self.board, self.original, &self.additions[..self.previous])?;
        let baseline = fs::read(self.board).map_err(|error| error.to_string())?;
        if let Some(budget) = budget {
            budget.native_checks += 1;
        }
        let a = native_report::connectivity_partition(self.board, version, self.refill, expired);
        let unchanged = fs::read(self.board).is_ok_and(|board| board == baseline);
        write_atomically(self.board, &candidate)?;
        if !unchanged {
            return Err("KiCad modified the baseline during its connectivity check".into());
        }
        let a = a?;
        if expired() {
            return Err("the wall guard expired during native connectivity verification".into());
        }
        if let Some(budget) = budget {
            budget.native_checks += 1;
        }
        let b = native_report::connectivity_partition(self.board, version, self.refill, expired)?;
        if !fs::read(self.board).is_ok_and(|board| board == candidate) {
            return Err("KiCad modified the candidate during its connectivity check".into());
        }
        match expected {
            Some(ExpectedWitnesses::PadBridge(bridge)) if !bridge.proves_partitions(&a, &b) => {
                return Err(
                    "the native partition does not identify the two pads as the open components"
                        .into(),
                );
            }
            Some(ExpectedWitnesses::WidthReplay {
                original,
                candidate,
            }) if verify_replayed_open_pairs(&a, &b, original, before, candidate).is_none() => {
                return Err("the replay changed which original components were open, or the trial failed to join the reported open endpoints".into());
            }
            _ => (),
        }
        verify(
            a,
            b,
            self.target,
            reduction,
            &self.additions[self.previous..],
        )
    }
}

pub(super) fn verify(
    before: Partition,
    after: Partition,
    target: &str,
    native_open_reduction: usize,
    additions: &[Expr],
) -> Result<OpenConnectivityProof, String> {
    verify_native_partition_merges(
        before,
        after,
        &[target.to_string()],
        native_open_reduction,
        additions,
    )
}

/// Confirms that existing copper components either keep their membership
/// or merge on nominated nets, and that all new copper is accounted for.
pub fn verify_native_partition_merges(
    before: Partition,
    after: Partition,
    targets: &[String],
    native_open_reduction: usize,
    additions: &[Expr],
) -> Result<OpenConnectivityProof, String> {
    let after_index = |partition: &Partition| -> Option<BTreeMap<String, usize>> {
        let mut at = BTreeMap::new();
        for (index, (net, members)) in partition.iter().enumerate() {
            if net.is_empty() || members.is_empty() {
                return None;
            }
            for member in members {
                if member.is_empty() || at.insert(member.clone(), index).is_some() {
                    return None;
                }
            }
        }
        Some(at)
    };
    let index = after_index(&after).ok_or("candidate has invalid native copper identities")?;
    let mut original = BTreeSet::new();
    let mut connected = BTreeSet::new();
    let mut target_before = 0;
    for (net, members) in before {
        target_before += usize::from(targets.contains(&net));
        let component = members
            .first()
            .and_then(|member| index.get(member))
            .copied()
            .ok_or("candidate is missing previously connected copper")?;
        if after[component].0 != net
            || (!targets.contains(&net) && after[component].1.len() != members.len())
            || !members
                .into_iter()
                .all(|member| index.get(&member) == Some(&component) && original.insert(member))
        {
            return Err("candidate splits a native component or changes an untouched net".into());
        }
        connected.insert(component);
    }
    let exact_added: BTreeSet<_> = additions.iter().filter_map(item_id).collect();
    if exact_added.len() != additions.len()
        || exact_added.iter().any(|id| original.contains(id))
        || !index
            .iter()
            .filter(|(id, _)| !original.contains(*id))
            .all(|(id, &component)| {
                exact_added.contains(id)
                    && connected.contains(&component)
                    && targets.contains(&after[component].0)
            })
        || index.len() != original.len() + exact_added.len()
    {
        return Err("candidate has unproven copper identities, net assignments or islands".into());
    }
    let target_after = after
        .iter()
        .filter(|(net, _)| targets.contains(net))
        .count();
    if native_open_reduction == 0
        || target_before.checked_sub(target_after) != Some(native_open_reduction)
    {
        return Err("native open reduction differs from the target net's merged components".into());
    }
    Ok(OpenConnectivityProof {
        prior_copper_objects: original.len(),
        added_copper_objects: exact_added.len(),
        target_components_before: target_before,
        target_components_after: target_after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn partition(groups: &[(&str, &[&str])]) -> Partition {
        groups
            .iter()
            .map(|(net, members)| {
                (
                    net.to_string(),
                    members.iter().map(|id| id.to_string()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn width_replay_accepts_another_witness_for_exactly_the_components_it_joins() {
        let source = partition(&[("V", &["cap", "via"]), ("V", &["ic"]), ("V", &["third"])]);
        let candidate = partition(&[("V", &["cap", "via", "ic"]), ("V", &["third"])]);
        let open = |a, b| {
            json!({
                "unconnected_items":[{
                    "type":"unconnected_items","severity":"error","items":[{"uuid":a},{"uuid":b}]
                }]
            })
        };
        let original = open("cap", "ic");
        let replay = open("ic", "via");
        let closed = json!({"unconnected_items":[]});
        assert!(
            verify_replayed_open_pairs(&source, &candidate, &original, &replay, &closed).is_some()
        );
        assert!(
            verify_replayed_open_pairs(
                &source,
                &candidate,
                &original,
                &open("ic", "third"),
                &closed
            )
            .is_none()
        );
        assert!(
            verify_replayed_open_pairs(&source, &source, &original, &replay, &closed).is_none()
        );
        assert!(
            verify_replayed_open_pairs(
                &source,
                &candidate,
                &original,
                &replay,
                &open("via", "third")
            )
            .is_none()
        );
    }

    #[test]
    fn multiple_named_nets_can_merge_without_changing_an_untouched_partition() {
        let before = partition(&[
            ("A", &["a1"]),
            ("A", &["a2"]),
            ("B", &["b1"]),
            ("B", &["b2"]),
            ("untouched", &["u1", "u2"]),
        ]);
        let after = partition(&[
            ("A", &["a1", "a2", "new_a"]),
            ("B", &["b1", "b2", "new_b"]),
            ("untouched", &["u1", "u2"]),
        ]);
        let additions = [
            parse("(segment (uuid new_a))").unwrap(),
            parse("(segment (uuid new_b))").unwrap(),
        ];
        assert!(
            verify_native_partition_merges(
                before.clone(),
                after.clone(),
                &["A".into(), "B".into()],
                2,
                &additions
            )
            .is_ok()
        );
        assert!(
            verify_native_partition_merges(before, after, &["A".into()], 2, &additions).is_err()
        );
    }

    #[test]
    fn accepts_only_monotonic_target_merges_with_exact_added_ids() {
        let before = partition(&[
            ("DATA", &["a"]),
            ("DATA", &["b", "c"]),
            ("GND", &["g1", "g2"]),
            ("GND", &["g3"]),
        ]);
        let added = [parse("(segment (uuid new))").unwrap()];
        let merge = ("DATA", &["a", "b", "c", "new"][..]);
        let after = partition(&[merge, ("GND", &["g1", "g2"]), ("GND", &["g3"])]);
        let proof = verify(before.clone(), after.clone(), "DATA", 1, &added).unwrap();
        assert_eq!(
            (
                proof.target_components_before,
                proof.target_components_after
            ),
            (2, 1)
        );
        assert!(verify(before.clone(), after.clone(), "GND", 1, &added).is_err());
        assert!(verify(before.clone(), after, "DATA", 2, &added).is_err());
        for damaged in [
            partition(&[merge, ("GND", &["g1", "g3"]), ("GND", &["g2"])]),
            partition(&[merge, ("GND", &["g1", "g2", "g3"])]),
            partition(&[
                ("DATA", &["a", "b", "c"]),
                ("DATA", &["new"]),
                ("GND", &["g1", "g2"]),
                ("GND", &["g3"]),
            ]),
            partition(&[
                ("DATA", &["a", "b", "new"]),
                ("GND", &["g1", "g2"]),
                ("GND", &["g3"]),
            ]),
        ] {
            assert!(verify(before.clone(), damaged, "DATA", 1, &added).is_err());
        }
    }
}
