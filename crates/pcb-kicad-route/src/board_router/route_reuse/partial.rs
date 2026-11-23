// Copyright (C) 2026 Florian Loitsch.

//! Locally update one terminal of a proven tree without rerouting its trunk.

use super::*;
use pcb_kicad_file::copper_geometry::lower_pad;
use pcb_router::{Shape, geometry::segment_segment_distance, verify::shape_distance};

mod tees;

const MAX_SEGMENTS: usize = 256;
pub(super) const MAX_CANDIDATES: usize = 32;
const MM_PER_NM: f64 = 0.000001;

pub(super) fn net_zone_layers<'a>(
    pcb: &'a Expr,
    codes: &'a BTreeMap<String, String>,
    layers: &LayerTable,
    names: &NetNames,
) -> BTreeMap<&'a str, u32> {
    let mut masks = BTreeMap::new();
    let mut accumulate = |zone: &'a Expr, on_footprint: bool| {
        if zone.head() != Some("zone") {
            return;
        }
        let mask = if on_footprint {
            // A footprint may give the zone local layer semantics.
            layers.all()
        } else {
            explicit_zone_mask(zone, layers).unwrap_or_else(|| layers.all())
        };
        for net in [item_net(zone, codes), form_atom(zone, "net_name", 1)]
            .into_iter()
            .flatten()
            .filter(|net| !net.is_empty())
        {
            *masks.entry(names.normalize(net)).or_insert(0) |= mask;
        }
    };
    for item in pcb.children() {
        accumulate(item, false);
        if matches!(item.head(), Some("footprint" | "module")) {
            for child in item.children() {
                accumulate(child, true);
            }
        }
    }
    masks
}

fn explicit_zone_mask(zone: &Expr, layers: &LayerTable) -> Option<u32> {
    let ((Some(layer), None) | (None, Some(layer))) = (zone.child("layer"), zone.child("layers"))
    else {
        return None;
    };
    let mut mask = 0;
    for name in &layer.children()[1..] {
        mask |= match name.atom()? {
            "F&B.Cu" => 1 | 1 << (layers.len() - 1),
            "*.Cu" => layers.all(),
            name => 1 << layers.index(name)?,
        };
    }
    (mask != 0).then_some(mask)
}

/// Require all but exactly one unambiguous contact on the same current net.
/// A contact can move on that net or leave its membership altogether.
pub(super) fn changed_leaf<'a>(
    prior: &'a PadGroups,
    current: &'a PadGroups,
    old_name: &str,
) -> Option<(&'a str, &'a PadNet, &'a PadId)> {
    let source = prior.nets.get(old_name)?;
    if source.ambiguous
        || source.terminals.len() < 3
        || source
            .terminals
            .values()
            .any(|contacts| contacts.len() != 1)
    {
        return None;
    }
    let same_contact = |pad: &PadId, contacts: &[PadContact]| {
        current.owners.get(pad).filter(|name| {
            current.nets[*name].terminals.get(pad).map(Vec::as_slice) == Some(contacts)
        })
    };
    let mut survivors = BTreeMap::new();
    for name in source
        .terminals
        .iter()
        .filter_map(|(pad, contacts)| same_contact(pad, contacts))
    {
        *survivors.entry(name).or_insert(0) += 1;
    }
    let (name, _) = survivors
        .into_iter()
        .find(|(_, count)| *count == source.terminals.len() - 1)?;
    let (leaf, _) = source
        .terminals
        .iter()
        .find(|(pad, contacts)| same_contact(pad, contacts) != Some(name))?;
    let net = &current.nets[name];
    let expected = source.terminals.len() - usize::from(!net.terminals.contains_key(leaf));
    if net.ambiguous || net.terminals.len() != expected || merges_prior_nets(prior, old_name, net) {
        return None;
    }
    Some((name, net, leaf))
}

type Point = [i32; 2];

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Node {
    Pad(usize),
    End(usize, Point),
}

fn point(coordinates: [f64; 2]) -> Option<Point> {
    if !coordinates.iter().all(|value| value.is_finite()) {
        return None;
    }
    let [x, y] = coordinates.map(|value| i32::try_from((value / MM_PER_NM).round() as i64).ok());
    Some([x?, y?])
}

