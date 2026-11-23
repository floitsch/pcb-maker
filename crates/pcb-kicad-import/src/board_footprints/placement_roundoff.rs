// Copyright (C) 2026 Florian Loitsch.

//! Comparison-only normalization for two-terminal SMD donor models.

use super::*;

pub(crate) fn same_two_pad_donor_model(first: &Expr, second: &Expr) -> bool {
    let (Some(first), Some(second)) = (comparison_model(first), comparison_model(second)) else {
        return false;
    };
    same_library_model_with_fab_text(&first, &second)
}

fn comparison_model(model: &Expr) -> Option<Expr> {
    let mut model = model.clone();
    let Expr::List(items) = &mut model else {
        return None;
    };
    let mut pins = items
        .iter()
        .filter(|item| item.head() == Some("pad"))
        .map(|item| item.children().get(1)?.atom())
        .collect::<Option<Vec<_>>>()?;
    pins.sort_unstable();
    if pins != ["1", "2"]
        || items
            .iter()
            .any(|item| matches!(item.head(), Some("group" | "zone")))
    {
        return None;
    }
    for item in &mut items[2..] {
        match item.head() {
            Some("pad") => {
                if item.children().get(2).and_then(Expr::atom) != Some("smd")
                    || item.children().get(3).and_then(Expr::atom) != Some("rect")
                    || item.child("padstack").is_some()
                    || item.child("drill").is_some()
                {
                    return None;
                }
                round_xy(item, "at")?;
            }
            Some("fp_line") if form_atom(item, "layer", 1) == Some("F.SilkS") => {
                if form_atom(item.child("stroke")?, "type", 1) != Some("solid") {
                    continue;
                }
                let start = round_xy(item, "start")?;
                let end = round_xy(item, "end")?;
                if start > end {
                    write_xy(item, "start", end)?;
                    write_xy(item, "end", start)?;
                }
            }
            Some("property") => normalize_hidden_fab_field(item),
            _ => {}
        }
    }
    Some(model)
}

fn field<'a>(item: &'a mut Expr, name: &str) -> Option<&'a mut Vec<Expr>> {
    let Expr::List(items) = item else {
        return None;
    };
    let Expr::List(values) = items.iter_mut().find(|item| item.head() == Some(name))? else {
        return None;
    };
    Some(values)
}

fn round_xy(item: &mut Expr, key: &str) -> Option<[i64; 2]> {
    let values = field(item, key)?;
    let tick = |index: usize| {
        let nm = values.get(index)?.atom()?.parse::<f64>().ok()? * 1e6;
        // KiCad board coordinates are integer nanometres. Bin adjacent
        // ticks together for comparison. A bin never spans two ticks.
        (nm.is_finite() && nm.abs() <= f64::from(i32::MAX) && (nm - nm.round()).abs() < 1e-6)
            .then_some(((nm.round() / 2.0).round() * 2.0) as i64)
    };
    let point = [tick(1)?, tick(2)?];
    write_xy(item, key, point)?;
    Some(point)
}

fn write_xy(item: &mut Expr, key: &str, [x, y]: [i64; 2]) -> Option<()> {
    let values = field(item, key)?;
    *values.get_mut(1)? = Expr::Atom(x.to_string());
    *values.get_mut(2)? = Expr::Atom(y.to_string());
    Some(())
}

fn normalize_hidden_fab_field(item: &mut Expr) {
    if !matches!(
        item.children().get(1).and_then(Expr::atom),
        Some("Footprint" | "Datasheet" | "Description")
    ) || !matches!(form_atom(item, "layer", 1), Some("F.Fab" | "B.Fab"))
        || form_atom(item, "hide", 1) != Some("yes")
        || item.child("render_cache").is_some()
    {
        return;
    }
    if let Some(layer) = field(item, "layer") {
        layer[1] = Expr::Atom(quote("F.Fab"));
    }
    if let Some(effects) = field(item, "effects")
        && let Some(index) = effects
            .iter()
            .position(|item| item.head() == Some("justify"))
        && let Expr::List(justify) = &mut effects[index]
    {
        justify.retain(|value| value.atom() != Some("mirror"));
        if justify.len() == 1 {
            effects.remove(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: &str = r#"(footprint "Part" (layer "F.Cu") (attr smd)
      (property "Footprint" "Board:Part" (at 0 0 180) (layer "F.Fab") (hide yes)
        (effects (font (size 1.27 1.27))))
      (fp_line (start 0 -0.349999) (end 0 0.349999) (stroke (width 0.12) (type solid)) (layer "F.SilkS"))
      (fp_line (start -1 1) (end 1 1) (stroke (width 0.12) (type solid)) (layer "F.SilkS"))
      (fp_rect (start -2 -1) (end 2 1) (stroke (width 0.05) (type solid)) (layer "F.CrtYd"))
      (pad "1" smd rect (at -0.949999 0 0) (size 0.8 1) (layers "F.Cu" "F.Paste" "F.Mask"))
      (pad "2" smd rect (at 0.949999 0 0) (size 0.8 1) (layers "F.Cu" "F.Paste" "F.Mask"))
      (model "model.step"))"#;

    fn equivalent_variant() -> String {
        MODEL
            .replace(
                "(at 0 0 180) (layer \"F.Fab\")",
                "(at 0 0) (layer \"B.Fab\")",
            )
            .replace(
                "(size 1.27 1.27))))",
                "(size 1.27 1.27)) (justify mirror)))",
            )
            .replace(
                "(start 0 -0.349999) (end 0 0.349999)",
                "(start 0 0.35) (end 0 -0.35)",
            )
            .replace("0.949999", "0.95")
    }

    #[test]
    fn side_and_coordinate_roundoff_are_comparison_only() {
        let base = parse(MODEL).unwrap();
        let changed = parse(&equivalent_variant()).unwrap();
        let original = encode(&changed);
        assert!(!same_library_model(&base, &changed));
        assert!(same_two_pad_donor_model(&base, &changed));
        assert_eq!(encode(&changed), original);

        let back = |mut library: Expr| {
            library = posed(&library, [17.0, 23.0, 133.0]).unwrap();
            crate::flip::flip_footprint(&mut library).unwrap();
            library_form(&library, "Part").unwrap()
        };
        assert!(same_two_pad_donor_model(&base, &back(changed)));
    }

    #[test]
    fn reports_different_pad_models_and_artwork() {
        let base = parse(MODEL).unwrap();
        let variant = equivalent_variant();
        for (from, to) in [
            ("0.95", "0.950001"),
            ("0.95", "0.9500001"),
            ("(size 0.8 1)", "(size 0.800001 1)"),
            ("\"F.Mask\"", "\"B.Mask\""),
            ("\"F.Cu\" \"F.Paste\"", "\"B.Cu\" \"F.Paste\""),
            ("(pad \"2\"", "(pad \"3\""),
            ("(pad \"2\"", "(pad \"1\""),
            ("(end 2 1)", "(end 2.000001 1)"),
            ("(start -1 1)", "(start -1.1 1)"),
            ("(end 1 1)", "(end 1.02 1)"),
            ("(hide yes)", "(hide no)"),
            ("\"model.step\"", "\"other-model.step\""),
        ] {
            let changed = parse(&variant.replace(from, to)).unwrap();
            assert!(!same_two_pad_donor_model(&base, &changed), "{to}");
        }
    }
}
