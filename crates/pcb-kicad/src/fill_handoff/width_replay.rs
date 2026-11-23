// Copyright (C) 2026 Florian Loitsch.

use super::*;
use pcb_kicad_check::width_witness::{WidthWitnessChange, local_width_change};
use pcb_kicad_file::kicad_sexpr::{encode, parse};

#[derive(Debug, Serialize)]
pub struct FillWidthReplay {
    #[serde(flatten)]
    pub witness: WidthWitnessChange,
    pub source_refilled_report: &'static str,
    pub source_track_order: &'static str,
    pub saved_refilled_report: &'static str,
}

pub(super) struct Trial<'a> {
    pub board: &'a Path,
    pub original: &'a [u8],
    pub before: &'a Value,
    pub filled: &'a Value,
    pub stored: &'a Value,
    pub parity: bool,
}

impl Trial<'_> {
    pub(super) fn prove(
        self,
        native_checks: &mut usize,
        connectivity_checks: &mut usize,
    ) -> Result<Option<(FillWidthReplay, FillConnectivityProof)>, String> {
        if same_metadata(self.before, self.filled, self.stored).is_err()
            || [self.filled, self.stored].into_iter().any(|report| {
                !counts(report).possibly_capped_types.is_empty()
                    || report[crate::outline_pieces::CAPPED_KEY] == true
            })
            || ["unconnected_items", "schematic_parity"]
                .into_iter()
                .any(|key| keys(self.filled, key) != keys(self.stored, key))
        {
            return Ok(None);
        }
        let preserved = fs::read_to_string(self.board).map_err(|error| error.to_string())?;
        let source = parse(std::str::from_utf8(self.original).map_err(|error| error.to_string())?)?;
        let saved = parse(&preserved)?;
        let Some(witness) = local_width_change(&source, &saved, self.filled, self.stored, None)
        else {
            return Ok(None);
        };
        let directory = self
            .board
            .parent()
            .ok_or("missing native project directory")?;
        let inputs = layout::project_inputs(directory)?;
        let mut matched = None;
        for (order, name, probe) in [
            (
                "swapped_witnesses",
                "drc-width-source-replay.json",
                witness.swap_tracks(source.clone()),
            ),
            (
                "rotated_tracks",
                "drc-width-source-replay-rotated.json",
                WidthWitnessChange::rotate_tracks(source),
            ),
        ] {
            let Some(probe) = probe else {
                continue;
            };
            let probe = encode(&probe);
            fs_util::write_atomically(self.board, probe.as_bytes())?;
            *native_checks += 1;
            let replay = native_report::check_kept_route_input(
                self.board,
                &directory.join(name),
                self.parity,
                true,
                || false,
            );
            let unchanged = fs::read(self.board).is_ok_and(|bytes| bytes == probe.as_bytes());
            fs_util::write_atomically(self.board, preserved.as_bytes())?;
            if !unchanged || layout::project_inputs(directory)? != inputs {
                return Err(
                    "checking the original width witness changed the replay board or project"
                        .into(),
                );
            }
            let replay = replay?;
            if compare_reports(self.before, &replay, self.stored).is_ok() {
                matched = Some((order, name, replay));
                break;
            }
        }
        let Some((source_track_order, source_refilled_report, replay)) = matched else {
            return Err("the original geometry did not reproduce the complete stored width finding under either track order".into());
        };
        let evidence = FillWidthReplay {
            witness,
            source_refilled_report,
            source_track_order,
            saved_refilled_report: "drc-refilled-as-published.json",
        };
        *native_checks += 1;
        let reopened = native_report::check_kept_route_input(
            self.board,
            &directory.join(evidence.saved_refilled_report),
            self.parity,
            true,
            || false,
        )?;
        if compare_reports(self.before, self.filled, &reopened).is_err() {
            compare_reports(self.before, &replay, &reopened).map_err(|error| {
                format!("the reopened fill did not reproduce the original DRC: {error}")
            })?;
        }
        if !fs::read(self.board).is_ok_and(|bytes| bytes == preserved.as_bytes())
            || layout::project_inputs(directory)? != inputs
        {
            return Err("checking the width witness changed the saved board or project".into());
        }
        let connectivity = connectivity::prove(
            self.board,
            self.original,
            self.filled,
            self.stored,
            connectivity_checks,
            || false,
            Some(&reopened),
        )?;
        if layout::project_inputs(directory)? != inputs {
            return Err(
                "KiCad changed the project during the saved fill connectivity checks".into(),
            );
        }
        Ok(Some((evidence, connectivity)))
    }
}
