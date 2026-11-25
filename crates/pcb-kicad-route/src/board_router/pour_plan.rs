// Copyright (C) 2026 Toit contributors.

//! Pours a routing request adds: zones planned from the request and
//! written to the board.

use super::*;

mod region;
pub use region::{KiCadPourRegion, refresh_pour_regions};
pub(crate) use region::{PAD_REGION_PREFIX, region_name};
use region::{pad_region_polygon, region_uuid};

const MIN_POUR_WIDTH_MM: f64 = 0.25;
const DEFAULT_THERMAL_BRIDGE_WIDTH_MM: f64 = 0.3;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadPourRequest {
    /// The net, as in the board (`GND` or `/GND`).
    pub net: String,
    /// Copper layers, such as `B.Cu`.
    pub layers: Vec<String>,
    /// Limit the zone to a box around the named pads in the current placement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<KiCadPourRegion>,
    /// Clearance between the pour and other nets (default 0.3 mm).
    #[serde(default)]
    pub clearance_mm: Option<f64>,
    /// Thermal relief spoke width (default 0.3 mm, minimum 0.25 mm).
    #[serde(
        default,
        deserialize_with = "read_thermal_bridge_width",
        skip_serializing_if = "Option::is_none"
    )]
    pub thermal_bridge_width_mm: Option<f64>,
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

fn check_thermal_bridge_width(width: f64) -> Result<f64, String> {
    let max_mm = f64::from(i32::MAX) / 1.0e6;
    if !(MIN_POUR_WIDTH_MM..=max_mm).contains(&width) {
        return Err(format!(
            "add_pours thermal_bridge_width_mm must be at least {MIN_POUR_WIDTH_MM} mm and fit KiCad's dimension range"
        ));
    }
    Ok(width)
}

fn read_thermal_bridge_width<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<f64>::deserialize(deserializer)?
        .map(check_thermal_bridge_width)
        .transpose()
        .map_err(serde::de::Error::custom)
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
                    if !crate::kicad_nets::glob_matches(pattern, &reference) {
                        continue;
                    }
                    found = true;
                    if let Some(bounds) = crate::footprint_bodies::part_box(footprint)? {
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
            None | Some("board") => crate::outline::outline_corner(pcb)?,
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
pub fn planned_pours(
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
            region: None,
            clearance_mm: None,
            thermal_bridge_width_mm: None,
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
            region: None,
            clearance_mm: None,
            thermal_bridge_width_mm: None,
            plane: false,
            keep_whole: Vec::new(),
        });
    }
    Ok(requests)
}

