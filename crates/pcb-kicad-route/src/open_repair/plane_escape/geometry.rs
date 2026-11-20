// Copyright (C) 2026 Florian Loitsch.

//! Find two compatible pad-to-plane access paths on the fixed placement.

use super::*;
use pcb_router as core;
use pcb_router::{ObstacleKind, Shape, Terminal};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

const PITCH_MM: f64 = 0.025;
const SEARCH_RADIUS_MM: f64 = 2.0;
const NEIGHBOR_RADIUS_MM: f64 = 1.2;
const DONOR_RADIUS_MM: f64 = 4.0;
const MAX_OPEN_PADS: usize = 4;
const MAX_NEIGHBORS: usize = 4;
const MAX_SEARCH_STEPS: usize = 20_000;

#[derive(Clone)]
pub(in crate::open_repair) struct Pad {
    pub(in crate::open_repair) id: String,
    pub(in crate::open_repair) reference: String,
    pub(in crate::open_repair) label: String,
    pub(in crate::open_repair) net: String,
}

pub(in crate::open_repair) fn pads(pcb: &Expr) -> Vec<Pad> {
    let codes = net_codes(pcb);
    pcb.children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .filter_map(|item| {
            let reference = item
                .children()
                .iter()
                .find(|property| {
                    property.head() == Some("property")
                        && property.children().get(1).and_then(Expr::atom) == Some("Reference")
                })?
                .children()
                .get(2)?
                .atom()?;
            Some((reference, item))
        })
        .flat_map(|(reference, item)| {
            let codes = &codes;
            item.children().iter().filter_map(move |pad| {
                if pad.head() != Some("pad") || pad.children().get(2)?.atom()? != "smd" {
                    return None;
                }
                Some(Pad {
                    id: item_id(pad)?,
                    reference: reference.to_owned(),
                    label: format!("{reference}.{}", pad.children().get(1)?.atom()?),
                    net: item_net(pad, codes)?.to_owned(),
                })
            })
        })
        .collect()
}

pub(in crate::open_repair) fn contact<'a>(
    board: &'a core::Board,
    pad: &Pad,
) -> Option<(usize, &'a Terminal)> {
    contact_on_net(board, &pad.label, &pad.net)
}

pub(in crate::open_repair) fn contact_on_net<'a>(
    board: &'a core::Board,
    label: &str,
    net_name: &str,
) -> Option<(usize, &'a Terminal)> {
    let mut matches = board
        .nets
        .iter()
        .enumerate()
        .filter(|(_, net)| net.name == net_name)
        .flat_map(|(index, net)| {
            net.terminals
                .iter()
                .filter(|t| t.label == label)
                .map(move |t| (index, t))
        });
    let (net, terminal) = matches.next()?;
    let layers = terminal.layers;
    (matches.next().is_none() && (layers == 1 || layers == (1 << (board.layer_count - 1))))
        .then_some((net, terminal))
}

pub(super) fn open_pad_ids(report: &Value) -> BTreeSet<String> {
    report["unconnected_items"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|row| row["items"].as_array().into_iter().flatten())
        .filter_map(|item| Some(item["uuid"].as_str()?.to_ascii_lowercase()))
        .collect()
}

