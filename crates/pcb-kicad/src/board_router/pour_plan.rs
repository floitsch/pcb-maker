// Copyright (C) 2026 Toit contributors.

//! Pours a routing request adds: zones planned from the request and
//! written to the board.

use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadPourRequest {
    /// The net, as in the board (`GND` or `/GND`).
    pub net: String,
    /// Copper layers, such as `B.Cu`.
    pub layers: Vec<String>,
    /// Clearance between the pour and other nets (default 0.3 mm).
    #[serde(default)]
    pub clearance_mm: Option<f64>,
    /// A plane (default false): no track of any net on these layers, only
    /// vias through them, so the pour stays whole on every rung of the
    /// ladder (exclusive_planes holds only on some). A rule area keeping
    /// tracks off each layer (`plane <net> <layer>`) is written beside the
    /// zone, also where the board has the zone already, so KiCad checks
    /// it too.
    #[serde(default)]
    pub plane: bool,
    /// Regions where the pour stays whole on these layers (default none):
    /// other nets' tracks on the layer and other nets' vias keep out, the
    /// pour's own net may go there. Each is a box or the boxes of named
    /// parts; a rule area named `keep whole <net> <layer>` marks each
    /// (everything allowed for KiCad, which has no rule by net; the router
    /// lowers it by its name).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keep_whole: Vec<KiCadKeepWhole>,
}

/// A region where a pour stays whole (`KiCadPourRequest::keep_whole`):
/// the boxes of the parts `parts` names (references or globs, each part's
/// courtyard, else its pads), or the box `x`, `y`, each grown by
/// `margin_mm`. The parts are where the board has them when the pours are
/// added (a layout: after placement).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadKeepWhole {
    /// Parts by reference, or globs (`U*`); one region per part.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<String>,
    /// A box, from-to in mm, from the outline's top-left corner unless
    /// `origin` is `absolute` (as constraints.json's keepouts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<[f64; 2]>,
    /// `board` (default: the outline's top-left corner) or `absolute`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// How far each region reaches beyond its box (default 0.5 mm for
    /// parts, 0 for a box).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin_mm: Option<f64>,
}

/// The name of the rule areas that mark where a pour stays whole: `keep
/// whole <net> <layer>`.
pub(crate) const KEEP_WHOLE_NAME: &str = "keep whole";

/// The regions of `entries` on `pcb` as boxes [x0, y0, x1, y1].
pub(crate) fn keep_whole_boxes(
    pcb: &Expr,
    net: &str,
    entries: &[KiCadKeepWhole],
) -> Result<Vec<[f64; 4]>, String> {
    let mut boxes = Vec::new();
    for entry in entries {
        if entry.parts.is_empty() == (entry.x.is_none() || entry.y.is_none()) {
            return Err(format!(
                "keep_whole of the {net} pour: give either parts or both x and y"
            ));
        }
        if !entry.parts.is_empty() {
            let margin = entry.margin_mm.unwrap_or(0.5);
            for pattern in &entry.parts {
                let mut found = false;
                for footprint in pcb
                    .children()
                    .iter()
                    .filter(|item| item.head() == Some("footprint"))
                {
                    let reference = footprint_reference(footprint).unwrap_or_default();
                    if !crate::board_placer::glob_matches(pattern, &reference) {
                        continue;
                    }
                    found = true;
                    if let Some(bounds) = crate::placement_constraints::part_box(footprint)? {
                        boxes.push([
                            bounds[0] - margin,
                            bounds[1] - margin,
                            bounds[2] + margin,
                            bounds[3] + margin,
                        ]);
                    }
                }
                if !found {
                    return Err(format!(
                        "keep_whole of the {net} pour: no part matches {pattern:?}"
                    ));
                }
            }
            continue;
        }
        let (x, y) = (entry.x.unwrap_or_default(), entry.y.unwrap_or_default());
        let offset = match entry.origin.as_deref() {
            None | Some("board") => crate::placement_constraints::outline_corner(pcb)?,
            Some("absolute") => [0.0, 0.0],
            Some(other) => {
                return Err(format!(
                    "keep_whole of the {net} pour: unknown origin {other:?} (board or absolute)"
                ));
            }
        };
        let margin = entry.margin_mm.unwrap_or(0.0);
        let bounds = [
            offset[0] + x[0].min(x[1]) - margin,
            offset[1] + y[0].min(y[1]) - margin,
            offset[0] + x[0].max(x[1]) + margin,
            offset[1] + y[0].max(y[1]) + margin,
        ];
        if !(bounds[2] > bounds[0] && bounds[3] > bounds[1]) {
            return Err(format!("keep_whole of the {net} pour: an empty box"));
        }
        boxes.push(bounds);
    }
    Ok(boxes)
}

