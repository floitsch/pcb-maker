// Copyright (C) 2026 Florian Loitsch.

//! Prove alternate KiCad width witnesses on the unchanged source geometry.

use super::*;
use std::collections::BTreeSet;

fn active_width(finding: &Value) -> bool {
    finding["type"] == "connection_width"
        && finding["severity"] == "warning"
        && finding["excluded"] != true
}

fn ordered(mut finding: Value) -> Value {
    if let Some(items) = finding["items"].as_array_mut() {
        items.sort_by_cached_key(Value::to_string);
    }
    finding
}

fn difference(before: &Value, after: &Value) -> Option<(Value, Value)> {
    let widths = |report: &Value| {
        report["violations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|finding| active_width(finding))
            .cloned()
            .map(ordered)
            .collect::<Vec<_>>()
    };
    let mut missing = Vec::new();
    let mut added = widths(after);
    for warning in widths(before) {
        if let Some(index) = added.iter().position(|trial| *trial == warning) {
            added.swap_remove(index);
        } else {
            missing.push(warning);
        }
    }
    if missing.len() != 1 || added.len() != 1 {
        return None;
    }
    Some((missing.pop()?, added.pop()?))
}

fn replace(report: &Value, prior: &Value, trial: &Value) -> Option<Value> {
    let mut report = report.clone();
    let prior = report["violations"]
        .as_array_mut()?
        .iter_mut()
        .find(|finding| ordered((*finding).clone()) == *prior)?;
    *prior = trial.clone();
    Some(report)
}

fn unchanged_same_net(
    source: &Expr,
    candidate: &Expr,
    prior: &Value,
    trial: &Value,
    target: &str,
    moved: &str,
) -> bool {
    let ids = |finding: &Value| {
        let entries = finding["items"].as_array()?;
        let ids = entries
            .iter()
            .map(|entry| Some(entry["uuid"].as_str()?.to_ascii_lowercase()))
            .collect::<Option<BTreeSet<_>>>()?;
        (entries.len() == 2 && ids.len() == 2).then_some(ids)
    };
    let (Some(old), Some(new)) = (ids(prior), ids(trial)) else {
        return false;
    };
    if prior["description"].as_str().is_none()
        || prior["description"] != trial["description"]
        || old.intersection(&new).count() != 1
    {
        return false;
    }
    let ids = old.union(&new).collect::<BTreeSet<_>>();
    fn objects<'a>(board: &'a Expr, ids: &BTreeSet<&String>) -> Option<BTreeMap<String, &'a Expr>> {
        let mut found = BTreeMap::new();
        for item in board.children() {
            let relevant = if item.head() == Some("footprint") {
                item.children()
            } else {
                std::slice::from_ref(item)
            };
            for item in relevant {
                if matches!(item.head(), Some("pad" | "segment" | "arc" | "via"))
                    && let Ok(id) = uuid(item)
                    && ids.contains(&id)
                    && found.insert(id, item).is_some()
                {
                    return None;
                }
            }
        }
        (found.len() == ids.len()).then_some(found)
    }
    let (Some(before), Some(after)) = (objects(source, &ids), objects(candidate, &ids)) else {
        return false;
    };
    let codes = net_codes(source);
    let blocker = index(source)
        .ok()
        .and_then(|items| items.get(moved).copied());
    let Some(blocker_net) = blocker.and_then(|via| item_net(via, &codes)) else {
        return false;
    };
    let Some(net) = before
        .values()
        .next()
        .and_then(|item| item_net(item, &codes))
    else {
        return false;
    };
    before == after
        && !net.is_empty()
        && ![target, blocker_net].contains(&net)
        && before
            .values()
            .all(|item| item_net(item, &codes) == Some(net))
}

/// A native width rule may report another pair of objects at the same
/// unchanged joint. Reproduce the complete trial finding on a private copy
/// of the source, changing only the iteration order of its routing forms.
pub(crate) struct Proof<'a> {
    pub source: &'a Expr,
    pub source_project: &'a Path,
    pub filename: &'a str,
    pub scratch: &'a Path,
    pub target: &'a str,
    pub moved: &'a str,
    pub try_third_order: bool,
}

