// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The congestion network, loaded from ONNX and run with `tract`.
//!
//! Contract with the training script (`experiments/congestion/train.py`):
//! input `features`, `[1, C, H, W]` raw channels (`features::channel_names`;
//! the normalisation is part of the graph), `H` and `W` multiples of
//! `PAD`; outputs `overflow`, `[1, P, H, W]` (per layer slot and the via
//! plane: predicted overflow per tile), and `open`, `[1, 1]` (predicted
//! `ln(1 + unfinished nets)` after a probe).

use crate::features::FeatureMaps;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use tract_onnx::prelude::*;

/// Sizes are padded to a multiple of this (three poolings of 2).
pub const PAD: usize = 8;

type Plan = TypedSimplePlan;

pub struct CongestionModel {
    model: InferenceModel,
    channels: usize,
    /// Optimised plans per padded input size.
    plans: Mutex<HashMap<(usize, usize), Arc<Plan>>>,
}

#[derive(Clone, Debug)]
pub struct Prediction {
    pub tiles_x: usize,
    pub tiles_y: usize,
    /// `[plane][tile_y][tile_x]`, cropped to the board's tiles.
    pub overflow: Vec<f32>,
    pub planes: usize,
    /// Predicted unfinished nets after a probe (from the scalar head).
    pub open: f32,
}

impl Prediction {
    /// Predicted overflow summed over the board.
    pub fn overflow_sum(&self) -> f32 {
        self.overflow.iter().map(|value| value.max(0.0)).sum()
    }
}

impl CongestionModel {
    pub fn load(path: &Path) -> Result<Self, String> {
        let model = tract_onnx::onnx()
            .model_for_path(path)
            .map_err(|error| format!("congestion model {}: {error}", path.display()))?;
        let channels = crate::features::channel_count();
        Ok(Self { model, channels, plans: Mutex::new(HashMap::new()) })
    }

    fn plan(&self, height: usize, width: usize) -> Result<Arc<Plan>, String> {
        if let Some(plan) = self.plans.lock().unwrap().get(&(height, width)) {
            return Ok(plan.clone());
        }
        let plan = self
            .model
            .clone()
            .with_input_fact(0, f32::fact([1, self.channels, height, width]).into())
            .and_then(|model| model.into_optimized())
            .and_then(|model| model.into_runnable())
            .map_err(|error| format!("congestion model for {width} x {height} tiles: {error}"))?;
        self.plans.lock().unwrap().insert((height, width), plan.clone());
        Ok(plan)
    }

    pub fn predict(&self, maps: &FeatureMaps) -> Result<Prediction, String> {
        let (width, height) = (maps.tiles_x.div_ceil(PAD) * PAD, maps.tiles_y.div_ceil(PAD) * PAD);
        let size = maps.tiles_x * maps.tiles_y;
        let mut input = vec![0.0f32; self.channels * width * height];
        for channel in 0..self.channels {
            for y in 0..maps.tiles_y {
                let from = channel * size + y * maps.tiles_x;
                let to = (channel * height + y) * width;
                input[to..to + maps.tiles_x].copy_from_slice(&maps.data[from..from + maps.tiles_x]);
            }
        }
        let plan = self.plan(height, width)?;
        let tensor = Tensor::from_shape(&[1, self.channels, height, width], &input).map_err(|error| error.to_string())?;
        let outputs = plan.run(tvec!(tensor.into())).map_err(|error| format!("congestion model: {error}"))?;
        let overflow = outputs[0].to_plain_array_view::<f32>().map_err(|error| error.to_string())?;
        let shape = overflow.shape().to_vec();
        let planes = shape[1];
        let mut cropped = Vec::with_capacity(planes * size);
        for plane in 0..planes {
            for y in 0..maps.tiles_y {
                for x in 0..maps.tiles_x {
                    cropped.push(overflow[[0, plane, y, x]]);
                }
            }
        }
        let open = outputs
            .get(1)
            .and_then(|tensor| tensor.to_plain_array_view::<f32>().ok().and_then(|view| view.iter().next().copied()))
            .map_or(0.0, |log| log.exp_m1());
        Ok(Prediction { tiles_x: maps.tiles_x, tiles_y: maps.tiles_y, overflow: cropped, planes, open })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixed tiny network of `experiments/congestion/export_tiny.py`
    /// (width 8, seed 0) on its test pattern gives what PyTorch computed.
    #[test]
    fn tiny_model_matches_pytorch() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/tiny.onnx");
        let model = CongestionModel::load(&path).unwrap();
        let channels = crate::features::channel_count();
        let (tiles_x, tiles_y) = (13, 10);
        let mut data = vec![0.0f32; channels * tiles_x * tiles_y];
        for c in 0..channels {
            for y in 0..tiles_y {
                for x in 0..tiles_x {
                    data[(c * tiles_y + y) * tiles_x + x] = ((7 * c + 3 * y + 5 * x) % 11) as f32 / 10.0;
                }
            }
        }
        let prediction = model.predict(&FeatureMaps { tiles_x, tiles_y, data }).unwrap();
        assert_eq!(prediction.planes, 5);
        let close = |value: f32, expected: f32| (value - expected).abs() <= 1.0e-3 * expected.abs().max(1.0);
        assert!(close(prediction.open, 83.306961), "open {}", prediction.open);
        let size = tiles_x * tiles_y;
        let sums: Vec<f32> = (0..5).map(|plane| prediction.overflow[plane * size..(plane + 1) * size].iter().sum()).collect();
        for (sum, expected) in sums.iter().zip([0.557891, 3.312456, 1.670501, 97.009605, 27.844288]) {
            assert!(close(*sum, expected), "plane sums {sums:?}");
        }
        let largest = prediction.overflow[(3 * tiles_y + 8) * tiles_x + 1];
        assert!(close(largest, 4.966362), "overflow[3, 8, 1] = {largest}");
        assert!(close(prediction.overflow_sum(), 130.394730));
    }
}