struct Contact {
    at: Point,
    layers: u32,
    copper: Shape,
}

fn contacts(board: &LeafBoard<'_>, layers: &LayerTable) -> Option<BTreeMap<PadId, Contact>> {
    let LeafBoard {
        pcb,
        identities,
        net,
        ..
    } = board;
    let mut contacts = BTreeMap::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|form| matches!(form.head(), Some("footprint" | "module")))
    {
        let reference = footprint_reference(footprint)?;
        let reference = if let Some(identities) = identities {
            let Some(reference) = identities.prior_contact_reference(&reference) else {
                continue;
            };
            reference.to_string()
        } else {
            reference
        };
        for pad in footprint
            .children()
            .iter()
            .filter(|form| form.head() == Some("pad"))
        {
            let number = pad.children().get(1).and_then(Expr::atom)?;
            let id = (reference.clone(), number.to_string());
            if !net.terminals.contains_key(&id) {
                continue;
            }
            let mask = layers.pad_mask(pad).ok()?;
            if !matches!(
                pad.children().get(2).and_then(Expr::atom),
                Some("smd" | "thru_hole")
            ) || (pad.children()[2].atom() == Some("smd") && mask.count_ones() != 1)
                || pad.child("padstack").is_some()
                || form_atom(pad, "remove_unused_layers", 1) == Some("yes")
                || pad
                    .child("drill")
                    .is_some_and(|drill| drill.child("offset").is_some())
                || mask == 0
            {
                return None;
            }
            let lowered = lower_pad(pad, form_at(footprint).ok()?).ok()?;
            let copper = super::super::lower::shape(&lowered.geometry);
            if !copper.contains(lowered.center) {
                return None;
            }
            contacts.insert(
                id,
                Contact {
                    at: point(lowered.center)?,
                    layers: mask,
                    copper,
                },
            );
        }
    }
    (contacts.len() == net.terminals.len()).then_some(contacts)
}

#[derive(Clone)]
struct Segment {
    // A virtual edge of an original trunk cannot be pruned separately.
    source: Option<usize>,
    layer: usize,
    at: [[f64; 2]; 2],
    points: [Point; 2],
    nodes: [Node; 2],
    radius: f64,
}

impl Segment {
    fn read(
        item: &Expr,
        source: usize,
        layers: &LayerTable,
        pads: &BTreeMap<(usize, Point), usize>,
    ) -> Option<Self> {
        if item.head() != Some("segment")
            || item.child("locked").is_some()
            || item
                .children()
                .iter()
                .any(|field| field.atom() == Some("locked"))
        {
            return None;
        }
        let layer = layers.index(form_atom(item, "layer", 1)?)?;
        let at = [form_xy(item, "start").ok()?, form_xy(item, "end").ok()?];
        let points = [point(at[0])?, point(at[1])?];
        let nodes = points.map(|xy| {
            let key = (layer, xy);
            pads.get(&key)
                .map_or(Node::End(layer, key.1), |pad| Node::Pad(*pad))
        });
        let radius = form_f64(item, "width", 1).ok()? / 2.0;
        (nodes[0] != nodes[1] && radius > 0.0).then_some(Self {
            source: Some(source),
            layer,
            at,
            points,
            nodes,
            radius,
        })
    }

    fn intersects_without_join(&self, other: &Self) -> bool {
        if self.layer != other.layer {
            return false;
        }
        if let Some(endpoint) = self
            .nodes
            .iter()
            .position(|node| other.nodes.contains(node))
        {
            let join = self.nodes[endpoint];
            let other_endpoint = usize::from(other.nodes[0] != join);
            let a = self.points[endpoint];
            let [x, y] = [0, 1]
                .map(|axis| i128::from(self.points[1 - endpoint][axis]) - i128::from(a[axis]));
            let [u, v] = [0, 1].map(|axis| {
                i128::from(other.points[1 - other_endpoint][axis]) - i128::from(a[axis])
            });
            // Collinear segments must meet at the joint and run apart.
            return x * v == y * u && x * u + y * v > 0;
        }
        segment_segment_distance(self.at[0], self.at[1], other.at[0], other.at[1]) <= MM_PER_NM
    }

