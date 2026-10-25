// Copyright (C) 2026 Toit contributors.

//! Two defects of a board that KiCad 10.0.7's DRC with zones refilled
//! cannot get past, repaired before it reads the board (`repair_file`,
//! from `native_report`). Both are the designer's, carried into every
//! board pcb-maker writes from theirs; boards without them are not
//! touched.
//!
//! **Copper zones of two nets that overlap at one priority.** KiCad
//! 10.0.7's zone filler can wait forever on such zones: on the 0xCB
//! pcb-panel (four copies of a keyboard, a KiKit panel whose copies'
//! pours reach 23 mm into their neighbours) `kicad-cli pcb drc
//! --refill-zones` sleeps after 15 s of work, its main thread polling and
//! every worker idle, until the 900 s limit; the designer's own board does
//! the same, and two of the panel's pours alone (Board_0-GND and
//! Board_1-GND) are enough. Not every such pair hangs (BAGEL-MK1's twelve
//! do not; two overlapping pours with pads in the overlap on a small test
//! board do not either), but distinct priorities (any order) let it
//! finish, and KiCad says what equal priorities mean for overlapping pours
//! of two nets nowhere: the fill of the overlap was undefined, so choosing
//! an order loses nothing the designer could rely on. Every copper zone's
//! priority `p` becomes `p * k + r`, where `r` is 0 for a zone in no pair
//! and, among the zones in pairs at one priority, larger for the zone
//! earlier in the file (so every order the board had stays, and the
//! earlier zone of a pair wins the overlap); `k` is one more than the
//! zones in pairs.
//!
//! **Edge.Cuts lines of no length.** KiCad finds no outline on a board
//! with one ("malformed outline (segment has null or very small length)")
//! and fills the zones to the board's bounding box, not to its outline:
//! the 0xCB pcb-panel (36 such lines among the mouse bites) took 504 s of
//! DRC on its placed board with distinct priorities, 210 s without them
//! (a machine at load 20; SmartSpin2k's panel has 115). A line from a
//! point to itself draws nothing, so it goes.

use super::*;

/// A copper zone: where it is among the board's items, its net, priority,
/// copper layers and outline.
struct Pour {
    item: usize,
    net: String,
    priority: u64,
    layers: Vec<String>,
    outline: Vec<[f64; 2]>,
}

/// What `separate` changed: the pairs found (net names, layer) and the new
/// priority of each zone in one.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Separated {
    pub pairs: Vec<(String, String, String)>,
}

impl Separated {
    pub(crate) fn describe(&self) -> String {
        let shown: Vec<String> = self
            .pairs
            .iter()
            .take(4)
            .map(|(a, b, layer)| format!("{a} and {b} on {layer}"))
            .collect();
        format!(
            "{} pairs of copper zones of two nets overlap at one priority ({}{}): KiCad 10.0.7's filler can wait forever on them, so their priorities were made distinct (the zone earlier in the file wins the overlap)",
            self.pairs.len(),
            shown.join(", "),
            if self.pairs.len() > shown.len() {
                ", ..."
            } else {
                ""
            }
        )
    }
}

fn zone_net(zone: &Expr) -> String {
    if let Some(name) = form_atom(zone, "net_name", 1) {
        return name.to_string();
    }
    // KiCad 10 names the net in the net form itself.
    form_atom(zone, "net", 1)
        .filter(|name| name.parse::<u64>().is_err())
        .unwrap_or("")
        .to_string()
}

fn zone_layers(zone: &Expr) -> Vec<String> {
    let mut layers = Vec::new();
    for head in ["layer", "layers"] {
        if let Some(form) = zone.child(head) {
            for layer in form.children().iter().skip(1).filter_map(Expr::atom) {
                match layer {
                    "F&B.Cu" => layers.extend(["F.Cu".to_string(), "B.Cu".to_string()]),
                    other if other.ends_with(".Cu") => layers.push(other.to_string()),
                    _ => {}
                }
            }
        }
    }
    layers
}

fn shared_layer(a: &[String], b: &[String]) -> Option<String> {
    if a.iter().any(|layer| layer == "*.Cu") {
        return b.first().cloned();
    }
    if b.iter().any(|layer| layer == "*.Cu") {
        return a.first().cloned();
    }
    a.iter().find(|layer| b.contains(layer)).cloned()
}

fn zone_outline(zone: &Expr) -> Vec<[f64; 2]> {
    let Some(points) = zone
        .child("polygon")
        .and_then(|polygon| polygon.child("pts"))
    else {
        return Vec::new();
    };
    points
        .children()
        .iter()
        .filter(|point| point.head() == Some("xy"))
        .filter_map(form_xy_point)
        .collect()
}

fn form_xy_point(point: &Expr) -> Option<[f64; 2]> {
    let x = point.children().get(1)?.atom()?.parse::<f64>().ok()?;
    let y = point.children().get(2)?.atom()?.parse::<f64>().ok()?;
    (x.is_finite() && y.is_finite()).then_some([x, y])
}

