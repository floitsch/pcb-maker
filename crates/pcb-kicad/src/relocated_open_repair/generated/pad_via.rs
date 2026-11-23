// Copyright (C) 2026 Florian Loitsch.

//! Recover opposite-face access around a generated filled via in a large land.

use super::*;
use pcb_kicad_file::copper_emit::{
    SupplementalSegment, SupplementalVia, pad_via_expr, supplemental_segment_expr,
};
use pcb_kicad_file::copper_geometry::lower_pad;
use pcb_kicad_file::kicad_nets::footprint_reference;
use pcb_kicad_file::kicad_sexpr::form_at;
use pcb_kicad_route::board_router::LayerTable;
use pcb_kicad_route::open_repair::{lower_for_local, propose_generated_branch};
use pcb_router::{
    self as core,
    board::{NetRoute, Segment, Via},
    geometry::{Aabb, Shape},
};

const GRID_MM: f64 = 0.05;
const LAND_MARGIN_MM: f64 = 0.05;
const MAX_MOVE_MM: f64 = 2.0;
const NEAR_OPEN_MM: f64 = 2.0;
const MAX_MODEL_SEARCHES: usize = 2;

#[derive(Serialize)]
pub(in super::super) struct PadViaMove {
    uuid: String,
    pad_uuid: String,
    net: String,
    from_mm: Point,
    to_mm: Point,
    junction_mm: Point,
    junction_layer: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct GeneratedPadViaRetry {
    pub net: String,
    pub open_pad: String,
    pub open_pad_uuid: String,
    pub open_layer: String,
    pub open_at_mm: Point,
    pub via_uuid: String,
    pub via_net: String,
    pub via_at_mm: Point,
    pub via_mm: [f64; 2],
    pub host_pad: String,
    pub host_pad_uuid: String,
    pub host_layer: String,
    pub junction_mm: Point,
    pub junction_layer: String,
    pub retry_argv: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_evidence_limit: Option<String>,
}

struct Land {
    uuid: String,
    name: String,
    shape: Shape,
    center: Point,
    side: usize,
}

struct Blocker<'a> {
    slot: usize,
    uuid: String,
    net: &'a str,
    via: SupplementalVia,
    land: Land,
    pad_at: Point,
    open_pad_uuid: String,
    open_pad: String,
}

fn pad_label(owner: &Expr, pad: &Expr) -> Option<String> {
    Some(format!(
        "{}:{}",
        footprint_reference(owner)?,
        pad.children().get(1)?.atom()?
    ))
}

fn generated_pad_via(
    item: &Expr,
    placed: &BTreeSet<String>,
    codes: &BTreeMap<String, String>,
    names: &pcb_kicad_file::kicad_nets::NetNames,
) -> Option<(String, SupplementalVia)> {
    if item.head() != Some("via")
        || item.children().len() != 9
        || form_atom(item, "capping", 1) != Some("yes")
        || form_atom(item, "filling", 1) != Some("yes")
        || form_atom(item, "layers", 1) != Some("F.Cu")
        || form_atom(item, "layers", 2) != Some("B.Cu")
    {
        return None;
    }
    let uuid = uuid(item).ok()?;
    if placed.contains(&uuid) {
        return None;
    }
    let via = SupplementalVia {
        connection: names.normalize(item_net(item, codes)?).to_owned(),
        at: form_xy(item, "at").ok()?,
        size: form_f64(item, "size", 1).ok()?,
        drill: form_f64(item, "drill", 1).ok()?,
        layers: ["F.Cu".into(), "B.Cu".into()],
        kind: None,
    };
    let generated = pad_via_expr(&via);
    (self::uuid(&generated).ok()? == uuid && via.size > via.drill && via.drill > 0.0)
        .then_some((uuid, via))
}

fn opposite_lands(
    pcb: &Expr,
    layers: &LayerTable,
    side: usize,
    net: &str,
    via: &SupplementalVia,
    codes: &BTreeMap<String, String>,
) -> Vec<Land> {
    let opposite = layers.len() - 1 - side;
    pcb.children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .flat_map(|owner| {
            owner.children().iter().filter_map(move |pad| {
                if pad.head() != Some("pad")
                    || pad.children().get(2)?.atom()? != "smd"
                    || layers.pad_mask(pad).ok()? != 1 << opposite
                    || item_net(pad, codes) != Some(net)
                {
                    return None;
                }
                let lowered = lower_pad(pad, form_at(owner).ok()?).ok()?;
                let shape = lowered.geometry.router_shape();
                let bounds = shape.aabb();
                if (0..2).any(|i| bounds.maximum[i] - bounds.minimum[i] < 2.0)
                    || !shape.contains_disc(via.at, via.size / 2.0)
                {
                    return None;
                }
                Some(Land {
                    uuid: uuid(pad).ok()?,
                    name: pad_label(owner, pad)?,
                    shape,
                    center: lowered.center,
                    side: opposite,
                })
            })
        })
        .collect()
}

