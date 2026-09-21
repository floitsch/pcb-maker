use std::fs;

#[test]
fn persisted_candidate_passes_both_exact_gates() {
    let problem: layout_trace_model::Problem = serde_json::from_str(
        &fs::read_to_string("benchmarks/smoke/placement-only.problem.json").unwrap(),
    )
    .unwrap();
    let candidate: pcb_validate::CandidateArtifact = serde_json::from_str(
        &fs::read_to_string("benchmarks/smoke/placement-only.candidate.json").unwrap(),
    )
    .unwrap();
    let assessment = pcb_validate::validate_candidate(&problem, &candidate).unwrap();
    assert!(assessment.complete, "{:?}", assessment.violations);
}