/// Adds a zone over the board outline or a requested pad region.
pub fn add_pour_zones(pcb: &mut Expr, requests: &[KiCadPourRequest]) -> Result<(), String> {
    let net_names = NetNames::of(pcb);
    if requests.is_empty() {
        return Ok(());
    }
    if let Some(request) = requests
        .iter()
        .find(|request| request.region.is_some() && request.plane)
    {
        return Err(format!(
            "pour for {:?}: a pad region cannot reserve a whole plane",
            request.net
        ));
    }
    let outline = outline::board_loops(pcb)?.outline;
    let layers = LayerTable::from_pcb(pcb)?;
    let pads = if requests.iter().any(|request| request.region.is_some()) {
        region_pads(pcb, &layers)?
    } else {
        Vec::new()
    };
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
    // Whole-board requests respect existing zones by (net, layer).
    // A zone on a layer set (`F&B.Cu`,
    // `*.Cu`, a list) has each of its layers (the trial's board A wrote a
    // second F.Cu zone over its F&B.Cu one, 2026-10-26).
    let mut existing = BTreeSet::new();
    for zone in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone"))
    {
        let Some(net) = form_atom(zone, "net_name", 1).or_else(|| node_net(zone)) else {
            continue;
        };
        let mask = layers.mask_of_item(zone)?;
        for (index, layer) in layers.names.iter().enumerate() {
            if mask & (1 << index) != 0 {
                existing.insert((net_names.normalize(net).to_string(), layer.clone()));
            }
        }
    }
    let mut zones = Vec::new();
    let mut region_ids = BTreeSet::new();
    let mut requested_modes = BTreeMap::new();
    let names: BTreeSet<String> = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone"))
        .filter_map(|zone| form_atom(zone, "name", 1).map(str::to_string))
        .collect();
    let points = outline
        .iter()
        .map(|point| format!("(xy {} {})", point[0], point[1]))
        .collect::<Vec<_>>()
        .join(" ");
    for request in requests {
        let net = [request.net.clone(), format!("/{}", request.net)]
            .into_iter()
            .find(|name| nets.contains(name))
            .ok_or_else(|| format!("pour for {:?}: no pad has that net", request.net))?;
        let clearance = request.clearance_mm.unwrap_or(0.3);
        let region = request
            .region
            .as_ref()
            .map(|region| pad_region_polygon(region, net_names.normalize(&net), &pads))
            .transpose()?;
        let pour_points = region.as_deref().unwrap_or(&points);
        let thermal_bridge_width = check_thermal_bridge_width(
            request
                .thermal_bridge_width_mm
                .unwrap_or(DEFAULT_THERMAL_BRIDGE_WIDTH_MM),
        )?;
        for layer in &request.layers {
            if layers.index(layer).is_none() {
                return Err(format!(
                    "pour for {:?}: {layer:?} is not a copper layer of the board",
                    request.net
                ));
            }
            let key = (net_names.normalize(&net).to_string(), layer.clone());
            let regional = request.region.is_some();
            if requested_modes
                .insert(key.clone(), regional)
                .is_some_and(|mode| mode != regional)
            {
                return Err(format!(
                    "pour for {net:?} on {layer}: choose either a whole-board zone or pad regions"
                ));
            }
            // A stable identifier from the request.
            let hash = pour_hash(&net, layer);
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
                    points
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
            let identity = if let Some(region) = &request.region {
                let id = region_uuid(&net, layer, region);
                if !region_ids.insert(id.clone()) {
                    return Err(format!(
                        "pour for {net:?} on {layer}: more than one region uses the same anchor pads"
                    ));
                }
                let name = region_name(&key.0, layer);
                let mut matching = pcb
                    .children()
                    .iter()
                    .filter(|item| form_atom(item, "uuid", 1) == Some(id.as_str()));
                if let Some(old) = matching.next() {
                    if matching.next().is_some()
                        || old.head() != Some("zone")
                        || node_net(old) != Some(net.as_str())
                        || form_atom(old, "layer", 1) != Some(layer.as_str())
                        || form_atom(old, "name", 1) != Some(name.as_str())
                    {
                        return Err(format!(
                            "pour for {net:?} on {layer}: region identity already belongs to another PCB item"
                        ));
                    }
                    continue;
                }
                existing.insert(key);
                format!("(uuid {id:?}) (name {name:?})")
            } else {
                if !existing.insert(key) {
                    eprintln!(
                        "pour for {} on {layer}: the board has one already",
                        request.net
                    );
                    continue;
                }
                format!("(uuid {:?})", zone_uuid(hash))
            };
            zones.push(solid_pour(
                &format!("{net:?}"),
                layer,
                &identity,
                clearance,
                thermal_bridge_width,
                pour_points,
            )?);
        }
    }
    refresh_pour_regions(pcb, requests)?;
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

fn solid_pour(
    net_field: &str,
    layer: &str,
    identity: &str,
    clearance: f64,
    thermal_bridge_width: f64,
    points: &str,
) -> Result<Expr, String> {
    parse(&format!(
        "(zone (net {net_field}) (layer {layer:?}) {identity} (hatch edge 0.5) \
         (connect_pads (clearance {clearance})) (min_thickness {MIN_POUR_WIDTH_MM}) \
         (fill yes (thermal_gap {clearance}) (thermal_bridge_width {thermal_bridge_width}) (island_removal_mode 0)) \
         (polygon (pts {points})))",
    ))
}

/// Forms one solid zone using the same fill settings as `add_pours`.
pub fn local_pour_zone(
    net: &str,
    layer: &str,
    endpoints: [&str; 2],
    [x0, y0, x1, y1]: [f64; 4],
) -> Result<(String, Expr), String> {
    let extent = f64::from(i32::MAX) / 1.0e6;
    if !(x0 < x1 && y0 < y1)
        || [x0, x1, y0, y1]
            .into_iter()
            .any(|n| !(-extent..=extent).contains(&n))
    {
        return Err("local pour bounds do not fit KiCad's coordinate range".into());
    }
    let identity = zone_uuid(pour_hash(
        net,
        &format!("{layer} local {} {}", endpoints[0], endpoints[1]),
    ));
    let label = format!("(uuid {identity:?}) (name \"local repair pour\")");
    let points = format!(
        "(xy {x0:.6} {y0:.6}) (xy {x1:.6} {y0:.6}) (xy {x1:.6} {y1:.6}) (xy {x0:.6} {y1:.6})"
    );
    let net = kicad_sexpr::quote(&kicad_sexpr::unescape(net));
    Ok((
        identity,
        solid_pour(
            &net,
            layer,
            &label,
            0.3,
            DEFAULT_THERMAL_BRIDGE_WIDTH_MM,
            &points,
        )?,
    ))
}

fn pour_hash(net: &str, layer: &str) -> u64 {
    net.bytes()
        .chain(layer.bytes())
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
        })
}

