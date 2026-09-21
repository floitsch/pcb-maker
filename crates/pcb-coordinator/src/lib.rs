// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Transactional coordinators sit outside individual placer/router strategies.
//! A failed experiment can propose semantic changes, but only an exact-gated
//! candidate becomes selected.

mod blocker_repair;
mod board_continuation;
mod conflict_action;
mod connection_insertion;
mod feedback_archive;
mod pressure_repair;
mod route_junction;
mod route_order_repair;
mod selective_ripup;

pub use blocker_repair::*;
pub use board_continuation::*;
pub use conflict_action::*;
pub use connection_insertion::*;
pub use feedback_archive::*;
pub use pressure_repair::*;
pub use route_junction::*;
pub use route_order_repair::*;
pub use selective_ripup::*;
