// Copyright (C) 2026 Toit contributors.

//! Moving a footprint to the other side of the board the way KiCad's flip
//! (top to bottom) does: mirrored across the footprint's own x axis,
//! orientation negated, front layers swapped with back layers.
//!
//! In a board file, pads, graphics and texts sit in the footprint's frame
//! (pad and text angles are absolute); zones inside a footprint are in
//! board coordinates.

use super::*;

/// Flips `footprint` about its origin to the other side of the board.
pub(super) fn flip_footprint(footprint: &mut Expr) -> Result<(), String> {
    let origin = form_at(footprint)?;
    let Expr::List(items) = footprint else {
        return Err("footprint is not a list".into());
    };
    for item in items.iter_mut().skip(1) {
        match item.head() {
            // The footprint's pose: position unchanged, orientation negated.
            Some("at") => negate(item, 3),
            Some("model") => {}
            // Board coordinates: mirrored across the footprint's origin.
            Some("zone") => mirror(item, origin[1]),
            Some(_) => mirror(item, 0.0),
            None => {}
        }
    }
    Ok(())
}

/// Mirrors every coordinate below `node` across the line y = `axis`, negates
/// angles and swaps sides.
fn mirror(node: &mut Expr, axis: f64) {
    let Expr::List(items) = node else {
        return;
    };
    match items.first().and_then(Expr::atom) {
        Some("at") => {
            reflect(items, 2, axis);
            // Angles turn the other way.
            reflect(items, 3, 0.0);
        }
        Some("start" | "end" | "mid" | "center" | "xy" | "offset") => reflect(items, 2, axis),
        // The trapezoid's widening along y turns around.
        Some("rect_delta") => reflect(items, 2, 0.0),
        Some("layer" | "layers") => {
            for atom in items.iter_mut().skip(1) {
                if let Some(layer) = atom.atom() {
                    let flipped = if let Some(rest) = layer.strip_prefix("F.") {
                        format!("B.{rest}")
                    } else if let Some(rest) = layer.strip_prefix("B.") {
                        format!("F.{rest}")
                    } else {
                        continue;
                    };
                    *atom = Expr::Atom(quote(&flipped));
                }
            }
        }
        Some("chamfer") => {
            for atom in items.iter_mut().skip(1) {
                let swapped = match atom.atom() {
                    Some("top_left") => "bottom_left",
                    Some("bottom_left") => "top_left",
                    Some("top_right") => "bottom_right",
                    Some("bottom_right") => "top_right",
                    _ => continue,
                };
                *atom = Expr::Atom(swapped.into());
            }
        }
        // Text on the back reads mirrored.
        Some("effects") => {
            let justify = items.iter().position(|item| item.head() == Some("justify"));
            match justify {
                Some(index) => {
                    let Expr::List(values) = &mut items[index] else {
                        unreachable!();
                    };
                    if let Some(mirrored) = values.iter().position(|value| value.atom() == Some("mirror")) {
                        values.remove(mirrored);
                        if values.len() == 1 {
                            items.remove(index);
                        }
                    } else {
                        values.push(Expr::Atom("mirror".into()));
                    }
                }
                None => items.push(Expr::List(vec![
                    Expr::Atom("justify".into()),
                    Expr::Atom("mirror".into()),
                ])),
            }
        }
        // Not geometry of the part.
        Some("model" | "uuid" | "stroke" | "size" | "net" | "font") => {}
        _ => {
            for item in items.iter_mut().skip(1) {
                mirror(item, axis);
            }
        }
    }
}

fn number(value: f64) -> Expr {
    // No "-0".
    Expr::Atom((value + 0.0).to_string())
}

/// y -> 2 axis - y for the atom at `index`.
fn reflect(items: &mut [Expr], index: usize, axis: f64) {
    if let Some(value) = items.get(index).and_then(Expr::atom).and_then(|atom| atom.parse::<f64>().ok()) {
        items[index] = number(2.0 * axis - value);
    }
}

fn negate(node: &mut Expr, index: usize) {
    if let Expr::List(items) = node {
        reflect(items, index, 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOOTPRINT: &str = r#"(footprint "Lib:Part" (layer "F.Cu") (at 10 20 90)
        (property "Reference" "U1" (at 0 -2 90) (layer "F.SilkS") (effects (font (size 1 1))))
        (fp_text user "x" (at 1 1 90) (layer "F.Fab") (effects (font (size 1 1)) (justify left mirror)))
        (fp_line (start -1 -1) (end 1 -1) (stroke (width 0.1) (type solid)) (layer "F.CrtYd"))
        (fp_arc (start -1 0) (mid 0 1) (end 1 0) (layer "F.SilkS"))
        (pad "1" smd roundrect (at -1 0.5 90) (size 1 0.5) (layers "F.Cu" "F.Mask" "F.Paste"))
        (pad "2" thru_hole trapezoid (at 1 0.5 180) (size 1 1) (rect_delta 0 0.2) (drill 0.6 (offset 0 0.1)) (layers "*.Cu" "*.Mask"))
        (pad "3" smd roundrect (at 2 1) (size 1 1) (chamfer top_left bottom_right) (layers "F.Cu"))
        (zone (layers "F.Cu") (polygon (pts (xy 10 18) (xy 12 18) (xy 12 19))))
        (model "part.step" (offset (xyz 0 1 0))))"#;

    #[test]
    fn a_flip_mirrors_the_part_and_swaps_its_layers() {
        let mut footprint = parse(FOOTPRINT).unwrap();
        flip_footprint(&mut footprint).unwrap();
        let text = encode(&footprint).split_whitespace().collect::<Vec<_>>().join(" ");
        for expected in [
            r#"(layer "B.Cu") (at 10 20 -90)"#,
            r#"(at 0 2 -90) (layer "B.SilkS") (effects (font (size 1 1)) (justify mirror))"#,
            r#"(at 1 -1 -90) (layer "B.Fab") (effects (font (size 1 1)) (justify left))"#,
            r#"(start -1 1) (end 1 1)"#,
            r#"(layer "B.CrtYd")"#,
            r#"(start -1 0) (mid 0 -1) (end 1 0)"#,
            r#"(at -1 -0.5 -90) (size 1 0.5) (layers "B.Cu" "B.Mask" "B.Paste")"#,
            r#"(at 1 -0.5 -180) (size 1 1) (rect_delta 0 -0.2) (drill 0.6 (offset 0 -0.1)) (layers "*.Cu" "*.Mask")"#,
            r#"(at 2 -1) (size 1 1) (chamfer bottom_left top_right)"#,
            r#"(xy 10 22) (xy 12 22) (xy 12 21)"#,
            r#"(model "part.step" (offset (xyz 0 1 0)))"#,
        ] {
            assert!(text.contains(expected), "{expected} missing in {text}");
        }
    }

    #[test]
    fn a_second_flip_restores_the_footprint() {
        let original = parse(FOOTPRINT).unwrap();
        let mut footprint = original.clone();
        flip_footprint(&mut footprint).unwrap();
        flip_footprint(&mut footprint).unwrap();
        let normal = |expression: &Expr| encode(expression).split_whitespace().collect::<Vec<_>>().join(" ");
        assert_eq!(normal(&footprint), normal(&original));
    }
}