pub(super) fn plan(
    initial: &Expr,
    original: &Expr,
    report: &Value,
    config: &KiCadBoardRouterConfig,
) -> Result<(Vec<ProtectedEscape>, Vec<Expr>, u64), String> {
    let ids = open_pad_ids(report);
    let pads = pads(initial);
    let layers = board_router::LayerTable::from_pcb(initial)?;
    let mut rules = config.clone();
    rules.bga_fanout = Some(false);
    rules.via_in_pad = Some(KiCadViaInPad::All(false));
    rules.apply_width_floor();
    let board = board_router::lower(initial, &rules, true)?.board;
    let mut probes = 0;
    let codes = net_codes(initial);
    for pad in pads
        .iter()
        .filter(|pad| ids.contains(&pad.id))
        .take(MAX_OPEN_PADS)
    {
        let Some((net, target)) = contact(&board, pad) else {
            continue;
        };
        if !permits_layer(config, pad, target.layers)
            || !plane_at(&board, net, target.layers, target.anchor)
        {
            continue;
        }
        let mut neighbors: Vec<_> = pads
            .iter()
            .filter(|other| other.reference == pad.reference && other.net != pad.net)
            .filter_map(|other| {
                let (other_net, terminal) = contact(&board, other)?;
                let distance = core::geometry::distance(target.anchor, terminal.anchor);
                (distance <= NEIGHBOR_RADIUS_MM
                    && permits_layer(config, other, terminal.layers)
                    && terminal.layers == target.layers
                    && plane_at(&board, other_net, terminal.layers, terminal.anchor))
                .then_some((distance, other, other_net, terminal))
            })
            .collect();
        neighbors.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.label.cmp(&b.1.label)));
        for (_, other, other_net, terminal) in neighbors.into_iter().take(MAX_NEIGHBORS) {
            let layer = target.layers.trailing_zeros() as usize;
            let Some(donor) = donor_escape(original, other, terminal, &layers.names[layer]) else {
                continue;
            };
            let other_class = board.classes[board.nets[other_net].class];
            if donor.diameter + 1e-9 < other_class.via_diameter
                || donor.drill + 1e-9 < other_class.via_drill
                || donor
                    .items
                    .iter()
                    .filter(|item| item.head() == Some("segment"))
                    .any(|item| {
                        form_f64(item, "width", 1).ok().is_none_or(|width| {
                            width + 1e-9
                                < rules.width_floor().max(
                                    rules
                                        .net_rules
                                        .as_ref()
                                        .and_then(|rules| {
                                            rules.width_floor.get(&other.net).copied()
                                        })
                                        .unwrap_or(0.0),
                                )
                        })
                    })
                || !plane_at(&board, other_net, terminal.layers, donor.via_mm)
            {
                continue;
            }
            let class_width = board.classes[board.nets[net].class].trace_width;
            let width = if rules.route_reuse_clearance_unverified
                || rules.ruled_widths.contains(&pad.net)
            {
                class_width
            } else {
                board.neck_width.min(class_width)
            };
            let width = width.max(rules.width_floor()).max(
                rules
                    .net_rules
                    .as_ref()
                    .and_then(|rules| rules.width_floor.get(&pad.net))
                    .copied()
                    .unwrap_or(0.0),
            );
            let search = Pocket::new(&board, net, target, other_net, &donor, width);
            let Some(path) = search.find(&mut probes) else {
                continue;
            };
            let via = *path.last().unwrap();
            let escape = ProtectedEscape {
                terminal: pad.label.clone(),
                net: pad.net.clone(),
                layer: layers.names[layer].clone(),
                path_mm: compact(&path),
                via_mm: via,
            };
            let mut occupied = item_ids(initial);
            let mut pieces = donor
                .items
                .into_iter()
                .chain(emit(&escape, &search, source_net_field(&codes, &pad.net))?)
                .collect::<Vec<_>>();
            for item in &mut pieces {
                // Donor segment IDs belong to the routed board. They
                // can be kept if unique, as on the original placement.
                if !item_id(item).is_some_and(|id| occupied.insert(id)) {
                    return Err("protected escape copper has an ambiguous item identifier".into());
                }
            }
            let neighbor = ProtectedEscape {
                terminal: other.label.clone(),
                net: other.net.clone(),
                layer: layers.names[layer].clone(),
                path_mm: donor.path,
                via_mm: donor.via_mm,
            };
            return Ok((vec![neighbor, escape], pieces, probes));
        }
    }
    Ok((Vec::new(), Vec::new(), probes))
}

fn permits_layer(config: &KiCadBoardRouterConfig, pad: &Pad, layer: u32) -> bool {
    // Whole-board routing still enforces authored pairs and lengths. A
    // pad-to-plane seed must not predetermine one of their connections.
    !config
        .track_disallowed
        .get(&pad.net)
        .is_some_and(|layers| *layers & layer != 0)
        && !config.diff_pair_rules.contains_key(&pad.net)
        && !(config.diff_pairs_on() && project_rules::diff_pair_of(&pad.net).is_some())
        && !config.length_rules.iter().any(|rule| {
            rule.nets
                .iter()
                .any(|net| project_rules::wildcard_match(&net.net, &pad.net))
        })
}

fn plane_at(board: &core::Board, net: usize, surface: u32, at: Point) -> bool {
    board.planes.iter().any(|plane| {
        plane.net as usize == net
            && surface != 1 << plane.layer
            && plane.hatch.is_none()
            && core::geometry::point_in_polygon(at, &plane.polygon)
            && !plane
                .excluded
                .iter()
                .any(|cut| core::geometry::point_in_polygon(at, cut))
    })
}

type NativePoint = [i64; 2];

fn nm(point: Point) -> NativePoint {
    point.map(|coordinate| (coordinate * 1e6).round() as i64)
}
fn mm(point: NativePoint) -> Point {
    point.map(|coordinate| coordinate as f64 * 1e-6)
}

struct Donor {
    path: Vec<Point>,
    via_mm: Point,
    items: Vec<Expr>,
    diameter: f64,
    drill: f64,
}

