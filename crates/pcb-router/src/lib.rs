// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! An in-memory whole-board PCB router.
//!
//! The board is lowered once into static per-layer maps; routed copper is
//! tracked in incrementally updated occupancy maps; nets negotiate for space
//! until the board is conflict free; and an exact geometric verifier checks
//! the emitted copper. No files or external processes are involved.

pub mod board;
pub mod geometry;
pub mod grid;
pub mod pour;
pub mod router;
pub mod verify;

pub use board::{
    Board, ClassId, LayerMask, Net, NetId, NetRoute, Obstacle, ObstacleKind, Plane, RuleClass,
    Segment,
    Terminal, Via,
};
pub use geometry::{Point, Shape};
pub use router::{Config, NetStatus, RoutingResult, route};
pub use verify::{Violation, verify};