fn blocker<'a>(
    source: &'a Expr,
    placed: &Expr,
    layers: &LayerTable,
    witness: &Witness<'_>,
    target: &str,
    codes: &'a BTreeMap<String, String>,
) -> Option<Blocker<'a>> {
    let names = note_net_names(source);
    let placed: BTreeSet<_> = index(placed).ok()?.into_keys().collect();
    let mut alternatives = Vec::new();
    for (pad_at, pad_id) in std::iter::once((witness.pad, witness.pad_id)).chain(witness.other_pad)
    {
        let (owner, pad) = source
            .children()
            .iter()
            .filter(|owner| owner.head() == Some("footprint"))
            .flat_map(|owner| owner.children().iter().map(move |item| (owner, item)))
            .find(|(_, item)| {
                item.head() == Some("pad")
                    && uuid(item).is_ok_and(|id| id.eq_ignore_ascii_case(pad_id))
            })?;
        let actual = lower_pad(pad, form_at(owner).ok()?).ok()?;
        if core::geometry::distance(actual.center, pad_at) > 0.000002 {
            continue;
        }
        let pad_name = pad_label(owner, pad)?;
        let layers_mask = layers.pad_mask(pad).ok()?;
        if layers_mask.count_ones() != 1 {
            continue;
        }
        let side = layers_mask.trailing_zeros() as usize;
        if side != 0 && side + 1 != layers.len() {
            continue;
        }
        for (slot, item) in source.children().iter().enumerate() {
            let Some(net) = item_net(item, codes).filter(|net| *net != target) else {
                continue;
            };
            let Some((uuid, via)) = generated_pad_via(item, &placed, codes, &names) else {
                continue;
            };
            let distance = point_segment_distance_squared(pad_at, via.at, via.at);
            if distance > NEAR_OPEN_MM.powi(2) {
                continue;
            }
            let mut lands = opposite_lands(source, layers, side, net, &via, codes).into_iter();
            if let Some(land) = lands.next().filter(|_| lands.next().is_none()) {
                alternatives.push((
                    distance,
                    Blocker {
                        slot,
                        uuid,
                        net,
                        via,
                        land,
                        pad_at,
                        open_pad_uuid: pad_id.to_owned(),
                        open_pad: pad_name.clone(),
                    },
                ));
            }
        }
    }
    alternatives
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.slot.cmp(&b.1.slot)))
        .map(|(_, via)| via)
}

/// Only one opposite-layer branch, with an existing exact junction. All old
/// tracks stay in place. Native connectivity verifies every retained member.
fn junction(
    source: &Expr,
    blocker: &Blocker<'_>,
    layers: &LayerTable,
    codes: &BTreeMap<String, String>,
) -> Option<(Point, usize, f64)> {
    let mut contact = Vec::new();
    for item in source
        .children()
        .iter()
        .filter(|item| route(item) && item_net(item, codes) == Some(blocker.net))
    {
        let layer = form_atom(item, "layer", 1).and_then(|name| layers.index(name));
        if layer == Some(blocker.land.side) || item.head() == Some("via") {
            continue;
        }
        let [a, b] = [form_xy(item, "start").ok()?, form_xy(item, "end").ok()?];
        let width = form_f64(item, "width", 1).ok()?;
        if point_segment_distance_squared(blocker.via.at, a, b)
            <= ((blocker.via.size + width) / 2.0).powi(2)
        {
            if item.head() != Some("segment") {
                return None;
            }
            contact.push((a, b, layer?, width));
        }
    }
    let &(a, b, layer, width) = contact.first()?;
    if contact
        .iter()
        .any(|item| item.2 != layer || item.3 != width)
    {
        return None;
    }
    let min = |p: Point| point_segment_distance_squared(blocker.land.center, p, p);
    let connected = |p: Point| {
        contact
            .iter()
            .filter(|(a, b, ..)| *a == p || *b == p)
            .count()
            >= 2
    };
    let anchor = contact
        .iter()
        .flat_map(|(a, b, ..)| [*a, *b])
        .filter(|&p| connected(p))
        .min_by(|&a, &b| min(a).total_cmp(&min(b)))
        .or_else(|| (contact.len() == 1).then_some(if min(a) < min(b) { a } else { b }))?;
    // The junction must be at the old via, rather than another point
    // reached farther down the net.
    (point_segment_distance_squared(blocker.via.at, anchor, anchor)
        <= (blocker.via.size / 2.0 + width / 2.0).powi(2))
    .then_some((anchor, layer, width))
}