/// The pours to add to `pcb`: the requested ones, or else the automatic
/// planes of a multilayer board (see `automatic_planes`).
pub(crate) fn planned_pours(
    pcb: &Expr,
    config: &KiCadBoardRouterConfig,
) -> Result<Vec<KiCadPourRequest>, String> {
    if !config.add_pours.is_empty() {
        return Ok(config.add_pours.clone());
    }
    if config.automatic_planes == Some(false) {
        return Ok(Vec::new());
    }
    let layers = LayerTable::from_pcb(pcb)?;
    let inner: Vec<&String> = layers
        .names
        .iter()
        .filter(|name| name.starts_with("In"))
        .collect();
    if inner.len() < 2 || !pours(pcb, &layers)?.is_empty() {
        return Ok(Vec::new());
    }
    let net_names = NetNames::of(pcb);
    let mut pads: BTreeMap<String, usize> = BTreeMap::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        for pad in footprint
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
        {
            if let Some(net) = node_net(pad).filter(|net| net_names.routable(net)) {
                *pads.entry(net.to_string()).or_default() += 1;
            }
        }
    }
    let busiest = |accept: &dyn Fn(&str) -> bool| {
        pads.iter()
            .filter(|(net, count)| accept(net) && **count >= 3)
            .max_by_key(|(_, count)| **count)
            .map(|(net, _)| net.clone())
    };
    let mut requests = Vec::new();
    if let Some(ground) = busiest(&|net| crate::quality::ground(net)) {
        requests.push(KiCadPourRequest {
            net: ground,
            layers: vec![inner[0].clone()],
            clearance_mm: None,
            plane: false,
            keep_whole: Vec::new(),
        });
    }
    if let Some(rail) =
        busiest(&|net| crate::quality::rail_name(net) && !crate::quality::ground(net))
    {
        requests.push(KiCadPourRequest {
            net: rail,
            layers: vec![inner[inner.len() - 1].clone()],
            clearance_mm: None,
            plane: false,
            keep_whole: Vec::new(),
        });
    }
    Ok(requests)
}

