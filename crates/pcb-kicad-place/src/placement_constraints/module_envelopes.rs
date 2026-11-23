// Copyright (C) 2026 Florian Loitsch.

//! Named mounted-module envelopes from explicitly selected footprint graphics.

use super::*;
use pcb_placer::constraints::{ModuleEnvelope, ModuleEnvelopePair};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadModuleNonoverlap {
    pub parts: Vec<String>,
    pub layer: String,
    #[serde(default)]
    pub min_gap_mm: f64,
}

fn layer_name(layer: &str) -> &str {
    match layer {
        "User.Eco1" => "Eco1.User",
        "User.Eco2" => "Eco2.User",
        _ => layer,
    }
}

fn footprint_outline(footprint: &Expr, layer: &str) -> Result<Vec<[f64; 2]>, String> {
    let mut drawings = vec![Expr::Atom("kicad_pcb".into())];
    for child in footprint
        .children()
        .iter()
        .filter(|child| form_atom(child, "layer", 1).map(layer_name) == Some(layer_name(layer)))
    {
        let Some(head @ ("fp_line" | "fp_rect" | "fp_poly" | "fp_arc" | "fp_circle" | "fp_curve")) =
            child.head()
        else {
            continue;
        };
        let mut graphic = child.children().to_vec();
        graphic[0] = Expr::Atom(format!("gr_{}", &head[3..]));
        if let Some(field) = graphic
            .iter_mut()
            .find(|field| field.head() == Some("layer"))
        {
            *field = parse(r#"(layer "Edge.Cuts")"#)?;
        }
        drawings.push(Expr::List(graphic));
    }
    if drawings.len() == 1 {
        return Err("no module contour graphics on this layer".into());
    }
    let loops = pcb_kicad_file::outline::board_loops_strict(&Expr::List(drawings))?;
    let [points] = loops.piece_loops.as_slice() else {
        return Err("use one closed module contour".into());
    };
    if !loops.cutouts.is_empty() || points.len() < 3 {
        return Err("use one closed module contour without cutouts".into());
    }
    // A convex polygon has every vertex on the same side of each edge.
    // This also rejects crossing outlines that could overstate the claim.
    let mut has_area = false;
    for (i, &[x, y]) in points.iter().enumerate() {
        let [a, b] = points[(i + 1) % points.len()];
        let (mut above, mut below) = (false, false);
        for &[u, v] in points {
            let cross = (a - x) * (v - y) - (b - y) * (u - x);
            above |= cross > 1.0e-8;
            below |= cross < -1.0e-8;
        }
        if above && below {
            return Err("use a convex module contour".into());
        }
        has_area |= above || below;
    }
    if !has_area {
        return Err("the module contour has no area".into());
    }
    Ok(points.clone())
}

pub(crate) fn apply_module_envelopes(
    pcb: &Expr,
    problem: &mut core::Problem,
    references: &[String],
    groups: &[KiCadModuleNonoverlap],
) -> Result<(), String> {
    let bad = |why: &str| crate::exit::input_error(format!("module_nonoverlap: {why}"));
    for group in groups {
        if group.parts.len() < 2
            || group.parts.iter().collect::<BTreeSet<_>>().len() != group.parts.len()
            || group.layer.trim().is_empty()
        {
            return Err(bad(
                "give two or more distinct exact references and a footprint graphics layer",
            ));
        }
        if !group.min_gap_mm.is_finite() || group.min_gap_mm < 0.0 {
            return Err(bad("min_gap_mm must be a finite nonnegative length"));
        }
        let mut modules = Vec::new();
        for reference in &group.parts {
            let mut indices = references
                .iter()
                .enumerate()
                .filter(|(_, name)| *name == reference);
            let Some((part, _)) = indices.next().filter(|_| indices.next().is_none()) else {
                return Err(bad(&format!("{reference} is missing or ambiguous")));
            };
            if problem
                .constraints
                .followers
                .iter()
                .any(|follower| follower.part == part || follower.leader == part)
            {
                return Err(bad(&format!(
                    "{reference} must be independently placed outside a row"
                )));
            }
            let mut footprints = pcb
                .children()
                .iter()
                .filter(|fp| matches!(fp.head(), Some("footprint" | "module")))
                .filter(|fp| footprint_reference(fp).as_deref() == Some(reference));
            let Some(footprint) = footprints.next().filter(|_| footprints.next().is_none()) else {
                return Err(bad(&format!("{reference} is missing or ambiguous")));
            };
            let back = match form_atom(footprint, "layer", 1) {
                Some("B.Cu") => true,
                Some("F.Cu") => false,
                _ => {
                    return Err(bad(&format!(
                        "{reference} has no front or back mounting face"
                    )));
                }
            };
            let vertices = footprint_outline(footprint, &group.layer)
                .map_err(|error| bad(&format!("{reference} on {}: {error}", group.layer)))?;
            modules.push(ModuleEnvelope {
                part,
                back,
                vertices,
            });
        }
        for (i, a) in modules.iter().enumerate() {
            for b in &modules[i + 1..] {
                problem
                    .constraints
                    .module_envelopes
                    .push(ModuleEnvelopePair {
                        modules: [a.clone(), b.clone()],
                        layer: layer_name(&group.layer).to_owned(),
                        min_gap_mm: group.min_gap_mm,
                    });
            }
        }
    }
    Ok(())
}

pub(super) fn report_module_envelopes(
    problem: &core::Problem,
    poses: &[core::Pose],
    references: &[String],
) -> impl Iterator<Item = KiCadConstraintStatus> {
    problem.constraints.module_envelopes.iter().map(move |pair| {
        let [first, second] = pair.modules.each_ref();
        let (overlap, gap) = pair.measurement(|part| poses[part]);
        let violation = gap.map_or(0.0, |gap| overlap + (pair.min_gap_mm - gap).max(0.0));
        let satisfied = violation <= 1.0e-6;
        let detail = match gap {
            None => format!("Mounted outlines on {} are on opposite faces.", pair.layer),
            Some(_) if overlap > 0.0 => format!(
                "Mounted outlines overlap on {}. Move or rotate a module to preserve a {:.3} mm gap.",
                pair.layer, pair.min_gap_mm,
            ),
            Some(gap) if !satisfied => format!(
                "Mounted outlines on {} are {:.3} mm apart. Move or rotate a module to reach {:.3} mm.",
                pair.layer, gap, pair.min_gap_mm,
            ),
            Some(gap) => format!(
                "Mounted outlines on {} are {:.3} mm apart (minimum {:.3} mm).",
                pair.layer, gap, pair.min_gap_mm,
            ),
        };
        KiCadConstraintStatus {
            kind: "module_nonoverlap".into(),
            part: references[first.part].clone(),
            other: Some(references[second.part].clone()),
            satisfied,
            violation_mm: if satisfied { 0.0 } else { violation },
            distance_mm: gap,
            pad_near: None,
            body_near_pin: None,
            near_geometry: None,
            detail: Some(detail),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_the_named_layer_and_ignores_its_text() {
        let footprint = parse(
            r#"(footprint "Test"
            (layer "F.Cu")
            (at 10 20 90)
            (fp_rect (start -2 -1) (end 4 2) (layer "User.Eco2"))
            (fp_rect (start -20 -10) (end 20 10) (layer "Eco1.User"))
            (fp_text user "Mounted on J1" (at 0 0) (layer "Eco2.User")))"#,
        )
        .unwrap();
        let outline = footprint_outline(&footprint, "Eco2.User").unwrap();
        assert_eq!(
            outline,
            [[-2.0, -1.0], [4.0, -1.0], [4.0, 2.0], [-2.0, 2.0]]
        );
        assert_eq!(footprint_outline(&footprint, "User.Eco2").unwrap(), outline);
        assert!(footprint_outline(&footprint, "Dwgs.User").is_err());
    }

    #[test]
    fn refuses_incomplete_or_unsupported_module_contours() {
        let footprint = |graphics| parse(&format!(r#"(footprint "Test" {graphics})"#)).unwrap();
        let missing_edge = footprint(
            r#"
            (fp_line (start 0 0) (end 4 0) (layer "Eco2.User"))
            (fp_line (start 4 0) (end 4 3) (layer "Eco2.User"))
            (fp_line (start 4 3) (end 0 3) (layer "Eco2.User"))"#,
        );
        assert!(footprint_outline(&missing_edge, "Eco2.User").is_err());
        let concave = footprint(
            r#"
            (fp_poly (pts (xy 0 0) (xy 4 0) (xy 2 1) (xy 4 3) (xy 0 3))
              (layer "Eco2.User"))"#,
        );
        assert_eq!(
            footprint_outline(&concave, "Eco2.User").unwrap_err(),
            "use a convex module contour"
        );
        let collapsed = footprint(
            r#"
            (fp_poly (pts (xy 0 0) (xy 2 2) (xy 4 4)) (layer "Eco2.User"))"#,
        );
        assert_eq!(
            footprint_outline(&collapsed, "Eco2.User").unwrap_err(),
            "the module contour has no area"
        );
    }
}
