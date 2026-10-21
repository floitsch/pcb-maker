// Copyright (C) 2026 Toit contributors.

//! One order for a board's footprints and for each footprint's own items,
//! whatever order the file holds them in.
//!
//! The placer and the router iterate footprints and their pads and
//! graphics in the order of the file, and that order reaches the result:
//! part indices decide the annealer's moves, the legaliser's order and
//! every tie, net numbers follow the first pad seen, the router's
//! obstacles follow the graphics. KiCad's save, the netlist import and
//! the schematic flow each write another order (homebrew_486 with the same
//! angles in pcbnew's order put 103 of 130 parts elsewhere), so two saves
//! of one design placed and routed differently.
//!
//! Every command that places or routes a board sorts it first
//! ([`parse_board`]): footprints by reference (natural order: R2 before
//! R10), then uuid, then the file's order (never their text, which holds
//! their pose); inside a footprint the pads
//! by number (natural), then position, the graphics (`fp_*`) by layer and
//! text without their uuid, the zones by text without their uuid. Each
//! kind keeps the places it had among the footprint's or board's items, so
//! the file's structure stays; everything else stays where it is. The
//! boards written from a sorted board are in that order too, so two saves
//! of one board give the same result file.

use super::*;
use std::cmp::Ordering;

/// The order to write a footprint's items in, from its content alone (so
/// that a board is written alike whatever order it was read in): the
/// canonical order, but each courtyard's graphics, in the courtyard's
/// places, in the order KiCad's own save writes them
/// (`FOOTPRINT::cmp_drawings`: by shape, segments before rectangles,
/// arcs, circles and polygons, then start and end point, then the arc's
/// middle, the polygon's points and the stroke, then the uuid). KiCad
/// chains a courtyard from its graphics in file order: AccordeurDeGuitare's
/// MK1 (two zero-length lines and two arcs) closes only with the lines
/// first, and dacha's boards (every courtyard segment drawn twice, points
/// between) give KiCad's DRC more malformed courtyards in any other order.
/// `None`: write the items as they are held.
///
/// Until 2026-10-21 the writer restored each footprint's graphics in the
/// order of the file read, from a process-wide map keyed by the
/// footprint's reference and graphics: two files with one footprint in
/// two orders were written apart (the placer's order test failed alone
/// and passed or failed by timing beside the lowering test, which reads
/// the same footprints), the last file read won, and a turned footprint's
/// `fp_text` changed the key and lost the order it was kept for. Files
/// KiCad saved are in this order already, so the two agree on them.
pub(crate) fn source_order(footprint: &Expr) -> Option<Vec<usize>> {
    let items = footprint.children();
    let courtyard = |index: &usize| {
        items[*index].head().is_some_and(graphic)
            && form_atom(&items[*index], "layer", 1).is_some_and(|layer| layer.ends_with(".CrtYd"))
    };
    let places: Vec<usize> = (0..items.len()).filter(courtyard).collect();
    if places.len() < 2 {
        return None;
    }
    let mut sorted = places.clone();
    sorted.sort_by(|a, b| {
        let (shape_a, numbers_a, uuid_a) = save_key(&items[*a]);
        let (shape_b, numbers_b, uuid_b) = save_key(&items[*b]);
        shape_a
            .cmp(&shape_b)
            .then_with(|| {
                numbers_a
                    .iter()
                    .zip(&numbers_b)
                    .map(|(x, y)| x.total_cmp(y))
                    .find(|ordering| ordering.is_ne())
                    .unwrap_or_else(|| numbers_a.len().cmp(&numbers_b.len()))
            })
            .then_with(|| uuid_a.cmp(&uuid_b))
    });
    if sorted == places {
        return None;
    }
    let mut order: Vec<usize> = (0..items.len()).collect();
    for (place, item) in places.into_iter().zip(sorted) {
        order[place] = item;
    }
    Some(order)
}

