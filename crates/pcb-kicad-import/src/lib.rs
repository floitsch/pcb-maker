// Copyright (C) 2026 Toit contributors.

//! `import-kicad-netlist`: a KiCad schematic's netlist, exported with
//! kicad-cli, turned into a new board (`netlist_import`: the parts and
//! their footprints from the project's libraries or from a reference
//! board, `board_footprints`), and the starters written beside it: a
//! `constraints.json` (`constraints_template`, and `key_matrix` for the
//! key matrix of a keyboard) and a `layout.json`. It sits beside the
//! layout flow, on the route and place crate (part bodies, the project's
//! rules, the placement constraints); nothing below it imports.

use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    process::Command,
};
// The tests' scratch directories.
#[cfg(test)]
use std::{
    env,
    time::{SystemTime, UNIX_EPOCH},
};

// The KiCad files (pcb-kicad-file), KiCad's checks (pcb-kicad-check) and
// a board routed and placed (pcb-kicad-route-place).
use pcb_kicad_check::native_report::KICAD_REQUIREMENT;
use pcb_kicad_check::{kicad_config, ledger, native_report};
use pcb_kicad_file::{canonical_order, exit, flip, geometry, outline};
use pcb_kicad_file::{fs_util::*, geometry::*, kicad_nets::*, kicad_sexpr::*};
use pcb_kicad_route_place::resolve_project_rules;
#[cfg(test)]
use pcb_kicad_route_place::{KiCadPlacementConstraints, read_placement_constraints};
use pcb_kicad_route_place::{board_placer, placement_constraints, project_rules, quality};

pub mod board_footprints;
pub mod constraints_template;
pub mod key_matrix;
pub mod netlist_import;

pub use constraints_template::constraints_template;
pub use key_matrix::{KiCadStarterHalf, KiCadStarterReport};
pub use netlist_import::{
    KiCadNetlistImport, import_kicad_netlist, import_kicad_netlist_from,
    import_kicad_netlist_options, import_kicad_netlist_with,
};