fn donor_escape(pcb: &Expr, pad: &Pad, terminal: &Terminal, layer: &str) -> Option<Donor> {
    let codes = net_codes(pcb);
    let start = nm(terminal.anchor);
    let mut graph = BTreeMap::<_, Vec<_>>::new();
    let mut vias = BTreeMap::new();
    let near = |point| core::geometry::distance(point, terminal.anchor) <= DONOR_RADIUS_MM;
    for item in pcb
        .children()
        .iter()
        .filter(|item| item_net(item, &codes) == Some(&pad.net))
    {
        if item.head() == Some("segment")
            && form_atom(item, "layer", 1) == Some(layer)
            && let (Ok(a), Ok(b)) = (form_xy(item, "start"), form_xy(item, "end"))
            && near(a)
            && near(b)
            && nm(a) != nm(b)
        {
            graph.entry(nm(a)).or_default().push((nm(b), item));
            graph.entry(nm(b)).or_default().push((nm(a), item));
        }
        if item.head() == Some("via")
            && item
                .children()
                .get(1)
                .is_some_and(|form| form.atom().is_none())
            && form_atom(item, "layers", 1) == Some("F.Cu")
            && form_atom(item, "layers", 2) == Some("B.Cu")
            && let Ok(at) = form_xy(item, "at")
            && near(at)
        {
            vias.entry(nm(at))
                .and_modify(|entry| *entry = None)
                .or_insert(Some(item));
        }
    }
    let mut seen = BTreeMap::new();
    let mut queue = VecDeque::from([start]);
    seen.insert(start, None);
    while let Some(point) = queue.pop_front() {
        if point != start
            && let Some(&Some(via)) = vias.get(&point)
        {
            let mut path = vec![mm(point)];
            let mut items = vec![via.clone()];
            let mut at = point;
            while let Some((parent, item)) = seen[&at] {
                path.push(mm(parent));
                items.push(Expr::clone(item));
                at = parent;
            }
            path.reverse();
            items.reverse();
            let diameter = form_f64(via, "size", 1).ok()?;
            let drill = form_f64(via, "drill", 1).ok()?;
            return (diameter > drill && drill > 0.0).then_some(Donor {
                path,
                via_mm: mm(point),
                items,
                diameter,
                drill,
            });
        }
        for &(neighbor, track) in graph.get(&point).into_iter().flatten() {
            if let std::collections::btree_map::Entry::Vacant(entry) = seen.entry(neighbor) {
                entry.insert(Some((point, track)));
                queue.push_back(neighbor);
            }
        }
    }
    None
}

struct Pocket<'a> {
    board: &'a core::Board,
    origin: Point,
    layer: u32,
    net: usize,
    pad: usize,
    width: f64,
    diameter: f64,
    drill: f64,
    obstacles: Vec<(&'a core::Obstacle, f64)>,
    donor_tracks: Vec<Shape>,
    donor_via: Shape,
    donor_hole: Shape,
    donor_clearance: f64,
}

impl<'a> Pocket<'a> {
    fn new(
        board: &'a core::Board,
        net: usize,
        pad: &Terminal,
        other_net: usize,
        donor: &Donor,
        width: f64,
    ) -> Self {
        let class = board.classes[board.nets[net].class];
        let donor_class = board.classes[board.nets[other_net].class];
        let extent = SEARCH_RADIUS_MM + class.via_diameter + board.hole_clearance + 1.0;
        let obstacles = board
            .obstacles
            .iter()
            .filter(|obstacle| {
                obstacle.shape.distance_to_point(pad.anchor) <= extent || obstacle.clearance > 1.0
            })
            .map(|obstacle| {
                let other = obstacle.net.map_or(class, |owner| {
                    board.classes[board.nets[owner as usize].class]
                });
                let clearance = obstacle
                    .clearance_override
                    .unwrap_or(other.clearance)
                    .max(obstacle.clearance)
                    .max(class.clearance);
                (obstacle, clearance)
            })
            .collect();
        let donor_tracks = donor
            .items
            .iter()
            .filter_map(|item| {
                let start = form_xy(item, "start").ok()?;
                let end = form_xy(item, "end").ok()?;
                let radius = form_f64(item, "width", 1).ok()? / 2.0;
                Some(Shape::Capsule { start, end, radius })
            })
            .collect();
        Self {
            board,
            origin: pad.anchor,
            layer: pad.layers,
            net,
            pad: pad.pad,
            width: width.max(0.001),
            diameter: class.via_diameter,
            drill: class.via_drill,
            obstacles,
            donor_tracks,
            donor_via: Shape::Circle {
                center: donor.via_mm,
                radius: donor.diameter / 2.0,
            },
            donor_hole: Shape::Circle {
                center: donor.via_mm,
                radius: donor.drill / 2.0,
            },
            donor_clearance: class.clearance.max(donor_class.clearance),
        }
    }