impl Proof<'_> {
    pub fn replay(
        &self,
        candidate: &Expr,
        before: &Value,
        comparison: &Value,
        after: &Value,
        refill: bool,
        check: impl Fn(&Path, &str, bool) -> Result<Value, String>,
    ) -> Result<bool, String> {
        if [before, after].into_iter().any(report_capped) {
            return Ok(false);
        }
        let Some((prior, trial)) = difference(comparison, after) else {
            return Ok(false);
        };
        if !unchanged_same_net(
            self.source,
            candidate,
            &prior,
            &trial,
            self.target,
            self.moved,
        ) || !eligible_open_partition(
            &replace(comparison, &prior, &trial).ok_or("missing source width witness")?,
            after,
        )? {
            return Ok(false);
        }
        let expected = replace(before, &prior, &trial).ok_or("missing source width witness")?;
        if !self.scratch.is_dir() {
            copy_directory_tree(self.source_project, self.scratch)?;
        }
        let board = self.scratch.join(self.filename);
        let mut sample = self.source.clone();
        let Expr::List(items) = &mut sample else {
            return Ok(false);
        };
        let slots = items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| route(item).then_some(i))
            .collect::<Vec<_>>();
        let tracks = slots.iter().map(|&i| items[i].clone()).collect::<Vec<_>>();
        if tracks.len() < 2 {
            return Ok(false);
        }
        let mode = if refill { "refilled" } else { "stored" };
        let samples = if self.try_third_order { 3 } else { 2 };
        for (attempt, offset) in [1, tracks.len() / 2, 2]
            .into_iter()
            .take(samples)
            .enumerate()
        {
            let Expr::List(items) = &mut sample else {
                unreachable!()
            };
            for (n, &slot) in slots.iter().enumerate() {
                items[slot] = tracks[(n + offset) % tracks.len()].clone();
            }
            let encoded = encode(&sample);
            write_atomically(&board, encoded.as_bytes())?;
            let checked = check(
                &board,
                &format!("drc-relocation-width-source-{mode}-{}.json", attempt + 1),
                refill,
            )?;
            if read(&board)? != encoded
                || project_inputs(self.source_project)? != project_inputs(self.scratch)?
            {
                return Err("native width witness replay changed its project".into());
            }
            let opens = |report: &Value| {
                let mut list = report["unconnected_items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .cloned()
                    .map(ordered)
                    .collect::<Vec<_>>();
                list.sort_by_cached_key(Value::to_string);
                list
            };
            if opens(before) == opens(&checked) && eligible_open_partition(&expected, &checked)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_an_unchanged_joint_seen_in_the_native_source_can_match() {
        let make = |width| {
            parse(&format!(
                r#"(kicad_pcb
                    (via (net "blocker") (uuid via))
                    (segment (width {width}) (net "retained") (uuid common))
                    (segment (net "retained") (uuid tail))
                    (footprint (pad (net "retained") (uuid land))))"#
            ))
            .unwrap()
        };
        let source = make(0.1);
        let report = |id: &str, severity: &str| {
            json!({
                "violations": [{
                    "type": "connection_width",
                    "severity": severity,
                    "description": "0.0420 mm on F.Cu",
                    "items": [{"uuid": "common"}, {"uuid": id}]
                }],
                "schematic_parity": [],
                "unconnected_items": [],
                "unconnected_across_sub_boards": []
            })
        };
        let before = report("tail", "warning");
        let after = report("land", "warning");
        let Some((prior, trial)) = difference(&before, &after) else {
            panic!("expected one alternate width witness");
        };
        assert!(unchanged_same_net(
            &source, &source, &prior, &trial, "target", "via"
        ));
        assert!(!unchanged_same_net(
            &source,
            &make(0.09),
            &prior,
            &trial,
            "target",
            "via"
        ));
        assert!(difference(&before, &report("land", "error")).is_none());
        assert!(difference(&before, &before).is_none());
        assert!(!unchanged_same_net(
            &source, &source, &prior, &trial, "retained", "via"
        ));
    }

    #[test]
    fn a_dangling_repair_can_replay_a_third_native_width_witness() {
        let project = temporary_next_to(&std::env::temp_dir().join("width-replay"));
        let scratch = temporary_next_to(&project);
        fs::create_dir(&project).unwrap();
        let source = parse(
            r#"(kicad_pcb
            (segment (net "branch") (uuid leaf))
            (segment (net "retained") (uuid common))
            (via (net "retained") (uuid near))
            (footprint (pad (net "retained") (uuid pad))))"#,
        )
        .unwrap();
        let report = |anchor| json!({"violations":[{"severity":"warning","type":"connection_width","description":"0.0789 mm on F.Cu","items":[{"uuid":"common"},{"uuid":anchor}]}],"unconnected_items":[],"schematic_parity":[]});
        let before = report("near");
        let after = report("pad");
        let calls = Cell::new(0);
        let proof = Proof {
            source: &source,
            source_project: &project,
            filename: "test.kicad_pcb",
            scratch: &scratch,
            target: "branch",
            moved: "leaf",
            try_third_order: true,
        };
        let accepted = proof.replay(&source, &before, &before, &after, false, |_, _, _| {
            calls.set(calls.get() + 1);
            Ok(if calls.get() == 3 {
                after.clone()
            } else {
                before.clone()
            })
        });
        fs::remove_dir_all(project).unwrap();
        fs::remove_dir_all(scratch).unwrap();
        assert!(accepted.unwrap());
        assert_eq!(calls.get(), 3);
    }
}