/// A courtyard graphic's key in KiCad's save order: its shape's rank, the
/// numbers it compares by, its uuid.
fn save_key(item: &Expr) -> (u8, Vec<f64>, String) {
    let shape = match item.head() {
        Some("fp_line") => 0,
        Some("fp_rect") => 1,
        Some("fp_arc") => 2,
        Some("fp_circle") => 3,
        Some("fp_poly") => 4,
        Some("fp_curve") => 5,
        _ => 6,
    };
    let point = |name: &str| -> Vec<f64> {
        item.child(name)
            .map(|form| {
                form.children()
                    .iter()
                    .skip(1)
                    .take(2)
                    .filter_map(|value| value.atom().and_then(|atom| atom.parse::<f64>().ok()))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut numbers = Vec::new();
    if shape == 3 {
        numbers.extend(point("center"));
    } else {
        numbers.extend(point("start"));
    }
    numbers.extend(point("end"));
    numbers.extend(point("mid"));
    if let Some(pts) = item.child("pts") {
        numbers.push(pts.children().len() as f64);
        for xy in pts
            .children()
            .iter()
            .filter(|child| child.head() == Some("xy"))
        {
            numbers.extend(
                xy.children()
                    .iter()
                    .skip(1)
                    .take(2)
                    .filter_map(|value| value.atom().and_then(|atom| atom.parse::<f64>().ok())),
            );
        }
    }
    let width = item
        .child("stroke")
        .and_then(|stroke| form_atom(stroke, "width", 1))
        .or_else(|| form_atom(item, "width", 1))
        .and_then(|width| width.parse::<f64>().ok())
        .unwrap_or(0.0);
    numbers.push(width);
    let uuid = form_atom(item, "uuid", 1)
        .or_else(|| form_atom(item, "tstamp", 1))
        .unwrap_or("")
        .to_string();
    (shape, numbers, uuid)
}

/// Parses a `.kicad_pcb` text and sorts it ([`canonical_order`]).
pub(crate) fn parse_board(text: &str) -> Result<Expr, String> {
    let mut pcb = parse(text)?;
    canonical_order(&mut pcb);
    Ok(pcb)
}

/// Compares two strings in natural order: runs of digits by value (`R2`
/// before `R10`, `01` with `1`), other runs by bytes, a digit run before
/// another; equal by that, the bytes decide (so the order is total).
pub(crate) fn natural_cmp(a: &str, b: &str) -> Ordering {
    fn runs(text: &str) -> Vec<&str> {
        let mut runs = Vec::new();
        let mut start = 0;
        let bytes = text.as_bytes();
        for index in 1..=bytes.len() {
            if index == bytes.len()
                || bytes[index].is_ascii_digit() != bytes[start].is_ascii_digit()
            {
                runs.push(&text[start..index]);
                start = index;
            }
        }
        runs
    }
    let (first, second) = (runs(a), runs(b));
    for (x, y) in first.iter().zip(&second) {
        let (x_digits, y_digits) = (
            x.as_bytes()[0].is_ascii_digit(),
            y.as_bytes()[0].is_ascii_digit(),
        );
        let order = match (x_digits, y_digits) {
            (true, true) => {
                let (x, y) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                x.len().cmp(&y.len()).then_with(|| x.cmp(y))
            }
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => x.cmp(y),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    first.len().cmp(&second.len()).then_with(|| a.cmp(b))
}

/// The item's text without its own `uuid` / `tstamp`: what it is, not
/// which it is.
fn content_text(item: &Expr) -> String {
    match item {
        Expr::List(items) => encode_canonical(&Expr::List(
            items
                .iter()
                .filter(|child| !matches!(child.head(), Some("uuid" | "tstamp")))
                .cloned()
                .collect(),
        )),
        Expr::Atom(_) => encode_canonical(item),
    }
}

fn graphic(head: &str) -> bool {
    matches!(
        head,
        "fp_line"
            | "fp_rect"
            | "fp_arc"
            | "fp_circle"
            | "fp_poly"
            | "fp_curve"
            | "fp_text"
            | "fp_text_box"
    )
}

/// Sorts the items of `items` for which `kind` is true among the places
/// they hold, by `compare` on a key computed once per item. Stable: items
/// `compare` finds equal keep their order.
fn sort_in_place<K>(
    items: &mut [Expr],
    kind: impl Fn(&Expr) -> bool,
    key: impl Fn(&Expr) -> K,
    compare: impl Fn(&K, &K) -> Ordering,
) -> Vec<usize> {
    let places: Vec<usize> = (0..items.len())
        .filter(|&index| kind(&items[index]))
        .collect();
    if places.len() < 2 {
        return (0..places.len()).collect();
    }
    let mut keyed: Vec<(K, usize, Expr)> = places
        .iter()
        .enumerate()
        .map(|(source, &index)| {
            let item = std::mem::replace(&mut items[index], Expr::List(Vec::new()));
            (key(&item), source, item)
        })
        .collect();
    keyed.sort_by(|(a, _, _), (b, _, _)| compare(a, b));
    let mut sources = Vec::with_capacity(places.len());
    for (place, (_, source, item)) in places.into_iter().zip(keyed) {
        items[place] = item;
        sources.push(source);
    }
    sources
}

/// Sorts a footprint's pads, graphics and zones (see the module's
/// comment).
pub(crate) fn canonical_footprint(footprint: &mut Expr) {
    let Expr::List(items) = footprint else {
        return;
    };
    sort_in_place(
        items,
        |item| item.head() == Some("pad"),
        |pad| {
            let number = pad
                .children()
                .get(1)
                .and_then(Expr::atom)
                .unwrap_or("")
                .to_string();
            let at = |index: usize| {
                form_atom(pad, "at", index)
                    .and_then(|value| value.parse::<f64>().ok())
                    .unwrap_or(0.0)
            };
            (
                number,
                at(1),
                at(2),
                content_text(pad),
                encode_canonical(pad),
            )
        },
        |a, b| {
            natural_cmp(&a.0, &b.0)
                .then_with(|| a.1.total_cmp(&b.1))
                .then_with(|| a.2.total_cmp(&b.2))
                .then_with(|| a.3.cmp(&b.3))
                .then_with(|| a.4.cmp(&b.4))
        },
    );
    sort_in_place(
        items,
        |item| item.head().is_some_and(graphic),
        |item| {
            (
                form_atom(item, "layer", 1).unwrap_or("").to_string(),
                content_text(item),
                encode_canonical(item),
            )
        },
        |a, b| a.cmp(b),
    );
    let Expr::List(items) = footprint else {
        return;
    };
    sort_in_place(
        items,
        |item| item.head() == Some("zone"),
        |zone| (content_text(zone), encode_canonical(zone)),
        |a, b| a.cmp(b),
    );
}

/// Sorts a board's footprints, and each footprint's own items (see the
/// module's comment). Idempotent; the rest of the board stays as it is.
pub(crate) fn canonical_order(pcb: &mut Expr) {
    let Expr::List(items) = pcb else {
        return;
    };
    for item in items
        .iter_mut()
        .filter(|item| item.head() == Some("footprint"))
    {
        canonical_footprint(item);
    }
    // Ties (the same reference and the same or no uuid) keep the file's
    // order (the sort is stable). Their text used to break them, and it
    // holds the pose: a placed board read back could order two such
    // footprints the other way round from the board they were placed
    // from, and the layout, which writes poses by index, swap them.
    sort_in_place(
        items,
        |item| item.head() == Some("footprint"),
        |footprint| {
            (
                footprint_reference(footprint).unwrap_or_default(),
                footprint_uuid(footprint).unwrap_or_default(),
            )
        },
        |a, b| natural_cmp(&a.0, &b.0).then_with(|| a.1.cmp(&b.1)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn references(pcb: &Expr) -> Vec<String> {
        pcb.children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
            .map(|footprint| footprint_reference(footprint).unwrap_or_default())
            .collect()
    }

    #[test]
    fn footprint_courtyards_are_written_in_kicads_save_order() {
        // AccordeurDeGuitare's microphone MK1 (pixzrr/AccordeurDeGuitare,
        // carteCMS/cms-KiCad, Apache-2.0): its courtyard is two points and
        // two arcs, and KiCad closes it only in this order; sorted, the
        // arcs came first and KiCad's DRC found the courtyard malformed.
        let text = r#"(kicad_pcb (version 20240108)
          (footprint "accordeurEmp:micro-KECG2740PBJ" (layer "F.Cu") (at 131.55 155 90)
            (property "Reference" "MK1" (at 0 0 0) (layer "F.SilkS") (uuid "r"))
            (fp_line (start 0.5 -3.05) (end 0.5 -3.05) (stroke (width 0.1) (type solid)) (layer "F.CrtYd") (uuid "c0f5ce33-7126-4c44-b647-a7bf524c5f7a"))
            (fp_line (start 0.5 4.95) (end 0.5 4.95) (stroke (width 0.1) (type solid)) (layer "F.CrtYd") (uuid "9c06ef35-75e1-4290-a7ed-50cbf7c00a80"))
            (fp_arc (start 0.5 -3.05) (mid 4.5 0.95) (end 0.5 4.95) (stroke (width 0.1) (type solid)) (layer "F.CrtYd") (uuid "c24589b9-e11c-4bd5-bcdd-baae5114db0a"))
            (fp_arc (start 0.5 4.95) (mid -3.5 0.95) (end 0.5 -3.05) (stroke (width 0.1) (type solid)) (layer "F.CrtYd") (uuid "d37d713a-7f5a-44f3-a762-468e9ea0fe9c"))
            (fp_line (start 0.5 -2.05) (end 0.5 -2.05) (stroke (width 0.2) (type solid)) (layer "F.Fab") (uuid "60dedca2-f33b-49dd-8639-4b62dc5ae040"))
            (fp_line (start 0 -2.75) (end 0 -2.75) (stroke (width 0.2) (type solid)) (layer "F.SilkS") (uuid "331b257e-deb9-40a2-83e0-4fce98b82b56"))
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu"))))"#;
        let heads = |pcb: &Expr| -> Vec<String> {
            pcb.child("footprint")
                .unwrap()
                .children()
                .iter()
                .filter(|item| item.head().is_some_and(graphic))
                .map(|item| {
                    let uuid = item.child("uuid").unwrap().children()[1].atom().unwrap();
                    format!("{} {}", item.head().unwrap(), &uuid[..4])
                })
                .collect()
        };
        let source = parse(text).unwrap();
        let sorted = parse_board(text).unwrap();
        // Held in the canonical order: arcs before points, F.CrtYd first.
        assert_eq!(heads(&sorted)[..2], ["fp_arc c245", "fp_arc d37d"]);
        // Written with the courtyard in KiCad's save order (lines before
        // arcs, by their points), which is the file's here.
        let written = parse(&encode(&sorted)).unwrap();
        assert_eq!(heads(&written), heads(&source));
        assert_eq!(encode(&sorted), encode(&source));
        // Keys and digests see the canonical order.
        assert_ne!(encode_canonical(&sorted), encode(&sorted));
        // The same footprint read in another order is written alike: the
        // order comes from the content, not from the file read.
        let footprint_start = text.find("(fp_line").unwrap();
        let footprint_end = text.find("(pad ").unwrap();
        let mut graphics: Vec<&str> = text[footprint_start..footprint_end]
            .split("\n")
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        graphics.reverse();
        let reversed = format!(
            "{}{}\n            {}",
            &text[..footprint_start],
            graphics.join("\n            "),
            &text[footprint_end..]
        );
        assert_ne!(reversed, text);
        assert_eq!(encode(&parse_board(&reversed).unwrap()), encode(&sorted));
    }

    #[test]
    fn natural_order_compares_digit_runs_by_value() {
        let mut names = vec![
            "R10", "R2", "C1", "R1", "U1A", "U1", "R02", "", "10", "9", "A1", "A10", "A2B",
        ];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            [
                "", "9", "10", "A1", "A2B", "A10", "C1", "R1", "R02", "R2", "R10", "U1", "U1A"
            ]
        );
        // Total: equal by value, the bytes decide.
        assert_eq!(natural_cmp("R02", "R2"), Ordering::Less);
        assert_eq!(natural_cmp("R2", "R2"), Ordering::Equal);
    }

    #[test]
    fn footprints_sort_by_reference_then_uuid_and_keep_their_places() {
        let pcb = |order: &[usize]| {
            let footprints = [
                r#"(footprint "a" (property "Reference" "R10") (uuid "1") (at 0 0))"#,
                r#"(footprint "a" (property "Reference" "R2") (uuid "2") (at 0 0))"#,
                r#"(footprint "a" (property "Reference" "C1") (uuid "3") (at 0 0))"#,
                r#"(footprint "a" (property "Reference" "H") (uuid "b") (at 0 0))"#,
                r#"(footprint "a" (fp_text reference "H" (at 0 0)) (tstamp "a") (at 0 0))"#,
            ];
            let mut text = String::from("(kicad_pcb (version 1) (net 0 \"\")");
            for (index, &footprint) in order.iter().enumerate() {
                text.push_str(footprints[footprint]);
                if index == 1 {
                    text.push_str("(gr_line (start 0 0) (end 1 0) (layer \"Edge.Cuts\"))");
                }
            }
            text.push(')');
            parse_board(&text).unwrap()
        };
        let first = pcb(&[0, 1, 2, 3, 4]);
        assert_eq!(references(&first), ["C1", "H", "H", "R2", "R10"]);
        // The second H by its tstamp "a", the first by its uuid "b".
        let footprints: Vec<&Expr> = first
            .children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
            .collect();
        assert_eq!(footprint_uuid(footprints[1]).as_deref(), Some("a"));
        // The board's other items keep their places.
        assert_eq!(first.children()[3].head(), Some("footprint"));
        assert_eq!(first.children()[5].head(), Some("gr_line"));
        for order in [[4, 3, 2, 1, 0], [2, 0, 4, 1, 3], [1, 4, 0, 3, 2]] {
            assert_eq!(pcb(&order), first, "order {order:?}");
        }
        // Idempotent.
        let mut again = first.clone();
        canonical_order(&mut again);
        assert_eq!(again, first);
    }

    #[test]
    fn footprints_that_tie_keep_the_files_order_whatever_their_poses() {
        // Two footprints with one reference and no uuid (a hand-drawn
        // board's two "H" holes), alike but for their pose and a pad: the
        // file's order decides, not their text (which holds the pose), so
        // a placed board read back keeps the order of the board it was
        // placed from, and poses written by index stay with their
        // footprint.
        let board = |first: [f64; 2], second: [f64; 2]| {
            format!(
                "(kicad_pcb (version 1) (footprint \"h\" (property \"Reference\" \"H\") (at {} {}) (pad \"1\" np_thru_hole circle (at 0 0) (size 3 3))) (footprint \"h\" (property \"Reference\" \"H\") (at {} {}) (pad \"1\" np_thru_hole circle (at 0 0) (size 2 2))) (footprint \"r\" (property \"Reference\" \"A1\") (uuid \"u\") (at 0 0)))",
                first[0], first[1], second[0], second[1]
            )
        };
        let sizes = |pcb: &Expr| -> Vec<String> {
            pcb.children()
                .iter()
                .filter(|item| item.head() == Some("footprint"))
                .map(|item| {
                    item.child("pad")
                        .and_then(|pad| form_atom(pad, "size", 1))
                        .unwrap_or("-")
                        .to_string()
                })
                .collect()
        };
        // The first in the file at (9, 9): by its text it came second.
        let source = parse_board(&board([9.0, 9.0], [1.0, 1.0])).unwrap();
        assert_eq!(sizes(&source), ["-", "3", "2"]);
        // Placed with the poses exchanged: the order is the same.
        let placed = parse_board(&board([1.0, 1.0], [9.0, 9.0])).unwrap();
        assert_eq!(sizes(&placed), ["-", "3", "2"]);
        let written = parse_board(&crate::kicad_sexpr::encode(&placed)).unwrap();
        assert_eq!(sizes(&written), sizes(&placed));
    }

    #[test]
    fn pads_sort_by_number_then_position_and_graphics_by_layer_and_content() {
        let items = [
            r#"(pad "10" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (uuid "p1"))"#,
            r#"(pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (uuid "p2"))"#,
            r#"(pad "EP" smd rect (at 0 1) (size 1 1) (layers "F.Cu") (uuid "p3"))"#,
            r#"(pad "EP" smd rect (at -1 1) (size 1 1) (layers "F.Cu") (uuid "p4"))"#,
            r#"(pad "1" smd rect (at 5 0) (size 1 1) (layers "F.Cu") (uuid "p5"))"#,
            r#"(fp_line (start 0 0) (end 1 0) (layer "F.SilkS") (uuid "g1"))"#,
            r#"(fp_line (start 0 0) (end 2 0) (layer "F.CrtYd") (uuid "g2"))"#,
            r#"(fp_rect (start 0 0) (end 2 2) (layer "F.CrtYd") (uuid "g3"))"#,
            r#"(fp_line (start 0 0) (end 1 0) (layer "F.SilkS") (uuid "g0"))"#,
        ];
        let footprint = |order: &[usize]| {
            let mut text = String::from(
                r#"(footprint "x" (layer "F.Cu") (property "Reference" "U1") (at 3 4)"#,
            );
            for &index in order {
                text.push_str(items[index]);
            }
            text.push(')');
            let mut footprint = parse(&text).unwrap();
            canonical_footprint(&mut footprint);
            footprint
        };
        let first = footprint(&[0, 1, 2, 3, 4, 5, 6, 7, 8]);
        let ids: Vec<&str> = first
            .children()
            .iter()
            .filter_map(|item| {
                item.child("uuid")
                    .and_then(|uuid| uuid.children().get(1))
                    .and_then(Expr::atom)
            })
            .collect();
        assert_eq!(ids, ["p5", "p2", "p1", "p4", "p3", "g2", "g3", "g0", "g1"]);
        for order in [[8, 7, 6, 5, 4, 3, 2, 1, 0], [5, 0, 8, 1, 6, 2, 7, 3, 4]] {
            let other = footprint(&order);
            let pads = |footprint: &Expr| -> Vec<Expr> {
                footprint
                    .children()
                    .iter()
                    .filter(|item| item.head() == Some("pad"))
                    .cloned()
                    .collect()
            };
            assert_eq!(pads(&other), pads(&first), "order {order:?}");
            let graphics = |footprint: &Expr| -> Vec<Expr> {
                footprint
                    .children()
                    .iter()
                    .filter(|item| item.head().is_some_and(graphic))
                    .cloned()
                    .collect()
            };
            assert_eq!(graphics(&other), graphics(&first), "order {order:?}");
        }
        // The pads keep the places the pads had: a footprint written pads
        // first stays so.
        assert!(
            first.children()[5..10]
                .iter()
                .all(|item| item.head() == Some("pad"))
        );
    }
}
