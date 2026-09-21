use serde_json::Value;
use std::{collections::HashSet, fs, path::Path};

const BENCHMARK_CASES: &[&str] = &[
    "benchmarks/imported/layout-trace/squeeze.json",
    "benchmarks/imported/layout-trace/movable-route-blocker.json",
    "benchmarks/imported/layout-trace/multi-terminal-tree.json",
    "benchmarks/imported/layout-trace/decoupling-capacitor.json",
    "benchmarks/imported/layout-trace/forced-crossing-two-layer.json",
    "benchmarks/imported/layout-trace/placement-perturbation-unblocks-routing.json",
    "benchmarks/imported/layout-trace/esp32-slices/esp-pad-multi-contact-escape.json",
    "benchmarks/imported/layout-trace/dual-esp32-benchmark.json",
    "benchmarks/small/passage-pressure-asymmetric.json",
    "benchmarks/small/continuous-resistor-coupling.json",
    "benchmarks/small/continuous-multipin-tree.json",
    "benchmarks/small/conflict-action-orthogonal-crossing.json",
    "benchmarks/small/conflict-action-two-crossings.json",
    "benchmarks/small/conflict-action-shared-trace.json",
    "benchmarks/small/board-continuation-clear.json",
    "benchmarks/small/board-continuation-bottleneck.json",
    "benchmarks/small/board-continuation-local-repair.json",
    "benchmarks/small/board-continuation-force-motion.json",
    "benchmarks/small/board-continuation-explicit-rect-motion.json",
    "benchmarks/small/board-continuation-explicit-keepout-fallback.json",
];

#[test]
fn benchmark_layout_cases_are_valid_json_with_unique_component_ids() {
    for name in BENCHMARK_CASES {
        let path = Path::new(name);
        let source = fs::read_to_string(path).unwrap();
        let value: Value = serde_json::from_str(&source).unwrap();
        assert_eq!(value["schema_version"], 1, "{}", path.display());
        let components = value["components"].as_array().unwrap();
        let mut ids = HashSet::new();
        for component in components {
            assert!(ids.insert(component["id"].as_str().unwrap()));
        }
        assert!(
            value["nets"].is_array() || value["electrical_nets"].is_array(),
            "{} has no routable net declaration",
            path.display()
        );
    }
}

#[test]
fn benchmark_layout_cases_pass_the_ported_layout_trace_schema() {
    for name in BENCHMARK_CASES {
        let path = Path::new(name);
        let source = fs::read_to_string(path).unwrap();
        let problem: layout_trace_model::Problem = serde_json::from_str(&source).unwrap();
        problem
            .check_schema()
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    }
}

#[test]
fn distant_integration_targets_remain_available() {
    let dual: Value = serde_json::from_str(
        &fs::read_to_string("benchmarks/imported/layout-trace/dual-esp32-benchmark.json").unwrap(),
    )
    .unwrap();
    assert!(dual["components"].as_array().unwrap().len() >= 40);

    let schematic =
        fs::read_to_string("benchmarks/imported/testing-esp32-duts/esp32-c3/esp32-c3.kicad_sch")
            .unwrap();
    let pcb =
        fs::read_to_string("benchmarks/imported/testing-esp32-duts/esp32-c3/esp32-c3.kicad_pcb")
            .unwrap();
    assert!(schematic.starts_with("(kicad_sch"));
    assert!(pcb.starts_with("(kicad_pcb"));
}