    fn wire(&self, a: Point, b: Point) -> bool {
        let half = self.width / 2.0;
        core::geometry::point_in_polygon(b, &self.board.outline)
            && self.obstacles.iter().all(|&(obstacle, gap)| {
                if obstacle.layers & self.layer == 0 || !obstacle.blocks_tracks {
                    return true;
                }
                let gap = match obstacle.kind {
                    ObstacleKind::Copper if obstacle.net == Some(self.net as core::NetId) => {
                        return true;
                    }
                    ObstacleKind::Copper => gap,
                    ObstacleKind::Hole => self.board.hole_clearance,
                    ObstacleKind::Keepout => obstacle.clearance,
                };
                obstacle.shape.distance_to_segment(a, b) + 1e-9 >= half + gap
            })
            && self
                .donor_tracks
                .iter()
                .all(|track| track.distance_to_segment(a, b) + 1e-9 >= half + self.donor_clearance)
            && self.donor_via.distance_to_segment(a, b) + 1e-9 >= half + self.donor_clearance
            && self.donor_hole.distance_to_segment(a, b) + 1e-9 >= half + self.board.hole_clearance
    }

    fn via(&self, at: Point) -> bool {
        let ring = self.diameter / 2.0;
        let hole = self.drill / 2.0;
        let hole_clearance = self.board.hole_clearance;
        plane_at(self.board, self.net, self.layer, at)
            && self.board.obstacles[self.pad].shape.distance_to_point(at) + 1e-9 >= ring
            && self.obstacles.iter().all(|&(obstacle, gap)| {
                if !obstacle.blocks_vias {
                    return true;
                }
                let needed = match obstacle.kind {
                    ObstacleKind::Copper if obstacle.net == Some(self.net as core::NetId) => {
                        return true;
                    }
                    ObstacleKind::Copper => (ring + gap).max(hole + hole_clearance),
                    ObstacleKind::Hole => {
                        (hole + self.board.hole_to_hole).max(ring + hole_clearance)
                    }
                    ObstacleKind::Keepout => ring + obstacle.clearance,
                };
                obstacle.shape.distance_to_point(at) + 1e-9 >= needed
            })
            && self.donor_tracks.iter().all(|track| {
                track.distance_to_point(at) + 1e-9
                    >= (ring + self.donor_clearance).max(hole + hole_clearance)
            })
            && self.donor_via.distance_to_point(at) + 1e-9
                >= (ring + self.donor_clearance).max(hole + hole_clearance)
            && self.donor_hole.distance_to_point(at) + 1e-9
                >= (hole + self.board.hole_to_hole).max(ring + hole_clearance)
    }

    fn find(&self, probes: &mut u64) -> Option<Vec<Point>> {
        if self.drill <= 0.0 || self.diameter <= self.drill {
            return None;
        }
        let limits = (SEARCH_RADIUS_MM / PITCH_MM) as i32;
        let xy = |key: (i32, i32)| {
            mm(nm([
                self.origin[0] + key.0 as f64 * PITCH_MM,
                self.origin[1] + key.1 as f64 * PITCH_MM,
            ]))
        };
        let root = (0, 0);
        let mut queue = BinaryHeap::from([Reverse((0, root))]);
        let mut seen = BTreeMap::from([(root, None)]);
        for _ in 0..MAX_SEARCH_STEPS {
            let Reverse((cost, key)) = queue.pop()?;
            let at = xy(key);
            *probes += 1;
            if self.via(at) {
                let mut path = vec![at];
                let mut current = key;
                while let Some(parent) = seen[&current] {
                    path.push(xy(parent));
                    current = parent;
                }
                path.reverse();
                return Some(path);
            }
            for dx in -1_i32..=1 {
                for dy in -1_i32..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let next = (key.0 + dx, key.1 + dy);
                    if next.0.abs() > limits || next.1.abs() > limits || seen.contains_key(&next) {
                        continue;
                    }
                    *probes += 1;
                    if self.wire(at, xy(next)) {
                        seen.insert(next, Some(key));
                        let step = if dx == 0 || dy == 0 { 1000 } else { 1414 };
                        queue.push(Reverse((cost + step, next)));
                    }
                }
            }
        }
        None
    }
}