    fn copper(&self) -> Shape {
        Shape::Capsule {
            start: self.at[0],
            end: self.at[1],
            radius: self.radius,
        }
    }
}

pub(super) struct LeafBoard<'a> {
    pub pcb: &'a Expr,
    pub identities: Option<&'a RouteReuseIdentities>,
    pub net: &'a PadNet,
    pub zone_layers: u32,
}

pub(super) struct LeafEdit<'a> {
    pub retained: Vec<&'a Expr>,
    pub splice: Option<Expr>,
    pub evidence: KiCadRouteReusePartialNet,
}

pub(super) fn reuse_leaf<'a>(
    previous: LeafBoard<'_>,
    current: LeafBoard<'_>,
    layers: &LayerTable,
    changed: &PadId,
    items: &[&'a Expr],
) -> Option<LeafEdit<'a>> {
    if items.len() > MAX_SEGMENTS {
        return None;
    }
    let previous_contacts = contacts(&previous, layers)?;
    let leaf = previous_contacts.keys().position(|id| id == changed)?;
    let mut pads = BTreeMap::new();
    for (index, contact) in previous_contacts.values().enumerate() {
        for layer in (0..layers.len()).filter(|layer| contact.layers & (1 << layer) != 0) {
            if pads.insert((layer, contact.at), index).is_some() {
                return None;
            }
        }
    }
    let segments: Vec<_> = items
        .iter()
        .enumerate()
        .map(|(source, item)| Segment::read(item, source, layers, &pads))
        .collect::<Option<_>>()?;
    let zone_layers = previous.zone_layers | current.zone_layers;
    if segments
        .iter()
        .any(|segment| zone_layers & (1 << segment.layer) != 0)
    {
        return None;
    }
    let segments = tees::subdivide(segments)?;
    let graph = tree(&segments, previous_contacts.len())?;
    let pad = Node::Pad(leaf);
    let old = previous_contacts.get(changed)?;
    let (pruned, splice, join) = if current.net.terminals.contains_key(changed) {
        let tip = graph.get(&pad)?;
        if tip.len() != 1 {
            return None;
        }
        let edge = *tip.first()?;
        let join = segments[edge].nodes.into_iter().find(|node| *node != pad)?;
        let current_contacts = contacts(&current, layers)?;
        let splice = splice_tip(
            &segments,
            edge,
            leaf,
            &current_contacts,
            items[segments[edge].source?],
        )?;
        (BTreeSet::from([edge]), Some(splice), Some(join))
    } else {
        (branch(graph, pad, &segments)?, None, None)
    };
    let pruned_items: BTreeSet<_> = pruned
        .iter()
        .map(|&edge| segments[edge].source)
        .collect::<Option<_>>()?;
    for (_, segment) in segments
        .iter()
        .enumerate()
        .filter(|(index, _)| !pruned.contains(index))
    {
        if old.layers & (1 << segment.layer) != 0
            && shape_distance(&old.copper, &segment.copper()) <= MM_PER_NM
            // The immediate continuation can start inside the previous
            // pad. A splice keeps that joint and checks the moved pad
            // against every other track at its actual new position.
            && !join.is_some_and(|join| segment.nodes.contains(&join))
        {
            return None;
        }
        if pruned.iter().any(|other| {
            let other = &segments[*other];
            segment.layer == other.layer
                && !segment.nodes.iter().any(|node| other.nodes.contains(node))
                && shape_distance(&segment.copper(), &other.copper()) <= MM_PER_NM
        }) {
            return None;
        }
    }
    let retained: Vec<_> = items
        .iter()
        .enumerate()
        .filter(|(index, _)| !pruned_items.contains(index))
        .map(|(_, item)| *item)
        .collect();
    Some(LeafEdit {
        retained,
        evidence: KiCadRouteReusePartialNet {
            pruned_terminal: format!("{}:{}", changed.0, changed.1),
            retained_terminals: previous_contacts.len() - 1,
            pending_terminals: 0,
            retained_segments: items.len() - pruned_items.len(),
            pruned_segments: pruned_items.len(),
            spliced_segments: usize::from(splice.is_some()),
        },
        splice,
    })
}