fn sites(blocker: &Blocker<'_>) -> Vec<Point> {
    let old = blocker.via.at;
    let delta = [old[0] - blocker.pad_at[0], old[1] - blocker.pad_at[1]];
    let axis = usize::from(delta[1].abs() > delta[0].abs());
    let direction = delta[axis].signum();
    let mut exit = blocker.pad_at;
    exit[axis] = old[axis] + direction * (blocker.via.size / 2.0 + LAND_MARGIN_MM);
    let clearance = |p| point_segment_distance_squared(p, blocker.pad_at, exit);
    let mut sites = Vec::new();
    let steps = (MAX_MOVE_MM / GRID_MM).round() as i32;
    for i in -steps..=steps {
        for j in -steps..=steps {
            let at = [
                old[0] + f64::from(i) * GRID_MM,
                old[1] + f64::from(j) * GRID_MM,
            ]
            .map(|v| (v * 1e6).round() / 1e6);
            if f64::from(i * i + j * j) * GRID_MM.powi(2) <= MAX_MOVE_MM.powi(2)
                && (at[axis] - old[axis]) * direction >= 0.0
                && clearance(at) > (blocker.via.size + 0.1).powi(2)
                && blocker
                    .land
                    .shape
                    .contains_disc(at, blocker.via.size / 2.0 + LAND_MARGIN_MM)
            {
                sites.push(at);
            }
        }
    }
    let to_center = |p| point_segment_distance_squared(p, blocker.land.center, blocker.land.center);
    sites.sort_by(|&a, &b| {
        clearance(b)
            .total_cmp(&clearance(a))
            .then(to_center(a).total_cmp(&to_center(b)))
    });
    sites
}

