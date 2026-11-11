// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The stages of `place` (docs/placer.md, "Inside `place`"), in their
//! order:
//!
//! - `prepare` (start.rs): the halos fitted to the board, the linked
//!   pairs, the canonical angles and a diagnostic start's poses.
//! - `Placing::new` (start.rs): the budget, the anneal's settings and the
//!   bus groups; `Placing::global_stage` the global placement and the
//!   banks' arrangement.
//! - `Placing::walk_levels` (levels.rs): the relaxation levels, each an
//!   anneal and a legalisation or the kept placement of the level before,
//!   until every part has room.
//! - `Placing::conclude` (finish.rs): the refinement, relation repair and
//!   alignment of the best level's placement (`finish`), the pad exits and
//!   the result.

mod finish;
mod levels;
mod start;

pub(crate) use levels::{relax_level, relaxation_levels};
pub(crate) use start::{Placing, prepare};
