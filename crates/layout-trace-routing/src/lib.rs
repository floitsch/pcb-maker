// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

// This crate is an audited import of the mature layout-trace routing kernel.
// Keep its implementation source stable while it is being integrated; these
// are style lints, not correctness or safety diagnostics.
#![allow(
    clippy::cloned_ref_to_slice_refs,
    clippy::collapsible_if,
    clippy::filter_map_bool_then,
    clippy::large_enum_variant,
    clippy::ptr_arg,
    clippy::question_mark,
    clippy::redundant_closure,
    clippy::result_unit_err,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::unnecessary_map_or
)]

pub use layout_trace_model::{geometry, model, topology};

pub mod combinatorial_regime;
pub mod context_correction;
pub mod copper_pair;
pub mod corridor;
pub mod family_assignment;
pub mod family_ir;
pub mod family_portfolio;