pub(super) fn suggestion(
    source: &Expr,
    placed: &Expr,
    target: &str,
    witness: &Witness<'_>,
    retry_argv: Vec<String>,
) -> Option<GeneratedPadViaRetry> {
    let layers = LayerTable::from_pcb(source).ok()?;
    let codes = net_codes(source);
    let blocker = blocker(source, placed, &layers, witness, target, &codes)?;
    let (anchor, layer, _) = junction(source, &blocker, &layers, &codes)?;
    if sites(&blocker).is_empty() {
        return None;
    }
    let host = blocker.land.side;
    Some(GeneratedPadViaRetry {
        net: target.to_owned(),
        open_pad: blocker.open_pad,
        open_pad_uuid: blocker.open_pad_uuid,
        open_layer: layers.names[layers.len() - 1 - host].clone(),
        open_at_mm: blocker.pad_at,
        via_uuid: blocker.uuid,
        via_net: blocker.net.to_owned(),
        via_at_mm: blocker.via.at,
        via_mm: [blocker.via.size, blocker.via.drill],
        host_pad: blocker.land.name,
        host_pad_uuid: blocker.land.uuid,
        host_layer: layers.names[host].clone(),
        junction_mm: anchor,
        junction_layer: layers.names[layer].clone(),
        retry_argv,
        native_evidence_limit: None,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn propose(
    source: &Expr,
    placed: &Expr,
    path: &Path,
    native: &Value,
    target: &str,
    witness: &Witness<'_>,
    deadline: Instant,
    report: &mut AutomaticSearch,
    work: &mut GeneratedBranchWork,
) -> Result<Option<Relocation>, String> {
    let layers = LayerTable::from_pcb(source)?;
    let codes = net_codes(source);
    let Some(blocker) = blocker(source, placed, &layers, witness, target, &codes) else {
        return Ok(None);
    };
    let Some((anchor, layer, width)) = junction(source, &blocker, &layers, &codes) else {
        return Ok(None);
    };
    let mut stripped = source.clone();
    let Expr::List(items) = &mut stripped else {
        unreachable!()
    };
    items.remove(blocker.slot);
    let rules = pcb_kicad_route::project_rules::resolve_project_rules(
        &path.with_extension("kicad_pro"),
        path,
    )?;
    let (rules, mut board) = lower_for_local(&stripped, rules)?;
    let Some(net) = board
        .nets
        .iter()
        .position(|net| net.name == blocker.via.connection)
    else {
        return Ok(None);
    };
    let limit = Aabb {
        minimum: blocker.land.center,
        maximum: blocker.land.center,
    }
    .inflated(5.0);
    board
        .obstacles
        .retain(|obstacle| obstacle.shape.aabb().intersects(limit));
    let (board, class_rules) = match rules.net_rules.as_deref() {
        Some(rules) => {
            let (board, classes) = core::rules::ClassRules::split(&board, rules);
            (board, Some(classes))
        }
        None => (board, None),
    };
    let mut routes = vec![NetRoute::default(); board.nets.len()];
    let from = blocker.via.at;
    for to in sites(&blocker) {
        if Instant::now() >= deadline
            || report.variants_searched >= MAX_MODEL_SEARCHES
            || work.exhausted()
        {
            break;
        }
        routes[net] = NetRoute {
            segments: vec![Segment {
                start: anchor,
                end: to,
                width,
                layer,
            }],
            vias: vec![Via {
                at: to,
                diameter: blocker.via.size,
                drill: blocker.via.drill,
                span: None,
            }],
        };
        if !core::verify::verify_with_rules(&board, &routes, class_rules.as_ref()).is_empty() {
            continue;
        }
        let mut trial = source.clone();
        let Expr::List(items) = &mut trial else {
            unreachable!()
        };
        let Expr::List(fields) = &mut items[blocker.slot] else {
            unreachable!()
        };
        let at = fields
            .iter_mut()
            .find(|field| field.head() == Some("at"))
            .expect("generated via has coordinates");
        *at = parse(&format!("(at {} {})", to[0], to[1]))?;
        let bridge = supplemental_segment_expr(&SupplementalSegment {
            connection: blocker.via.connection.clone(),
            start: anchor,
            end: to,
            width,
            layer: layers.names[layer].clone(),
        });
        let bridge = original_net(&bridge, blocker.net, &codes);
        let bridge_uuid = uuid(&bridge)?;
        if source
            .children()
            .iter()
            .any(|item| uuid(item).is_ok_and(|id| id == bridge_uuid))
        {
            continue;
        }
        items.push(bridge);
        report.variants_searched += 1;
        let Some(branch) = propose_generated_branch(
            &trial,
            path,
            native,
            target,
            blocker.net,
            witness.ids,
            deadline,
            false,
            work,
        )?
        else {
            continue;
        };
        let Expr::List(items) = &mut trial else {
            unreachable!()
        };
        items.extend(branch.copper.clone());
        index(&trial)?;
        report.filled_pad_via = Some(PadViaMove {
            uuid: blocker.uuid.clone(),
            pad_uuid: blocker.land.uuid,
            net: blocker.net.into(),
            from_mm: from,
            to_mm: to,
            junction_mm: anchor,
            junction_layer: layers.names[layer].clone(),
        });
        return Ok(Some(Relocation {
            pcb: trial,
            changed: vec![blocker.uuid.clone()],
            target_additions: branch.copper,
            other_additions: BTreeMap::from([(bridge_uuid, blocker.net.into())]),
            moved_filled_pad_via: Some(blocker.uuid),
        }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proven_pad_via_must_be_generated_filled_capped_and_absent_from_placed_copper() {
        let pcb = parse(r#"(kicad_pcb (footprint X (pad "1" smd circle (net "COL"))))"#).unwrap();
        let names = note_net_names(&pcb);
        let codes = net_codes(&pcb);
        let via = pad_via_expr(&SupplementalVia {
            connection: "COL".into(),
            at: [1.0, 2.0],
            size: 0.4,
            drill: 0.3,
            layers: ["F.Cu".into(), "B.Cu".into()],
            kind: None,
        });
        let known = uuid(&via).unwrap();
        assert!(generated_pad_via(&via, &BTreeSet::new(), &codes, &names).is_some());
        assert!(generated_pad_via(&via, &BTreeSet::from([known]), &codes, &names).is_none());
        let changed = parse(&encode(&via).replace("(filling yes)", "(filling no)")).unwrap();
        assert!(generated_pad_via(&changed, &BTreeSet::new(), &codes, &names).is_none());
        let moved = parse(&encode(&via).replace("(at 1 2)", "(at 1 2.5)")).unwrap();
        assert!(generated_pad_via(&moved, &BTreeSet::new(), &codes, &names).is_none());
    }
}
