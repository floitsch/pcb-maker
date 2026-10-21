// Copyright (C) 2026 Toit contributors.

//! Whole-board routing through the in-memory `pcb-router` core.
//!
//! The board is parsed once, lowered once, routed entirely in memory, checked
//! by the core's exact verifier, and written once. Native KiCad verification
//! is a single final gate, never part of the routing loop.

use super::*;

mod attempt;
mod budget;
mod config;
mod geometry;
mod ladder;
mod lower;
mod memory;
mod pour_plan;
mod result;
mod route;
#[cfg(test)]
mod tests;
mod tools;

pub(crate) use self::{attempt::*, budget::*, geometry::*, ladder::*, lower::*, memory::*};
pub use self::{config::*, pour_plan::*, result::*, route::*, tools::*};
