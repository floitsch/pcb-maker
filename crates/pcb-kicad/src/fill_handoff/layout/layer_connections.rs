// Copyright (C) 2026 Florian Loitsch.

//! KiCad's refill also records where zones flash plated pads and vias.

use super::{BTreeMap, BTreeSet, Expr, LayerTable, coordinate_nm, item_net};
use pcb_kicad_file::kicad_sexpr::form_at;

const CONNECTIONS: &str = "zone_layer_connections";

#[derive(Clone, Copy)]
struct Item<'a> {
    footprint: Option<&'a Expr>,
    node: &'a Expr,
    index: usize,
    pad_index: Option<usize>,
}

fn identities(node: &Expr) -> impl Iterator<Item = &str> + Clone {
    node.children()
        .iter()
        .filter(|part| matches!(part.head(), Some("uuid" | "tstamp")))
        .filter_map(|part| part.children().get(1)?.atom())
        .filter(|id| !id.is_empty())
}

fn index_items(board: &Expr) -> BTreeMap<&str, Option<Item<'_>>> {
    let mut indexed = BTreeMap::new();
    for (index, node) in board.children().iter().enumerate() {
        let top = Item {
            footprint: None,
            node,
            index,
            pad_index: None,
        };
        let nested = matches!(node.head(), Some("footprint" | "module"))
            .then(|| node.children().iter().enumerate())
            .into_iter()
            .flatten()
            .map(|(pad_index, pad)| Item {
                footprint: Some(node),
                node: pad,
                index,
                pad_index: Some(pad_index),
            });
        for item in std::iter::once(top).chain(nested) {
            let ids = identities(item.node);
            let unique = ids.clone().nth(1).is_none();
            for id in ids {
                indexed
                    .entry(id)
                    .and_modify(|entry| *entry = None)
                    .or_insert(unique.then_some(item));
            }
        }
    }
    indexed
}

fn unique_footprint<'a>(
    footprint: &'a Expr,
    indexed: &BTreeMap<&str, Option<Item<'_>>>,
) -> Option<&'a str> {
    let id = identities(footprint).next()?;
    indexed
        .get(id)?
        .filter(|item| std::ptr::eq(item.node, footprint))
        .map(|_| id)
}

fn unique_field<'a>(node: &'a Expr, name: &str) -> Option<Option<&'a Expr>> {
    let mut fields = node
        .children()
        .iter()
        .filter(|part| part.head() == Some(name));
    let field = fields.next();
    fields.next().is_none().then_some(field)
}

fn flag(node: &Expr, name: &str) -> Option<bool> {
    match unique_field(node, name)?.map(Expr::children) {
        None => Some(false),
        Some([_]) => Some(true),
        Some([_, value]) if value.atom() == Some("yes") => Some(true),
        Some([_, value]) if value.atom() == Some("no") => Some(false),
        _ => None,
    }
}

fn net<'a>(node: &'a Expr, codes: &'a BTreeMap<String, String>) -> Option<&'a str> {
    match unique_field(node, "net")?.map(Expr::children) {
        None => Some(""),
        Some([_, Expr::Atom(code)]) if code == "0" => Some(""),
        _ => item_net(node, codes),
    }
}

fn placement(node: &Expr) -> Option<(i32, i32, f64)> {
    let at = unique_field(node, "at")??.children();
    let angle = form_at(node).ok()?[2];
    Some((
        coordinate_nm(at.get(1)?)?,
        coordinate_nm(at.get(2)?)?,
        angle,
    ))
}

fn same_placement(a: &Expr, b: &Expr) -> bool {
    placement(a).is_some_and(|pose| placement(b) == Some(pose))
}

fn same_dimension_form(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::List(xs), Expr::List(ys)) => {
            xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| same_dimension_form(x, y))
        }
        _ => a == b || coordinate_nm(a).is_some_and(|x| Some(x) == coordinate_nm(b)),
    }
}

