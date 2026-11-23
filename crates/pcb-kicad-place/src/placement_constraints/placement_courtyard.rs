// Copyright (C) 2026 Florian Loitsch.

//! Geometry for a missing courtyard in the placement model only.

use super::*;
use crate::exit::input_error;
use crate::footprint_bodies::on_back;
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadCourtModelSide {
    Front,
    Back,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadPlacementCourtyard {
    /// Exact footprint references with the same missing model.
    pub parts: Vec<String>,
    /// Side in the unflipped, front-mounted library model.
    pub model_side: KiCadCourtModelSide,
    /// Minimum X, minimum Y, maximum X, maximum Y in that model's frame.
    pub bounds_mm: [f64; 4],
}

#[derive(Default)]
pub(crate) struct CourtyardModel<'a> {
    by_reference: BTreeMap<&'a str, Vec<&'a KiCadPlacementCourtyard>>,
}

impl<'a> CourtyardModel<'a> {
    pub(crate) fn read(
        entries: &'a [KiCadPlacementCourtyard],
        footprints: impl Iterator<Item = &'a Expr>,
        hollow: &[String],
    ) -> Result<Self, String> {
        if entries.is_empty() {
            return Ok(Self::default());
        }
        let mut references = BTreeMap::<_, usize>::new();
        for reference in footprints.filter_map(footprint_reference) {
            *references.entry(reference).or_default() += 1;
        }
        let mut model = Self::default();
        for entry in entries {
            let [x0, y0, x1, y1] = entry.bounds_mm;
            if !entry.bounds_mm.iter().all(|value| value.is_finite()) || x0 >= x1 || y0 >= y1 {
                return Err(input_error(
                    "placement_courtyard needs a finite, positive-area rectangle",
                ));
            }
            if entry.parts.is_empty() {
                return Err(input_error(
                    "placement_courtyard needs at least one exact reference",
                ));
            }
            for part in &entry.parts {
                if references.get(part) != Some(&1) {
                    return Err(input_error(format!(
                        "placement_courtyard requires a unique footprint reference {part}"
                    )));
                }
                if hollow.iter().any(|pattern| glob_matches(pattern, part)) {
                    return Err(input_error(format!(
                        "placement_courtyard and hollow both apply to {part}"
                    )));
                }
                let prior = model.by_reference.entry(part).or_default();
                if prior
                    .iter()
                    .any(|existing| existing.model_side == entry.model_side)
                {
                    return Err(input_error(format!(
                        "placement_courtyard lists {part} more than once on a model side"
                    )));
                }
                prior.push(entry);
            }
        }
        Ok(model)
    }

    pub(crate) fn with_geometry<'b>(
        &self,
        source: &'b Expr,
        reference: &str,
    ) -> Result<Cow<'b, Expr>, String> {
        let Some(entries) = self.by_reference.get(reference) else {
            return Ok(Cow::Borrowed(source));
        };
        let back = match form_atom(source, "layer", 1) {
            Some("F.Cu") => false,
            Some("B.Cu") => true,
            _ => {
                return Err(input_error(format!(
                    "placement_courtyard cannot resolve the mounted side of {reference}"
                )));
            }
        };
        if back != on_back(source) {
            return Err(input_error(format!(
                "placement_courtyard found different mounted and pad sides on {reference}"
            )));
        }
        let mut copy = source.clone();
        for entry in entries {
            let layer = match (entry.model_side, back) {
                (KiCadCourtModelSide::Front, true) | (KiCadCourtModelSide::Back, false) => {
                    "B.CrtYd"
                }
                _ => "F.CrtYd",
            };
            if source.children().iter().any(|child| {
                child.head().is_some_and(|kind| kind.starts_with("fp_"))
                    && form_atom(child, "layer", 1) == Some(layer)
            }) {
                return Err(input_error(format!(
                    "placement_courtyard cannot replace {reference}'s authored {layer} artwork"
                )));
            }
            let [x0, y0, x1, y1] = entry.bounds_mm;
            let [y0, y1] = if back { [-y1, -y0] } else { [y0, y1] };
            let Expr::List(items) = &mut copy else {
                unreachable!("a footprint with a layer is a list");
            };
            items.push(parse(&format!(
                "(fp_rect (start {x0} {y0}) (end {x1} {y1}) (stroke (width 0) (type default)) (fill none) (layer \"{layer}\"))"
            ))?);
        }
        Ok(Cow::Owned(copy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board_placer::{KiCadBoardPlacerConfig, lower_placement};

    const NATIVE_OPPOSITE: &str = r#"(fp_rect (start -7 -6) (end 7 6) (layer "B.CrtYd"))"#;
    const DECLARED: &str = r#"(fp_rect (start -3 1) (end 4 5) (layer "F.CrtYd"))"#;

    fn board(declared_rectangle: bool, back: bool) -> Expr {
        let mut part = parse(&format!(
            r#"(footprint "test:fixture" (layer "F.Cu") (at 10 10 90)
                (property "Reference" "U1")
                (pad "1" smd rect (at 0 2 90) (size 1 1) (layers "F.Cu") (net "A"))
                (pad "2" thru_hole circle (at 2 3 90) (size 2 2) (drill 0.7) (layers "*.Cu") (net "B"))
                {NATIVE_OPPOSITE} {})"#,
            if declared_rectangle { DECLARED } else { "" },
        )).unwrap();
        if back {
            crate::flip::flip_footprint(&mut part).unwrap();
        }
        let mut pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
                (gr_rect (start 0 0) (end 25 25) (layer "Edge.Cuts")))"#,
        )
        .unwrap();
        if let Expr::List(items) = &mut pcb {
            items.push(part);
        }
        pcb
    }

    fn declarations() -> KiCadBoardPlacerConfig {
        KiCadBoardPlacerConfig {
            constraints: Some(KiCadConstraintsSource::Inline(KiCadPlacementConstraints {
                version: 1,
                placement_courtyard: vec![KiCadPlacementCourtyard {
                    parts: vec!["U1".into()],
                    model_side: KiCadCourtModelSide::Front,
                    bounds_mm: [-3.0, 1.0, 4.0, 5.0],
                }],
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    #[test]
    fn local_envelope_matches_a_native_courtyard_on_both_mounted_sides() {
        for back in [false, true] {
            let original = board(false, back);
            let untouched = original.clone();
            let supplied = lower_placement(&original, &declarations()).unwrap();
            assert_eq!(original, untouched);
            let expected =
                lower_placement(&board(true, back), &KiCadBoardPlacerConfig::default()).unwrap();
            let (part, native) = (
                &supplied.problem.components[0],
                &expected.problem.components[0],
            );
            assert_eq!(part.body_center, native.body_center);
            assert_eq!(part.body_center[1], if back { -3.0 } else { 3.0 });
            assert_eq!(part.body_size, native.body_size);
            assert_eq!(part.body_size, [7.0, 4.0]);
            assert_eq!(part.pads, native.pads);
            assert_eq!(part.far_copper, native.far_copper);
            assert_eq!(part.hole_rings, native.hole_rings);
            assert_eq!(part.far_courtyard, native.far_courtyard);
            assert_eq!(part.far_courtyard.len(), 1);
            assert_eq!(part.pins.len(), 2);
            assert_eq!(supplied.problem.poses[0], expected.problem.poses[0]);
        }
    }

    #[test]
    fn a_declaration_refuses_an_existing_native_courtyard() {
        let error = lower_placement(&board(true, true), &declarations())
            .err()
            .unwrap();
        assert!(error.contains("cannot replace U1's authored B.CrtYd"));
    }
}
