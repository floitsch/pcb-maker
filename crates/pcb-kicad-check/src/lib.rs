// Copyright (C) 2026 Toit contributors.

//! KiCad's own checks and the machine they run on. `native_report` runs
//! kicad-cli (`pcb drc --schematic-parity`, `sch erc`, exports) with a
//! library-free configuration (`kicad_config`), bounded in time and
//! memory, and reads its reports; `drc_cache` keeps the reports on disk
//! by a key of everything they depend on; `verification` writes a routed
//! result's `erc.json`, `drc.json` and `verification.json`
//! (`verify_materialized_rung`), with `verification_preview`'s picture.
//! The machine: `ledger` is the machine-wide memory ledger that routes,
//! layouts and KiCad's checks reserve in before they allocate,
//! `proc_memory` reads what the machine and a process hold, and
//! `work_rate` turns work budgets into this machine's seconds. Nothing
//! here lowers a board or judges a result.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

// The KiCad files (pcb-kicad-file).
use pcb_kicad_file::{check_repairs, exit, fs_util, outline_pieces};
use pcb_kicad_file::{fs_util::*, kicad_nets::*, kicad_sexpr::*};

pub mod drc_cache;
pub mod kicad_config;
pub mod ledger;
pub mod native_report;
pub mod proc_memory;
pub mod verification;
pub mod verification_preview;
pub mod work_rate;

use native_report::{
    ReportKind, erc_design_issues, is_erc_library_warning, is_library_metadata_warning,
    remove_if_present, run_kicad_report, validate_report,
};
use pcb_kicad_file::copper_geometry::board_outline_checked;
use verification::VerificationReport;