/// Preserve every edge after the tip. The straight pad escape grows by
/// at most the distance the pad centre moved.
fn splice_tip(
    segments: &[Segment],
    edge: usize,
    leaf: usize,
    contacts: &BTreeMap<PadId, Contact>,
    item: &Expr,
) -> Option<Expr> {
    let new_pad = contacts.values().nth(leaf)?;
    let old = &segments[edge];
    let end = usize::from(old.nodes[0] != Node::Pad(leaf));
    let moved = new_pad
        .at
        .map(|coordinate| f64::from(coordinate) * MM_PER_NM);
    let distance = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
    if new_pad.layers & (1 << old.layer) == 0
        || new_pad.at == old.points[end]
        || new_pad.at == old.points[1 - end]
        || distance(moved, old.at[end]) > 2.0 * distance(old.at[0], old.at[1])
    {
        return None;
    }
    let mut splice = old.clone();
    splice.at[end] = moved;
    splice.points[end] = new_pad.at;
    let copper = splice.copper();
    for (index, segment) in segments.iter().enumerate() {
        if index == edge {
            continue;
        }
        let track = segment.copper();
        if (new_pad.layers & (1 << segment.layer) != 0
            && shape_distance(&track, &new_pad.copper) <= MM_PER_NM)
            || (segment.layer == splice.layer
                && (splice.intersects_without_join(segment)
                    || (!segment.nodes.contains(&splice.nodes[1 - end])
                        && shape_distance(&track, &copper) <= MM_PER_NM)))
        {
            return None;
        }
    }
    // The moved pad must attach to the old tip, and the tip must not
    // touch another same-net pad except at its original other endpoint.
    for (index, contact) in contacts.values().enumerate() {
        if (index != leaf
            && contact.layers & new_pad.layers != 0
            && shape_distance(&contact.copper, &new_pad.copper) <= MM_PER_NM)
            || (contact.layers & (1 << splice.layer) != 0
                && !splice.nodes.contains(&Node::Pad(index))
                && shape_distance(&contact.copper, &copper) <= MM_PER_NM)
        {
            return None;
        }
    }
    let endpoint = if end == 0 { "start" } else { "end" };
    let mut item = item.clone();
    let Expr::List(forms) = &mut item else {
        return None;
    };
    *forms
        .iter_mut()
        .find(|form| form.head() == Some(endpoint))? = Expr::List(vec![
        Expr::Atom(endpoint.to_string()),
        Expr::Atom(format!("{:.6}", moved[0])),
        Expr::Atom(format!("{:.6}", moved[1])),
    ]);
    Some(item)
}

fn tree(segments: &[Segment], pads: usize) -> Option<BTreeMap<Node, BTreeSet<usize>>> {
    let mut graph = BTreeMap::<_, BTreeSet<_>>::new();
    for (index, item) in segments.iter().enumerate() {
        if segments[..index]
            .iter()
            .any(|other| item.intersects_without_join(other))
        {
            return None;
        }
        for node in item.nodes {
            graph.entry(node).or_default().insert(index);
        }
    }
    if graph.len() != segments.len() + 1
        || (0..pads).any(|pad| !graph.contains_key(&Node::Pad(pad)))
        || graph
            .iter()
            .any(|(node, edges)| matches!(node, Node::End(..)) && edges.len() < 2)
    {
        return None;
    }
    let mut seen = BTreeSet::new();
    let mut pending = vec![*graph.first_key_value()?.0];
    while let Some(node) = pending.pop() {
        if seen.insert(node) {
            pending.extend(graph[&node].iter().flat_map(|index| segments[*index].nodes));
        }
    }
    (seen.len() == graph.len()).then_some(graph)
}

fn branch(
    mut graph: BTreeMap<Node, BTreeSet<usize>>,
    mut leaf: Node,
    segments: &[Segment],
) -> Option<BTreeSet<usize>> {
    let mut pruned = BTreeSet::new();
    while graph.get(&leaf)?.len() == 1 {
        let edge = *graph.remove(&leaf)?.first()?;
        pruned.insert(edge);
        leaf = segments[edge]
            .nodes
            .into_iter()
            .find(|node| *node != leaf)?;
        graph.get_mut(&leaf)?.remove(&edge);
        if matches!(leaf, Node::Pad(_)) {
            break;
        }
    }
    (!pruned.is_empty() && pruned.len() < segments.len()).then_some(pruned)
}