fn bounds(outline: &[[f64; 2]]) -> [f64; 4] {
    outline.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, p| {
            [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ]
        },
    )
}

fn cross(o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn inside(point: [f64; 2], polygon: &[[f64; 2]]) -> bool {
    let mut odd = false;
    for index in 0..polygon.len() {
        let (a, b) = (polygon[index], polygon[(index + 1) % polygon.len()]);
        if (a[1] > point[1]) != (b[1] > point[1])
            && point[0] < (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            odd = !odd;
        }
    }
    odd
}

/// Whether two simple polygons share area: two sides cross, or one holds
/// a corner of the other (sides that only touch do not count).
fn overlap(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    let (ba, bb) = (bounds(a), bounds(b));
    if ba[0] >= bb[2] || bb[0] >= ba[2] || ba[1] >= bb[3] || bb[1] >= ba[3] {
        return false;
    }
    for i in 0..a.len() {
        let (p, q) = (a[i], a[(i + 1) % a.len()]);
        for j in 0..b.len() {
            let (r, s) = (b[j], b[(j + 1) % b.len()]);
            let (d1, d2) = (cross(r, s, p), cross(r, s, q));
            let (d3, d4) = (cross(p, q, r), cross(p, q, s));
            if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
                return true;
            }
        }
    }
    inside(a[0], b) || inside(b[0], a)
}

fn pours(board: &Expr) -> Vec<Pour> {
    board
        .children()
        .iter()
        .enumerate()
        .filter(|(_, item)| item.head() == Some("zone") && item.child("keepout").is_none())
        .filter_map(|(item, zone)| {
            let layers = zone_layers(zone);
            let outline = zone_outline(zone);
            (!layers.is_empty() && outline.len() >= 3).then(|| Pour {
                item,
                net: zone_net(zone),
                priority: form_atom(zone, "priority", 1)
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0),
                layers,
                outline,
            })
        })
        .collect()
}

/// Gives `board`'s overlapping copper zones of two nets at one priority
/// distinct priorities (module documentation); `None` when it has none
/// (the board is not touched).
pub(crate) fn separate(board: &mut Expr) -> Option<Separated> {
    let pours = pours(board);
    let mut paired = vec![false; pours.len()];
    let mut separated = Separated::default();
    for i in 0..pours.len() {
        for j in i + 1..pours.len() {
            let (a, b) = (&pours[i], &pours[j]);
            if a.net.is_empty() || b.net.is_empty() || a.net == b.net || a.priority != b.priority {
                continue;
            }
            let Some(layer) = shared_layer(&a.layers, &b.layers) else {
                continue;
            };
            if overlap(&a.outline, &b.outline) {
                paired[i] = true;
                paired[j] = true;
                separated.pairs.push((a.net.clone(), b.net.clone(), layer));
            }
        }
    }
    if separated.pairs.is_empty() {
        return None;
    }
    let k = paired.iter().filter(|paired| **paired).count() as u64 + 1;
    // Among the paired zones of one priority, the earlier the larger r.
    let mut rank: BTreeMap<u64, u64> = BTreeMap::new();
    let mut new_priority = vec![0u64; pours.len()];
    for index in (0..pours.len()).rev() {
        let pour = &pours[index];
        let r = if paired[index] {
            let next = rank.entry(pour.priority).or_insert(0);
            *next += 1;
            *next
        } else {
            0
        };
        new_priority[index] = pour.priority * k + r;
    }
    let Expr::List(items) = board else {
        return None;
    };
    for (pour, priority) in pours.iter().zip(new_priority) {
        let Expr::List(zone) = &mut items[pour.item] else {
            continue;
        };
        let form = Expr::List(vec![
            Expr::Atom("priority".into()),
            Expr::Atom(priority.to_string()),
        ]);
        match zone.iter_mut().find(|item| item.head() == Some("priority")) {
            Some(existing) => *existing = form,
            None => {
                // After the net, layer and uuid forms, as KiCad writes it.
                let at = zone
                    .iter()
                    .position(|item| matches!(item.head(), Some("hatch" | "connect_pads")))
                    .unwrap_or(zone.len());
                zone.insert(at, form);
            }
        }
    }
    Some(separated)
}

/// Drops `board`'s top-level Edge.Cuts lines from a point to itself;
/// how many.
pub(crate) fn drop_null_edges(board: &mut Expr) -> usize {
    let Expr::List(items) = board else {
        return 0;
    };
    let before = items.len();
    items.retain(|item| {
        let null = item.head() == Some("gr_line")
            && form_atom(item, "layer", 1) == Some("Edge.Cuts")
            && item.child("start").and_then(form_xy_point)
                == item.child("end").and_then(form_xy_point)
            && item.child("start").is_some();
        !null
    });
    before - items.len()
}