fn zone_uuid(hash: u64) -> String {
    format!(
        "{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}",
        hash >> 32,
        (hash >> 16) & 0xffff,
        hash & 0xfff,
        (hash >> 12) & 0xfff,
        hash & 0xffff_ffff_ffff
    )
}

/// A pad of a board for the keep_whole checks: `REF:NUMBER`, its part, its
/// net (normalized; none for a pad without one), its centre and its copper
/// layers.
pub(crate) struct RegionPad {
    pub label: String,
    pub reference: String,
    unique_reference: bool,
    pub net: Option<String>,
    pub center: [f64; 2],
    pub mask: pcb_router::LayerMask,
}

/// Every pad of `pcb` with a copper layer (`RegionPad`).
pub(crate) fn region_pads(pcb: &Expr, layers: &LayerTable) -> Result<Vec<RegionPad>, String> {
    let net_names = NetNames::of(pcb);
    let mut pads = Vec::new();
    let parts = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"));
    let mut references = BTreeMap::<String, usize>::new();
    for name in parts.clone().filter_map(footprint_reference) {
        *references.entry(name).or_default() += 1;
    }
    for footprint in parts {
        let reference = footprint_reference(footprint).unwrap_or_default();
        let at = form_at(footprint)?;
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            let Ok(position) = form_xy(pad, "at") else {
                continue;
            };
            let mask = layers.pad_mask(pad)?;
            if mask == 0 {
                continue;
            }
            let offset = crate::geometry::rotate_vector(position, -at[2]);
            let number = pad.children().get(1).and_then(Expr::atom).unwrap_or("");
            pads.push(RegionPad {
                label: format!("{reference}:{number}"),
                reference: reference.clone(),
                unique_reference: references.get(&reference) == Some(&1),
                net: node_net(pad)
                    .filter(|net| !net.is_empty())
                    .map(|net| net_names.normalize(net).to_string()),
                center: [at[0] + offset[0], at[1] + offset[1]],
                mask,
            });
        }
    }
    Ok(pads)
}

/// The pads of other nets than `net` (normalized, as `RegionPad::net`) on
/// layer `layer` (an index) whose
/// centre lies in `region` [x0, y0, x1, y1]: KiCad's fill leaves them
/// out of the pour, and the region keeps their tracks off the layer and
/// their vias out, so they can only be reached from beyond it.
pub(crate) fn foreign_pads_in<'a>(
    pads: &'a [RegionPad],
    net: &'a str,
    layer: usize,
    region: [f64; 4],
) -> impl Iterator<Item = &'a RegionPad> + 'a {
    pads.iter().filter(move |pad| {
        pad.mask & (1 << layer) != 0
            && pad.net.as_deref().is_some_and(|other| other != net)
            && (region[0]..=region[2]).contains(&pad.center[0])
            && (region[1]..=region[3]).contains(&pad.center[1])
    })
}

/// How many pads a warning names before it says how many more.
const NAMED_PADS: usize = 8;

/// The input check of the `keep_whole` regions of `requests` on `pcb`: one
/// warning per pour and layer whose regions hold pads of other nets on
/// that layer, naming the pads (they would be cut off there: the region
/// keeps their tracks and vias out). With `placed` false (a layout, which
/// places the parts first) only a named part's own pads count, which move
/// with it; a box and other parts' pads are judged where the board has
/// them only when `placed`.
pub fn keep_whole_warnings(
    pcb: &Expr,
    requests: &[KiCadPourRequest],
    placed: bool,
) -> Result<Vec<String>, String> {
    if requests.iter().all(|request| request.keep_whole.is_empty()) {
        return Ok(Vec::new());
    }
    let layers = LayerTable::from_pcb(pcb)?;
    let pads = region_pads(pcb, &layers)?;
    let net_names = NetNames::of(pcb);
    let mut warnings = Vec::new();
    for request in requests
        .iter()
        .filter(|request| !request.keep_whole.is_empty())
    {
        for layer in &request.layers {
            let Some(index) = layers.index(layer) else {
                continue;
            };
            let mut covered: Vec<&str> = Vec::new();
            for entry in &request.keep_whole {
                if !placed && entry.parts.is_empty() {
                    continue;
                }
                for region in keep_whole_boxes(pcb, &request.net, std::slice::from_ref(entry))? {
                    let net = net_names.normalize(&request.net);
                    for pad in foreign_pads_in(&pads, net, index, region) {
                        let own = entry.parts.iter().any(|pattern| {
                            crate::kicad_nets::glob_matches(pattern, &pad.reference)
                        });
                        if (placed || own) && !covered.contains(&pad.label.as_str()) {
                            covered.push(&pad.label);
                        }
                    }
                }
            }
            if covered.is_empty() {
                continue;
            }
            let named: Vec<&str> = covered.iter().take(NAMED_PADS).copied().collect();
            let more = covered.len() - named.len();
            warnings.push(format!(
                "keep_whole of the {} pour on {layer} covers {} pad{} of other nets on that layer ({}{}): the region keeps their tracks off {layer} and their vias out, so they can only be reached from beyond it; keep the regions clear of other nets' pads (name parts on the other side, or a box), or pour another layer",
                request.net,
                covered.len(),
                if covered.len() == 1 { "" } else { "s" },
                named.join(", "),
                if more > 0 { format!(" and {more} more") } else { String::new() },
            ));
        }
    }
    Ok(warnings)
}

