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
//! R10), then uuid, then their canonical text; inside a footprint the pads
//! by number (natural), then position, the graphics (`fp_*`) by layer and
//! text without their uuid, the zones by text without their uuid. Each
//! kind keeps the places it had among the footprint's or board's items, so
//! the file's structure stays; everything else stays where it is. The
//! boards written from a sorted board are in that order too, so two saves
//! of one board give the same result file.

use super::*;
use std::cmp::Ordering;

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
            if index == bytes.len() || bytes[index].is_ascii_digit() != bytes[start].is_ascii_digit() {
                runs.push(&text[start..index]);
                start = index;
            }
        }
        runs
    }
    let (first, second) = (runs(a), runs(b));
    for (x, y) in first.iter().zip(&second) {
        let (x_digits, y_digits) = (x.as_bytes()[0].is_ascii_digit(), y.as_bytes()[0].is_ascii_digit());
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
        Expr::List(items) => encode(&Expr::List(
            items.iter().filter(|child| !matches!(child.head(), Some("uuid" | "tstamp"))).cloned().collect(),
        )),
        Expr::Atom(_) => encode(item),
    }
}

fn graphic(head: &str) -> bool {
    matches!(head, "fp_line" | "fp_rect" | "fp_arc" | "fp_circle" | "fp_poly" | "fp_curve" | "fp_text" | "fp_text_box")
}

/// Sorts the items of `items` for which `kind` is true among the places
/// they hold, by `compare` on a key computed once per item.
fn sort_in_place<K>(items: &mut [Expr], kind: impl Fn(&Expr) -> bool, key: impl Fn(&Expr) -> K, compare: impl Fn(&K, &K) -> Ordering) {
    let places: Vec<usize> = (0..items.len()).filter(|&index| kind(&items[index])).collect();
    if places.len() < 2 {
        return;
    }
    let mut keyed: Vec<(K, Expr)> = places
        .iter()
        .map(|&index| {
            let item = std::mem::replace(&mut items[index], Expr::List(Vec::new()));
            (key(&item), item)
        })
        .collect();
    keyed.sort_by(|(a, _), (b, _)| compare(a, b));
    for (place, (_, item)) in places.into_iter().zip(keyed) {
        items[place] = item;
    }
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
            let number = pad.children().get(1).and_then(Expr::atom).unwrap_or("").to_string();
            let at = |index: usize| form_atom(pad, "at", index).and_then(|value| value.parse::<f64>().ok()).unwrap_or(0.0);
            (number, at(1), at(2), content_text(pad), encode(pad))
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
        |item| (form_atom(item, "layer", 1).unwrap_or("").to_string(), content_text(item), encode(item)),
        |a, b| a.cmp(b),
    );
    sort_in_place(items, |item| item.head() == Some("zone"), |zone| (content_text(zone), encode(zone)), |a, b| a.cmp(b));
}

/// Sorts a board's footprints, and each footprint's own items (see the
/// module's comment). Idempotent; the rest of the board stays as it is.
pub(crate) fn canonical_order(pcb: &mut Expr) {
    let Expr::List(items) = pcb else {
        return;
    };
    for item in items.iter_mut().filter(|item| item.head() == Some("footprint")) {
        canonical_footprint(item);
    }
    sort_in_place(
        items,
        |item| item.head() == Some("footprint"),
        |footprint| {
            (
                footprint_reference(footprint).unwrap_or_default(),
                footprint_uuid(footprint).unwrap_or_default(),
                encode(footprint),
            )
        },
        |a, b| natural_cmp(&a.0, &b.0).then_with(|| a.1.cmp(&b.1)).then_with(|| a.2.cmp(&b.2)),
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
    fn natural_order_compares_digit_runs_by_value() {
        let mut names = vec!["R10", "R2", "C1", "R1", "U1A", "U1", "R02", "", "10", "9", "A1", "A10", "A2B"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, ["", "9", "10", "A1", "A2B", "A10", "C1", "R1", "R02", "R2", "R10", "U1", "U1A"]);
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
        let footprints: Vec<&Expr> = first.children().iter().filter(|item| item.head() == Some("footprint")).collect();
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
            let mut text = String::from(r#"(footprint "x" (layer "F.Cu") (property "Reference" "U1") (at 3 4)"#);
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
            .filter_map(|item| item.child("uuid").and_then(|uuid| uuid.children().get(1)).and_then(Expr::atom))
            .collect();
        assert_eq!(ids, ["p5", "p2", "p1", "p4", "p3", "g2", "g3", "g0", "g1"]);
        for order in [[8, 7, 6, 5, 4, 3, 2, 1, 0], [5, 0, 8, 1, 6, 2, 7, 3, 4]] {
            let other = footprint(&order);
            let pads = |footprint: &Expr| -> Vec<Expr> { footprint.children().iter().filter(|item| item.head() == Some("pad")).cloned().collect() };
            assert_eq!(pads(&other), pads(&first), "order {order:?}");
            let graphics = |footprint: &Expr| -> Vec<Expr> {
                footprint.children().iter().filter(|item| item.head().is_some_and(graphic)).cloned().collect()
            };
            assert_eq!(graphics(&other), graphics(&first), "order {order:?}");
        }
        // The pads keep the places the pads had: a footprint written pads
        // first stays so.
        assert!(first.children()[5..10].iter().all(|item| item.head() == Some("pad")));
    }
}
