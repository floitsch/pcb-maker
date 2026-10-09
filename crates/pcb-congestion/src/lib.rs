// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! A learned congestion predictor for placements (`docs/congestion-model.md`).
//!
//! `features` rasterizes a lowered board on the router's tiles; `model`
//! runs a small fully convolutional network (ONNX, through `tract`) on
//! those maps and predicts where negotiation will overflow and how many
//! nets a probe leaves unfinished.

pub mod features;
pub mod model;

pub use features::{FeatureMaps, channel_count, channel_names, rasterize};
pub use model::{CongestionModel, Prediction};