/// The parts of `pcb` with at least `KEEP_WHOLE_PADS` pads, each with
/// its pad count and the region a `keep_whole` entry naming it would
/// cover (its box grown by the default 0.5 mm margin).
pub(crate) fn keep_whole_parts(
    pcb: &Expr,
    pads: &[RegionPad],
) -> Result<Vec<(String, usize, [f64; 4])>, String> {
    let mut parts = Vec::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let reference = footprint_reference(footprint).unwrap_or_default();
        // A footprint without a reference cannot be named by a `parts`
        // selector, and its pads would be counted together with every
        // other reference-less footprint's.
        if reference.is_empty() {
            continue;
        }
        let count = pads.iter().filter(|pad| pad.reference == reference).count();
        if count < KEEP_WHOLE_PADS {
            continue;
        }
        if let Some(bounds) = crate::footprint_bodies::part_box(footprint)? {
            let region = [
                bounds[0] - 0.5,
                bounds[1] - 0.5,
                bounds[2] + 0.5,
                bounds[3] + 0.5,
            ];
            parts.push((reference, count, region));
        }
    }
    Ok(parts)
}

/// Of `parts` (`keep_whole_parts`), those under which a `keep_whole`
/// region for `net` on layer `layer` (an index) would cover no other
/// net's pad on that layer, at most `NAMED_PARTS`, the most pads first:
/// the verdict's pour advice names them, and none when every such region
/// would cut pads off.
pub(crate) fn keep_whole_candidates(
    parts: &[(String, usize, [f64; 4])],
    pads: &[RegionPad],
    net: &str,
    layer: usize,
    bounded: Option<&super::pour_quality::KiCadBoundedTrackLength>,
) -> Vec<String> {
    let mut occurrences = BTreeMap::new();
    if bounded.is_some() {
        for (reference, _, _) in parts {
            *occurrences.entry(reference.as_str()).or_insert(0) += 1;
        }
    }
    let mut candidates: Vec<&(String, usize, [f64; 4])> = parts
        .iter()
        .filter(|(reference, _, region)| {
            bounded.is_none_or(|bounded| {
                // A single part selector must identify the proposed region.
                occurrences.get(reference.as_str()) == Some(&1)
                    && !reference.contains(['*', '?', '[', ']'])
                    && bounded.regions.iter().any(|zone| {
                        let [x0, y0, x1, y1] = zone.bounds_mm;
                        x0 <= region[0] && y0 <= region[1] && x1 >= region[2] && y1 >= region[3]
                    })
            })
        })
        .filter(|(_, _, region)| foreign_pads_in(pads, net, layer, *region).next().is_none())
        .collect();
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    candidates
        .into_iter()
        .take(NAMED_PARTS)
        .map(|(reference, _, _)| reference.clone())
        .collect()
}

/// A part with this many pads or more is a `keep_whole` candidate (an
/// IC, a connector; not a capacitor).
const KEEP_WHOLE_PADS: usize = 6;

/// How many candidate parts the pour advice names.
const NAMED_PARTS: usize = 4;

/// `keep_whole_warnings` for the `add_pours` of `config` on the board
/// `source/<board_id>.kicad_pcb`, before the run: route mode passes
/// `placed` (the board's positions are final), a layout does not (only the
/// named parts' own pads, which move with them).
pub fn keep_whole_input_warnings(
    source: &Path,
    board_id: &str,
    config: &KiCadBoardRouterConfig,
    placed: bool,
) -> Result<Vec<String>, String> {
    if config
        .add_pours
        .iter()
        .all(|request| request.keep_whole.is_empty())
    {
        return Ok(Vec::new());
    }
    let path = source.join(format!("{board_id}.kicad_pcb"));
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let pcb = crate::canonical_order::parse_board(&text)?;
    keep_whole_warnings(&pcb, &config.add_pours, placed)
}