/// The repairs of the board file `path` (module documentation), each
/// said in a line; the file is rewritten only when one applies.
pub(crate) fn repair_file(path: &Path) -> Result<Vec<String>, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let mut board = parse(&text)?;
    let mut said = Vec::new();
    let null_edges = drop_null_edges(&mut board);
    if null_edges > 0 {
        said.push(format!(
            "{null_edges} Edge.Cuts lines of no length removed (KiCad finds no outline with them and fills the zones to the board's box)"
        ));
    }
    if let Some(separated) = separate(&mut board) {
        said.push(separated.describe());
    }
    if !said.is_empty() {
        fs::write(path, encode(&board) + "\n")
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    }
    Ok(said)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(net: &str, priority: Option<u64>, layer: &str, x: [f64; 2]) -> String {
        format!(
            "(zone (net 1) (net_name \"{net}\") (layer \"{layer}\") (uuid \"u\") {}(hatch edge 0.5) (connect_pads (clearance 0.2)) (polygon (pts (xy {} 0) (xy {} 0) (xy {} 10) (xy {} 10))))",
            priority.map_or(String::new(), |p| format!("(priority {p}) ")),
            x[0],
            x[1],
            x[1],
            x[0]
        )
    }

    fn priorities(board: &Expr) -> Vec<String> {
        board
            .children()
            .iter()
            .filter(|item| item.head() == Some("zone"))
            .map(|zone| form_atom(zone, "priority", 1).unwrap_or("-").to_string())
            .collect()
    }

    #[test]
    fn overlapping_pours_of_two_nets_at_one_priority_get_distinct_priorities() {
        let text = format!(
            "(kicad_pcb {} {} {} {})",
            zone("A", Some(1), "F.Cu", [0.0, 6.0]),
            zone("B", Some(1), "F.Cu", [4.0, 10.0]),
            zone("C", Some(1), "F.Cu", [20.0, 30.0]),
            zone("D", Some(2), "F.Cu", [0.0, 30.0]),
        );
        let mut board = parse(&text).unwrap();
        let separated = separate(&mut board).unwrap();
        assert_eq!(
            separated.pairs,
            vec![("A".into(), "B".into(), "F.Cu".into())]
        );
        // k = 3: A (earlier) 1*3+2, B 1*3+1, C untouched in rank 1*3, D 2*3:
        // D still over all, C still below A and B's level.
        assert_eq!(priorities(&board), vec!["5", "4", "3", "6"]);
    }

    #[test]
    fn boards_without_such_a_pair_are_not_touched() {
        for text in [
            // The same net.
            format!(
                "(kicad_pcb {} {})",
                zone("A", None, "F.Cu", [0.0, 6.0]),
                zone("A", None, "F.Cu", [4.0, 10.0])
            ),
            // Different priorities.
            format!(
                "(kicad_pcb {} {})",
                zone("A", Some(1), "F.Cu", [0.0, 6.0]),
                zone("B", Some(2), "F.Cu", [4.0, 10.0])
            ),
            // Different layers.
            format!(
                "(kicad_pcb {} {})",
                zone("A", None, "F.Cu", [0.0, 6.0]),
                zone("B", None, "B.Cu", [4.0, 10.0])
            ),
            // Touching only.
            format!(
                "(kicad_pcb {} {})",
                zone("A", None, "F.Cu", [0.0, 6.0]),
                zone("B", None, "F.Cu", [6.0, 10.0])
            ),
            // A netless pour.
            format!(
                "(kicad_pcb {} {})",
                zone("", None, "F.Cu", [0.0, 6.0]),
                zone("B", None, "F.Cu", [4.0, 10.0])
            ),
        ] {
            let mut board = parse(&text).unwrap();
            assert_eq!(separate(&mut board), None, "{text}");
        }
    }

    #[test]
    fn a_missing_priority_is_written_and_both_sides_layers_count() {
        let text = format!(
            "(kicad_pcb {} {})",
            zone("A", None, "F&B.Cu", [0.0, 6.0]),
            zone("B", None, "B.Cu", [4.0, 10.0])
        );
        let mut board = parse(&text).unwrap();
        let separated = separate(&mut board).unwrap();
        assert_eq!(separated.pairs[0].2, "B.Cu");
        assert_eq!(priorities(&board), vec!["2", "1"]);
        assert!(separated.describe().contains("A and B on B.Cu"));
    }

    #[test]
    fn edge_lines_of_no_length_go() {
        let mut board = parse(
            "(kicad_pcb (gr_line (start 1 2) (end 1 2) (layer \"Edge.Cuts\")) (gr_line (start 1 2) (end 3 2) (layer \"Edge.Cuts\")) (gr_line (start 5 5) (end 5 5) (layer \"F.SilkS\")))",
        )
        .unwrap();
        assert_eq!(drop_null_edges(&mut board), 1);
        assert_eq!(count_head(&board, "gr_line"), 2);
        assert_eq!(drop_null_edges(&mut board), 0);
    }

    #[test]
    fn a_pour_inside_another_overlaps() {
        assert!(overlap(
            &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            &[[2.0, 2.0], [3.0, 2.0], [3.0, 3.0]]
        ));
    }
}