fn compact(path: &[Point]) -> Vec<Point> {
    let step = |a: Point, b: Point| {
        let (a, b) = (nm(a), nm(b));
        [b[0] - a[0], b[1] - a[1]]
    };
    let mut points = vec![path[0]];
    for trio in path.windows(3) {
        if step(trio[0], trio[1]) != step(trio[1], trio[2]) {
            points.push(trio[1]);
        }
    }
    points.push(*path.last().unwrap());
    points
}

fn wire_forms(start: Point, end: Point, width: f64, layer: &str, net: &str) -> String {
    let ([ax, ay], [bx, by]) = (start, end);
    format!(
        "(start {ax:.6} {ay:.6}) (end {bx:.6} {by:.6}) (width {width}) (layer {}) {net}",
        quote(layer)
    )
}

fn escape_id(terminal: &str, forms: &str) -> String {
    deterministic_uuid_for(&format!("plane escape {terminal} {forms}"))
}

/// The old writer keyed each protected wire's UUID to its exact segment
/// geometry, native net field, and originating pad. Matching that UUID
/// identifies the writer's original segment, including its legacy width.
pub(crate) fn legacy_escape_segment(item: &Expr, reference: &str, pad: &str) -> bool {
    const FIELDS: [&str; 6] = ["start", "end", "width", "layer", "net", "uuid"];
    if item.head() != Some("segment")
        || item.children().len() != FIELDS.len() + 1
        || FIELDS.iter().any(|name| item.child(name).is_none())
    {
        return false;
    }
    let forms = || {
        let start = form_xy(item, "start").ok()?;
        let end = form_xy(item, "end").ok()?;
        // The old writer emitted endpoints on KiCad's nanometre lattice.
        if start.into_iter().chain(end).any(|value| {
            let nm = value * 1e6;
            (nm - nm.round()).abs() > 1e-6
        }) {
            return None;
        }
        let width = form_f64(item, "width", 1).ok()?;
        let layer = form_atom(item, "layer", 1)?;
        Some(wire_forms(
            start,
            end,
            width,
            layer,
            &encode(item.child("net")?),
        ))
    };
    forms().is_some_and(|forms| {
        item_id(item).is_some_and(|id| id == escape_id(&format!("{reference}.{pad}"), &forms))
    })
}

fn emit(escape: &ProtectedEscape, search: &Pocket<'_>, net: Expr) -> Result<Vec<Expr>, String> {
    let net = encode(&net);
    let mut result = Vec::new();
    for pair in escape.path_mm.windows(2) {
        let forms = wire_forms(pair[0], pair[1], search.width, &escape.layer, &net);
        let uuid = escape_id(&escape.terminal, &forms);
        result.push(parse(&format!("(segment {forms} (uuid \"{uuid}\"))"))?);
    }
    let [x, y] = escape.via_mm;
    let forms = format!(
        "(at {x:.6} {y:.6}) (size {}) (drill {}) (layers \"F.Cu\" \"B.Cu\") {net}",
        search.diameter, search.drill
    );
    let uuid = escape_id(&escape.terminal, &forms);
    result.push(parse(&format!("(via {forms} (uuid \"{uuid}\"))"))?);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn donor_uses_exact_surface_path_to_through_via() {
        let original = parse(
            r#"(kicad_pcb
          (segment (start 2 2) (end 2.2 2.2) (width 0.1) (layer "B.Cu") (net "GND"))
          (segment (start 3.1 2.2) (end 2.2 2.2) (width 0.1) (layer "B.Cu") (net "GND"))
          (via (at 3.1 2.2) (size 0.35) (drill 0.2) (layers "F.Cu" "B.Cu") (net "GND"))
          (segment (start 2 2) (end 2.5 2) (width 0.1) (layer "F.Cu") (net "GND"))
          (via blind (at 2.5 2) (size 0.35) (drill 0.2) (layers "B.Cu" "In1.Cu") (net "GND")))"#,
        )
        .unwrap();
        let pad = Pad {
            id: "pad".into(),
            reference: "J1".into(),
            label: "J1.1".into(),
            net: "GND".into(),
        };
        let mut terminal = Terminal {
            anchor: [2.0, 2.0],
            layers: 8,
            pad: 0,
            contact: None,
            label: pad.label.clone(),
        };
        let donor = donor_escape(&original, &pad, &terminal, "B.Cu").unwrap();
        assert_eq!(donor.items.len(), 3);
        assert_eq!(nm(donor.via_mm), [3_100_000, 2_200_000]);
        assert!(donor_escape(&original, &pad, &terminal, "F.Cu").is_none());
        terminal.anchor[0] += 0.001;
        assert!(donor_escape(&original, &pad, &terminal, "B.Cu").is_none());
    }
}
