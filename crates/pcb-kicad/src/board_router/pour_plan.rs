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
    // exists already is not added twice. A zone on a layer set (`F&B.Cu`,
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

/// A pad of a board for the keep_whole checks: `REF:NUMBER`, its part, its
/// net (normalized; none for a pad without one), its centre and its copper
/// layers.
pub(crate) struct RegionPad {
    pub label: String,
    pub reference: String,
    pub net: Option<String>,
    pub center: [f64; 2],
    pub mask: pcb_router::LayerMask,
}

/// Every pad of `pcb` with a copper layer (`RegionPad`).
pub(crate) fn region_pads(pcb: &Expr, layers: &LayerTable) -> Result<Vec<RegionPad>, String> {
    let net_names = NetNames::of(pcb);
    let mut pads = Vec::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
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
            let mask = layers.mask_of_item(pad)?;
            if mask == 0 {
                continue;
            }
            let offset = crate::geometry::rotate_vector(position, -at[2]);
            let number = pad.children().get(1).and_then(Expr::atom).unwrap_or("");
            pads.push(RegionPad {
                label: format!("{reference}:{number}"),
                reference: reference.clone(),
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
                            crate::board_placer::glob_matches(pattern, &pad.reference)
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
        let count = pads.iter().filter(|pad| pad.reference == reference).count();
        if count < KEEP_WHOLE_PADS {
            continue;
        }
        if let Some(bounds) = crate::placement_constraints::part_box(footprint)? {
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
) -> Vec<String> {
    let mut candidates: Vec<&(String, usize, [f64; 4])> = parts
        .iter()
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
