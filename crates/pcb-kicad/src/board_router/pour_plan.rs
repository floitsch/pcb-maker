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
    items.extend(zones);
    Ok(())
}
