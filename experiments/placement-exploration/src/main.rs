// Copyright (C) 2026 Toit contributors.
//! Experiment adapter. Calls production placement policies without modifying them.
use layout_trace_model::model::{Movement, Problem, Vec2};
use pcb_placement::{
    InitialPlacementConfig, InitialPlacementGenerator, PlacementPose, run_initial_placement,
    run_initial_placement_with_generator,
};
use serde_json::{Value, json};
use std::io::{self, BufRead};

struct Seed(Vec<PlacementPose>);
impl InitialPlacementGenerator for Seed {
    fn name(&self) -> &'static str {
        "experiment_junction_reinsertion"
    }
    fn propose(&self, _: &Problem, _: u64, _: usize) -> Result<Vec<PlacementPose>, String> {
        Ok(self.0.clone())
    }
}
fn run(v: Value) -> Value {
    let problem: Problem = serde_json::from_value(v["problem"].clone()).unwrap();
    let config: InitialPlacementConfig = serde_json::from_value(v["config"].clone()).unwrap();
    let start = std::time::Instant::now();
    let (result, junctions) = if v["junction"].as_bool().unwrap_or(false) {
        let mut reduced = problem.clone();
        let constraint_text = serde_json::to_string(&problem.placement_constraints).unwrap();
        let constraint_aware = v["constraint_aware"].as_bool().unwrap_or(false);
        for c in &mut reduced.components {
            let safe_to_shrink = c.constraints.movement == Movement::Free
                && c.constraints.region.is_none()
                && !constraint_text.contains(&format!("\"{}\"", c.id));
            if c.constraints.movement != Movement::Fixed && (!constraint_aware || safe_to_shrink) {
                c.size = Vec2::new(0.02, 0.02);
                c.routing_keepouts.clear();
                for p in &mut c.pins {
                    p.offset = Vec2::ZERO;
                    p.pads.clear();
                }
            }
        }
        match run_initial_placement(&reduced, &config) {
            Ok(seed) => {
                let junctions = serde_json::to_value(&seed.poses).unwrap();
                let result = run_initial_placement_with_generator(
                    &problem,
                    config.seed,
                    config.projection_sweeps,
                    &Seed(seed.poses),
                    true,
                );
                (result, junctions)
            }
            Err(e) => (Err(format!("junction seed: {e}")), Value::Null),
        }
    } else {
        (run_initial_placement(&problem, &config), Value::Null)
    };
    let elapsed = start.elapsed().as_secs_f64();
    match result {
        Ok(r) => json!({"ok":true,"result":r,"junctions":junctions,"seconds":elapsed}),
        Err(e) => json!({"ok":false,"error":e,"junctions":junctions,"seconds":elapsed}),
    }
}
fn main() {
    for line in io::stdin().lock().lines() {
        let value: Value = serde_json::from_str(&line.unwrap()).unwrap();
        println!("{}", run(value));
    }
}
