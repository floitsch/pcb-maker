// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Shrinks a board for strength benchmarks: the outline and every position
//! on it move towards the outline's centre by a factor while sizes stay.
//! Footprints whose copper reaches the board edge keep their distance to
//! that edge, so connectors stay flush.

use super::*;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadShrinkReport {
    pub factor: f64,
    pub centre: [f64; 2],
    pub size_before_mm: [f64; 2],
    pub size_after_mm: [f64; 2],
    pub footprints: usize,
    /// Footprints pushed back inside the outline after scaling.
    pub edge_bound: Vec<String>,
}

pub fn shrink_kicad_board(
    source: &Path,
    output: &Path,
    factor: f64,
) -> Result<KiCadShrinkReport, String> {
    if !(factor > 0.0 && factor <= 1.0) {
        return Err(format!("shrink factor must be in (0, 1], got {factor}"));
    }
    let text = fs::read_to_string(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?;
    let mut pcb = parse(&text)?;
    let loops = outline::board_loops(&pcb)?;
    let mut minimum = [f64::INFINITY; 2];
    let mut maximum = [f64::NEG_INFINITY; 2];
    for point in &loops.outline {
        for axis in 0..2 {
            minimum[axis] = minimum[axis].min(point[axis]);
            maximum[axis] = maximum[axis].max(point[axis]);
        }
    }
    if !minimum[0].is_finite() {
        return Err("board has no outline".into());
    }
    let centre = [
        (minimum[0] + maximum[0]) / 2.0,
        (minimum[1] + maximum[1]) / 2.0,
    ];
    let scale = |point: [f64; 2]| -> [f64; 2] {
        [
            centre[0] + (point[0] - centre[0]) * factor,
            centre[1] + (point[1] - centre[1]) * factor,
        ]
    };
    let new_minimum = scale(minimum);
    let new_maximum = scale(maximum);

    let Expr::List(items) = &mut pcb else {
        return Err("PCB root is not a list".into());
    };
    let mut footprints = 0;
    let mut edge_bound = Vec::new();
    for item in items.iter_mut() {
        match item.head() {
            Some("footprint") => {
                footprints += 1;
                let at = form_at(item)?;
                let mut reach = [[f64::INFINITY, f64::NEG_INFINITY]; 2];
                for pad in item
                    .children()
                    .iter()
                    .filter(|child| child.head() == Some("pad"))
                {
                    let bounds = lower_pad(pad, at)?.geometry.aabb();
                    for axis in 0..2 {
                        reach[axis][0] = reach[axis][0].min(bounds.minimum[axis]);
                        reach[axis][1] = reach[axis][1].max(bounds.maximum[axis]);
                    }
                }
                let mut position = scale([at[0], at[1]]);
                let mut bound = false;
                for axis in 0..2 {
                    if !reach[axis][0].is_finite() {
                        continue;
                    }
                    // Copper stays inside the new outline, keeping its old
                    // margin where that was small (flush connectors stay
                    // flush); long parts along an edge are pushed back in.
                    let margin_low = (reach[axis][0] - minimum[axis]).clamp(0.0, 0.5);
                    let margin_high = (maximum[axis] - reach[axis][1]).clamp(0.0, 0.5);
                    let low = reach[axis][0] - at[axis];
                    let high = reach[axis][1] - at[axis];
                    if position[axis] + low < new_minimum[axis] + margin_low {
                        position[axis] = new_minimum[axis] + margin_low - low;
                        bound = true;
                    }
                    if position[axis] + high > new_maximum[axis] - margin_high {
                        position[axis] = new_maximum[axis] - margin_high - high;
                        bound = true;
                    }
                }
                if bound {
                    edge_bound.push(reference_of(item));
                }
                set_form_at(item, [round6(position[0]), round6(position[1]), at[2]])?;
            }
            Some("zone") => {
                if let Expr::List(children) = item {
                    children.retain(|child| child.head() != Some("filled_polygon"));
                }
                scale_points(item, &scale)?;
            }
            Some(head)
                if head.starts_with("gr_")
                    || matches!(head, "segment" | "arc" | "via" | "dimension") =>
            {
                scale_points(item, &scale)?;
            }
            _ => {}
        }
    }
    fs::write(output, format!("{}\n", encode(&pcb)))
        .map_err(|error| format!("failed to write {}: {error}", output.display()))?;
    Ok(KiCadShrinkReport {
        factor,
        centre,
        size_before_mm: [maximum[0] - minimum[0], maximum[1] - minimum[1]],
        size_after_mm: [
            new_maximum[0] - new_minimum[0],
            new_maximum[1] - new_minimum[1],
        ],
        footprints,
        edge_bound,
    })
}

fn round6(value: f64) -> f64 {
    (value * 1.0e6).round() / 1.0e6
}

fn reference_of(footprint: &Expr) -> String {
    footprint
        .children()
        .iter()
        .find(|child| {
            child.head() == Some("property")
                && child.children().get(1).and_then(Expr::atom) == Some("Reference")
        })
        .and_then(|property| property.children().get(2))
        .and_then(Expr::atom)
        .unwrap_or("?")
        .to_string()
}

/// Rewrites every `at`, `start`, `mid`, `end`, `center` and `xy` form under
/// `node` (recursively) through `scale`.
fn scale_points(node: &mut Expr, scale: &dyn Fn([f64; 2]) -> [f64; 2]) -> Result<(), String> {
    let Expr::List(items) = node else {
        return Ok(());
    };
    let head = items.first().and_then(Expr::atom).map(str::to_string);
    if matches!(
        head.as_deref(),
        Some("at" | "start" | "mid" | "end" | "center" | "xy")
    ) && items.len() >= 3
    {
        let x: f64 = items[1]
            .atom()
            .unwrap_or("")
            .parse()
            .map_err(|error| format!("bad coordinate: {error}"))?;
        let y: f64 = items[2]
            .atom()
            .unwrap_or("")
            .parse()
            .map_err(|error| format!("bad coordinate: {error}"))?;
        let point = scale([x, y]);
        items[1] = Expr::Atom(round6(point[0]).to_string());
        items[2] = Expr::Atom(round6(point[1]).to_string());
        return Ok(());
    }
    for child in items.iter_mut().skip(1) {
        scale_points(child, scale)?;
    }
    Ok(())
}
