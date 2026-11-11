// Copyright (C) 2026 Toit contributors.

//! KiCad's files as data, below everything else on the KiCad side: the
//! S-expression tree with its parser and writer (`kicad_sexpr`), boards
//! read in one canonical order (`canonical_order`), nets (`kicad_nets`),
//! the geometry of pads and copper (`geometry`, `copper_geometry`,
//! `roundrect_pad`, `electrical_terminals`, `local_clearance`), the
//! outline (`outline`) and its pieces (`outline_pieces`: the sub-boards of
//! a panel, the router's net split, KiCad's check judged per sub-board),
//! copper written back (`copper_emit`, `route_graph`), a board's
//! statistics (`board_stats`), footprints flipped and moved (`flip`,
//! `part_motion`), the edits KiCad's check needs (`check_repairs`), file
//! utilities (`fs_util`) and how a run exits (`exit`: the exit codes and
//! the errors that are the input's). It knows the router's and the
//! placer's types but none of the adapters: nothing here lowers a board,
//! runs KiCad or decides a verdict.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub mod board_stats;
pub mod canonical_order;
pub mod check_repairs;
pub mod connection_rules;
pub mod copper_emit;
pub mod copper_geometry;
pub mod corridors_fact;
pub mod electrical_terminals;
pub mod exit;
pub mod flip;
pub mod footprint_bodies;
pub mod fs_util;
pub mod geometry;
#[cfg(test)]
mod geometry_queries;
pub mod kicad_nets;
pub mod kicad_sexpr;
pub mod local_clearance;
pub mod outline;
pub mod outline_pieces;
pub mod part_motion;
pub mod roundrect_pad;
pub mod route_graph;

use board_stats::*;
use copper_emit::*;
use copper_geometry::*;
use geometry::*;
use kicad_nets::*;
use kicad_sexpr::*;
use route_graph::*;
