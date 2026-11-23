// Copyright (C) 2026 Florian Loitsch.

use super::*;
use pcb_kicad_check::width_witness::{WidthWitnessChange, local_width_change};

#[derive(Serialize)]
pub struct OpenWidthReplay {
    #[serde(flatten)]
    pub witness: WidthWitnessChange,
    pub attempted_source_reports: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matching_source_report: Option<String>,
}

pub(super) struct WidthReplayResult {
    pub report: OpenWidthReplay,
    pub proof: Result<OpenConnectivityProof, String>,
}

impl connectivity::Trial<'_> {
    pub(super) fn replay_width(
        &self,
        before: &Value,
        after: &Value,
        parity: bool,
        trial_number: usize,
        budget: &mut Option<&mut LayoutRepairBudget<'_>>,
    ) -> Result<Option<WidthReplayResult>, String> {
        let Some(reduction) = before["unconnected_items"]
            .as_array()
            .unwrap()
            .len()
            .checked_sub(after["unconnected_items"].as_array().unwrap().len())
            .filter(|&reduction| reduction > 0)
        else {
            return Ok(None);
        };
        if !same_check_policy(before, after)
            || adds_findings(before, after, "unconnected_items")?
            || [before, after].into_iter().any(|report| {
                report_capped(report)
                    || !finding_types_at_cap(report, "schematic_parity").is_empty()
            })
        {
            return Ok(None);
        }
        for key in ["schematic_parity", ACROSS_KEY] {
            if finding_counts(before, key)? != finding_counts(after, key)? {
                return Ok(None);
            }
        }
        let repaired = fs::read_to_string(self.board).map_err(|error| error.to_string())?;
        let candidate = parse(&repaired)?;
        let mut baseline = parse(self.original)?;
        let Expr::List(items) = &mut baseline else {
            return Ok(None);
        };
        items.extend_from_slice(&self.additions[..self.previous]);
        let Some(witness) =
            local_width_change(&baseline, &candidate, before, after, Some(self.target))
        else {
            return Ok(None);
        };
        let directory = self
            .board
            .parent()
            .ok_or("missing native project directory")?;
        let inputs = project_companion_hashes(directory)?;
        let mut report = OpenWidthReplay {
            witness,
            attempted_source_reports: Vec::new(),
            matching_source_report: None,
        };
        let deadline = budget.as_ref().and_then(|budget| budget.router.deadline);
        for swapped in [false, true] {
            if budget
                .as_ref()
                .is_some_and(|budget| !budget.can_check_trials(3))
            {
                return Ok(Some(WidthReplayResult {
                    report,
                    proof: Err("remaining layout work does not cover the width witness replay and native connectivity checks".into()),
                }));
            }
            if swapped {
                let Some(ordered) = report.witness.swap_tracks(baseline.clone()) else {
                    break;
                };
                write_atomically(self.board, encode(&ordered).as_bytes())?;
            } else {
                write_additions(self.board, self.original, &self.additions[..self.previous])?;
            }
            let name = format!(
                "drc-open-width-source-{trial_number}-{}.json",
                if swapped { 2 } else { 1 }
            );
            let expected = fs::read(self.board).map_err(|error| error.to_string())?;
            if let Some(budget) = budget {
                budget.native_checks += 1;
            }
            let replay = native_report::check_kept_route_input(
                self.board,
                &directory.join(&name),
                parity,
                self.refill,
                || deadline.is_some_and(|limit| Instant::now() >= limit),
            );
            let unchanged = fs::read(self.board).is_ok_and(|bytes| bytes == expected);
            write_atomically(self.board, repaired.as_bytes())?;
            if !unchanged || project_companion_hashes(directory)? != inputs {
                return Err(
                    "KiCad changed the source board or project during the width witness replay"
                        .into(),
                );
            }
            let replay = match replay {
                Ok(replay) => replay,
                Err(error) => {
                    return Ok(Some(WidthReplayResult {
                        report,
                        proof: Err(error),
                    }));
                }
            };
            report.attempted_source_reports.push(name.clone());
            if same_check_policy(before, &replay)
                && before["coordinate_units"] == replay["coordinate_units"]
                && before["unconnected_items"].as_array().map(Vec::len)
                    == replay["unconnected_items"].as_array().map(Vec::len)
                && eligible_open_partition(&replay, after)?
            {
                report.matching_source_report = Some(name);
                let proof = self.prove(
                    &replay,
                    reduction,
                    budget,
                    Some(connectivity::ExpectedWitnesses::WidthReplay {
                        original: before,
                        candidate: after,
                    }),
                );
                if project_companion_hashes(directory)? != inputs {
                    return Err(
                        "KiCad changed the project during the native connectivity checks".into(),
                    );
                }
                return Ok(Some(WidthReplayResult { report, proof }));
            }
        }
        Ok(Some(WidthReplayResult {
            report,
            proof: Err("the unchanged source did not reproduce the alternate native width witness. Inspect the baseline, trial and replay reports before changing the copper".into()),
        }))
    }
}
