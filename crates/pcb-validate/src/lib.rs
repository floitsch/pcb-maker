//! Candidate interchange types and independent, fail-closed validation.

mod candidate;
mod electrical_connectivity;
mod geometry;

pub use candidate::*;
pub use electrical_connectivity::*;
pub use geometry::*;

use layout_trace_model::Problem;
use serde::Serialize;

pub const EXACT_VALIDATION_ASSESSMENT_CONTRACT: &str = "pcb-maker.exact-validation-assessment/v1";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExactValidationAssessment {
    pub contract: String,
    pub complete: bool,
    pub geometry: GeometryValidationAssessment,
    pub electrical: ElectricalConnectivityAssessment,
    pub violations: Vec<Violation>,
}

/// Apply every currently authoritative gate to a persisted candidate.
pub fn validate_candidate(
    problem: &Problem,
    candidate: &CandidateArtifact,
) -> Result<ExactValidationAssessment, String> {
    problem.check_schema()?;
    if candidate.schema_version != CANDIDATE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported candidate schema_version {}; expected {}",
            candidate.schema_version, CANDIDATE_SCHEMA_VERSION
        ));
    }
    let geometry = validate_geometry(
        problem,
        &candidate.components,
        &candidate.traces,
        &candidate.route_graphs,
    );
    let electrical = assess_electrical_connectivity(
        problem,
        &candidate.components,
        &candidate.traces,
        &candidate.route_graphs,
    );
    let mut violations = geometry.violations.clone();
    violations.extend(electrical.violations());
    violations.sort_by(|left, right| {
        (
            left.code.as_str(),
            left.objects.as_slice(),
            left.message.as_str(),
        )
            .cmp(&(
                right.code.as_str(),
                right.objects.as_slice(),
                right.message.as_str(),
            ))
    });
    Ok(ExactValidationAssessment {
        contract: EXACT_VALIDATION_ASSESSMENT_CONTRACT.into(),
        complete: geometry.complete && electrical.complete,
        geometry,
        electrical,
        violations,
    })
}

#[cfg(test)]
mod tests {
    use layout_trace_model::Vec2;

    use super::*;

    fn problem() -> Problem {
        serde_json::from_str(
            r#"{
                "schema_version":1,
                "board":{"bounds":{"min":{"x":0.0,"y":0.0},"max":{"x":10.0,"y":10.0}},"layers":[{"id":"top"}]},
                "rules":{"clearance":0.2,"via_diameter":0.8,"via_drill":0.4},
                "components":[{"id":"U1","position":{"x":5.0,"y":5.0},"size":{"x":1.0,"y":1.0},"constraints":{"movement":"fixed","rotation":"fixed"},"pins":[]}]
            }"#,
        )
        .unwrap()
    }

    fn candidate() -> CandidateArtifact {
        CandidateArtifact {
            schema_version: CANDIDATE_SCHEMA_VERSION,
            components: vec![SolvedComponent {
                id: "U1".into(),
                position: Vec2::new(5.0, 5.0),
                size: Vec2::new(1.0, 1.0),
                rotation_degrees: 0.0,
            }],
            traces: Vec::new(),
            route_graphs: Vec::new(),
        }
    }

    #[test]
    fn combined_gate_accepts_a_valid_placement_without_nets() {
        let result = validate_candidate(&problem(), &candidate()).unwrap();
        assert!(result.complete);
        assert!(result.violations.is_empty());
        assert_eq!(result.contract, EXACT_VALIDATION_ASSESSMENT_CONTRACT);
    }

    #[test]
    fn combined_gate_rejects_unknown_candidate_schema() {
        let mut candidate = candidate();
        candidate.schema_version += 1;
        assert!(validate_candidate(&problem(), &candidate).is_err());
    }
}