/// Adds a zone over the board outline for every request.
pub(crate) fn add_pour_zones(pcb: &mut Expr, requests: &[KiCadPourRequest]) -> Result<(), String> {
    let net_names = NetNames::of(pcb);
    if requests.is_empty() {
        return Ok(());
    }
    let outline = outline::board_loops(pcb)?.outline;
    let layers = LayerTable::from_pcb(pcb)?;
    let mut nets = BTreeSet::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            if let Some(net) = node_net(pad) {
                nets.insert(net.to_string());
            }
        }
    }
    // Zones the board already has, by (net, layer): a requested pour that
    // exists already is not added twice.
    let mut existing = BTreeSet::new();
    for zone in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone"))
    {
        let Some(net) = form_atom(zone, "net_name", 1).or_else(|| node_net(zone)) else {
            continue;
        };
        let names: Vec<String> = zone
            .child("layers")
            .map(|layers| {
                layers
                    .children()
                    .iter()
                    .skip(1)
                    .filter_map(Expr::atom)
                    .map(str::to_owned)
                    .collect()
            })
            .or_else(|| form_atom(zone, "layer", 1).map(|layer| vec![layer.to_string()]))
            .unwrap_or_default();
        for layer in names {
            existing.insert((net_names.normalize(net).to_string(), layer));
        }
    }
    let mut zones = Vec::new();
    let names: BTreeSet<String> = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone"))
        .filter_map(|zone| form_atom(zone, "name", 1).map(str::to_string))
        .collect();
    let points: Vec<String> = outline
        .iter()
        .map(|point| format!("(xy {} {})", point[0], point[1]))
        .collect();
    for request in requests {
        let net = [request.net.clone(), format!("/{}", request.net)]
            .into_iter()
            .find(|name| nets.contains(name))
            .ok_or_else(|| format!("pour for {:?}: no pad has that net", request.net))?;
        let clearance = request.clearance_mm.unwrap_or(0.3);
        for layer in &request.layers {
            if layers.index(layer).is_none() {
                return Err(format!(
                    "pour for {:?}: {layer:?} is not a copper layer of the board",
                    request.net
                ));
            }
            // A stable identifier from the request.
            let mut hash = 0xcbf2_9ce4_8422_2325u64;
            for byte in net.bytes().chain(layer.bytes()) {
                hash = (hash ^ byte as u64).wrapping_mul(0x100_0000_01b3);
            }
            let name = format!("plane {} {layer}", net_names.normalize(&net));
            if request.plane && !names.contains(&name) {
                let hash = hash.rotate_left(17) ^ 0x9e37_79b9_7f4a_7c15;
                zones.push(parse(&format!(
                    "(zone (layer {layer:?}) (uuid \"{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}\") (name {name:?}) \
                     (hatch edge 0.5) (connect_pads (clearance 0)) (min_thickness 0.25) \
                     (keepout (tracks not_allowed) (vias allowed) (pads allowed) (copperpour allowed) (footprints allowed)) \
                     (fill (thermal_gap 0.5) (thermal_bridge_width 0.5)) \
                     (polygon (pts {})))",
                    hash >> 32,
                    (hash >> 16) & 0xffff,
                    hash & 0xfff,
                    (hash >> 12) & 0xfff,
                    hash & 0xffff_ffff_ffff,
                    points.join(" ")
                ))?);
            }
            if !request.keep_whole.is_empty() {
                let name = format!("{KEEP_WHOLE_NAME} {} {layer}", net_names.normalize(&net));
                for (index, [x0, y0, x1, y1]) in
                    keep_whole_boxes(pcb, &request.net, &request.keep_whole)?
                        .into_iter()
                        .enumerate()
                {
                    let mut hash = hash.rotate_left(29) ^ 0x2545_f491_4f6c_dd1d;
                    for byte in format!("{index} {x0} {y0} {x1} {y1}").bytes() {
                        hash = (hash ^ byte as u64).wrapping_mul(0x100_0000_01b3);
                    }
                    zones.push(parse(&format!(
                        "(zone (layer {layer:?}) (uuid \"{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}\") (name {name:?}) \
                         (hatch edge 0.5) (connect_pads (clearance 0)) (min_thickness 0.25) \
                         (keepout (tracks allowed) (vias allowed) (pads allowed) (copperpour allowed) (footprints allowed)) \
                         (fill (thermal_gap 0.5) (thermal_bridge_width 0.5)) \
                         (polygon (pts (xy {x0} {y0}) (xy {x1} {y0}) (xy {x1} {y1}) (xy {x0} {y1}))))",
                        hash >> 32,
                        (hash >> 16) & 0xffff,
                        hash & 0xfff,
                        (hash >> 12) & 0xfff,
                        hash & 0xffff_ffff_ffff,
                    ))?);
                }
            }
            if existing.contains(&(net_names.normalize(&net).to_string(), layer.clone())) {
                eprintln!(
                    "pour for {} on {layer}: the board has one already",
                    request.net
                );
                continue;
            }
            zones.push(parse(&format!(
                "(zone (net {net:?}) (layer {layer:?}) (uuid \"{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}\") (hatch edge 0.5) \
                 (connect_pads (clearance {clearance})) (min_thickness 0.25) \
                 (fill yes (thermal_gap {clearance}) (thermal_bridge_width 0.3) (island_removal_mode 0)) \
                 (polygon (pts {})))",
                hash >> 32,
                (hash >> 16) & 0xffff,
                hash & 0xfff,
                (hash >> 12) & 0xfff,
                hash & 0xffff_ffff_ffff,
                points.join(" ")
            ))?);
        }
    }
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    // Regions of an earlier run (a routed board routed again) give way to
    // this request's.
    if requests
        .iter()
        .any(|request| !request.keep_whole.is_empty())
    {
        items.retain(|item| {
            !(item.head() == Some("zone")
                && form_atom(item, "name", 1)
                    .is_some_and(|name| name.starts_with(&format!("{KEEP_WHOLE_NAME} "))))
        });
    }
    items.extend(zones);
    Ok(())
}
