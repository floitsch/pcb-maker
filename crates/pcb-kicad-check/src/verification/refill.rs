// Copyright (C) 2026 Florian Loitsch.

//! Collect fill alongside an already-required fresh all-track DRC.

use super::*;
use crate::fs_util::{copy_directory_tree, temporary_next_to};
use crate::verified_fill_cache::{
    MAX_ENTRY_BYTES, VerifiedFillCache, layout_refill_identity, refill_identity,
};
use pcb_kicad_file::board_stats::is_rule_area;
use std::sync::Arc;

#[derive(Debug)]
pub(super) struct Snapshot {
    key: String,
    filled: String,
    ordinary_layout: bool,
}

impl Snapshot {
    pub(super) fn matches(&self, board: &Path) -> bool {
        identity(board, self.ordinary_layout).is_some_and(|(key, _)| self.key == key)
    }
}

fn identity(board: &Path, ordinary_layout: bool) -> Option<(String, String)> {
    if ordinary_layout {
        layout_refill_identity(board)
    } else {
        refill_identity(board)
    }
}

/// An in-process fresh refill from the selected DRC. The caller
/// still copies only the fill payload and runs an as-stored native check.
#[derive(Clone, Debug)]
pub struct CheckedRefill {
    pub(super) snapshot: Arc<Snapshot>,
    pub(super) drc: serde_json::Value,
    pub(super) report_name: &'static str,
    pub(super) report_sha256: String,
}

impl CheckedRefill {
    pub fn matches(&self, board: &Path) -> bool {
        self.snapshot.matches(board)
            && file_sha256(&board.with_file_name(self.report_name))
                .is_ok_and(|hash| hash == self.report_sha256)
    }

    pub fn from_ordinary_layout(&self) -> bool {
        self.snapshot.ordinary_layout
    }

    pub fn board(&self) -> &str {
        &self.snapshot.filled
    }

    pub fn drc(&self) -> &serde_json::Value {
        &self.drc
    }
}

pub(super) enum Capture {
    Skipped,
    Failed,
    Verified(serde_json::Value, Arc<Snapshot>),
}

pub(super) fn capture(project: &Path, board: &Path, report: &Path) -> Capture {
    capture_with(project, board, report, false)
}

pub(super) fn capture_layout(project: &Path, board: &Path, report: &Path) -> Capture {
    capture_with(project, board, report, true)
}

fn capture_with(project: &Path, board: &Path, report: &Path, ordinary_layout: bool) -> Capture {
    if !ordinary_layout
        && VerifiedFillCache::for_board(board).is_some_and(|cache| cache.has_entry())
    {
        return Capture::Skipped;
    }
    let Some((key, _)) = identity(board, ordinary_layout) else {
        return Capture::Skipped;
    };
    let Ok(source) = fs::read_to_string(board) else {
        return Capture::Skipped;
    };
    let has_fill = parse(&source).is_ok_and(|pcb| {
        if ordinary_layout
            && (!matches!(outline_pieces::Pieces::of_board(&pcb), Ok(None))
                || check_repairs::needs_repair(pcb.clone()))
        {
            return false;
        }
        pcb.children().iter().any(|zone| {
            zone.head() == Some("zone")
                && !is_rule_area(zone)
                && form_atom(zone, "fill", 1) != Some("no")
        })
    });
    let Some(name) = board.file_name().filter(|_| has_fill) else {
        return Capture::Skipped;
    };
    let stage = temporary_next_to(project);
    let mut ran_native = false;
    let capture = (|| -> Option<Capture> {
        copy_directory_tree(project, &stage).ok()?;
        let staged_board = stage.join(name);
        let staged_report = stage.join("captured-refill-drc.json");
        let matches =
            |board| identity(board, ordinary_layout).is_some_and(|(known, _)| known == key);
        if !matches(&staged_board) {
            return None;
        }
        ran_native = true;
        let drc = if ordinary_layout {
            native_report::run_layout_refill_drc(&staged_board, &staged_report)
        } else {
            native_report::run_fill_handoff_drc(&staged_board, &staged_report, true, true)
        }
        .ok()?;
        if fs::metadata(&staged_board).ok()?.len() > MAX_ENTRY_BYTES {
            return None;
        }
        let filled = fs::read_to_string(&staged_board).ok()?;
        // Put the original board back only in scratch to compare all
        // other current dependencies after native save.
        write_atomically(&staged_board, source.as_bytes()).ok()?;
        if !matches(&staged_board) || !matches(board) {
            return None;
        }
        write_atomically(report, &fs::read(staged_report).ok()?).ok()?;
        Some(Capture::Verified(
            drc,
            Arc::new(Snapshot {
                key,
                filled,
                ordinary_layout,
            }),
        ))
    })();
    let _ = fs::remove_dir_all(stage);
    capture.unwrap_or(if ran_native {
        Capture::Failed
    } else {
        Capture::Skipped
    })
}