fn same_copper_shape(a: &Expr, b: &Expr) -> bool {
    let prefix = |node: &Expr| {
        node.children()
            .iter()
            .take_while(|part| part.atom().is_some())
            .count()
    };
    a.children()[..prefix(a)] == b.children()[..prefix(b)]
        && same_placement(a, b)
        && ["size", "drill"].iter().all(|name| {
            match (unique_field(a, name), unique_field(b, name)) {
                (Some(Some(a)), Some(Some(b))) => same_dimension_form(a, b),
                _ => false,
            }
        })
        && [
            "layers",
            "padstack",
            "roundrect_rratio",
            "chamfer",
            "chamfer_ratio",
            "rect_delta",
            "options",
            "primitives",
            "backdrill",
            "tertiary_drill",
            "front_post_machining",
            "back_post_machining",
            "zone_connect",
            "clearance",
            "thermal_gap",
            "thermal_width",
            "thermal_bridge_width",
            "thermal_bridge_angle",
        ]
        .iter()
        .all(|name| unique_field(a, name).is_some_and(|field| unique_field(b, name) == Some(field)))
        && flag(a, "keep_end_layers").is_some_and(|keep| flag(b, "keep_end_layers") == Some(keep))
}

fn flash_span<'a>(item: Item<'_>, stack: &'a LayerTable) -> Option<&'a [String]> {
    let node = item.node;
    if flag(node, "remove_unused_layers") != Some(true)
        || flag(node, "start_end_only") != Some(false)
    {
        return None;
    }
    match (node.head(), item.footprint) {
        (Some("pad"), Some(_)) if node.children().get(2)?.atom() == Some("thru_hole") => {
            Some(&stack.names)
        }
        (Some("via"), None) => {
            let [_, from, to] = unique_field(node, "layers")??.children() else {
                return None;
            };
            let from = stack.index(from.atom()?)?;
            let to = stack.index(to.atom()?)?;
            Some(&stack.names[from.min(to)..=from.max(to)])
        }
        _ => None,
    }
}

pub(super) fn copy(
    original: &mut Expr,
    native: &Expr,
    old_nets: &BTreeMap<String, String>,
    new_nets: &BTreeMap<String, String>,
    stack: &LayerTable,
) {
    let before = index_items(original);
    let after = index_items(native);
    let replacements: Vec<_> = before
        .iter()
        .filter_map(|(id, old)| {
            let old = (*old)?;
            let span = flash_span(old, stack)?;
            let new = (*after.get(id)?)?;
            let payload = unique_field(new.node, CONNECTIONS)??;
            if unique_field(old.node, CONNECTIONS)? == Some(payload)
                || flash_span(new, stack)? != span
                || net(old.node, old_nets)? != net(new.node, new_nets)?
                || !same_copper_shape(old.node, new.node)
            {
                return None;
            }
            let same_parent = match (old.footprint, new.footprint) {
                (None, None) => true,
                (Some(a), Some(b)) => {
                    unique_footprint(a, &before)? == unique_footprint(b, &after)?
                        && same_placement(a, b)
                        && unique_field(a, "layer")?? == unique_field(b, "layer")??
                }
                _ => false,
            };
            if !same_parent {
                return None;
            }
            let mut seen = BTreeSet::new();
            for layer in &payload.children()[1..] {
                let layer = layer.atom()?;
                if !span.iter().any(|name| name == layer) || !seen.insert(layer) {
                    return None;
                }
            }
            Some((old.index, old.pad_index, payload))
        })
        .collect();
    let Expr::List(items) = original else { return };
    for (index, pad_index, payload) in replacements {
        let node = &mut items[index];
        let node = match (pad_index, node) {
            (Some(index), Expr::List(parts)) => &mut parts[index],
            (_, item) => item,
        };
        let Expr::List(fields) = node else {
            unreachable!("board item is a list")
        };
        if let Some(old) = fields
            .iter_mut()
            .find(|part| part.head() == Some(CONNECTIONS))
        {
            *old = payload.clone();
        } else if let Some(layers) = fields.iter().rposition(|part| {
            matches!(
                part.head(),
                Some("layers" | "remove_unused_layers" | "keep_end_layers")
            )
        }) {
            // The reader uses the item's current layer set when it sees this form.
            fields.insert(layers + 1, payload.clone());
        }
    }
}
