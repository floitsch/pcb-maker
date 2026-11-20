// Copyright (C) 2026 Florian Loitsch.

//! Seed a route with exact prior copper, then admit it using current-rule DRC.

use super::*;
use sha2::{Digest, Sha256};

mod drc;
mod identity;
mod keepouts;
mod partial;
pub use drc::{KiCadRouteReuseFinding, KiCadRouteReuseFindingItem, check_route_reuse};
pub use identity::RouteReuseIdentities;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadRouteReuse {
    pub path: PathBuf,
    pub sha256: String,
    pub status: String,
    pub reason: Option<String>,
    pub matched_nets: Vec<String>,
    pub extended_nets: Vec<String>,
    pub renamed_nets: Vec<String>,
    /// Old reference to current reference, matched by unique schematic path.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub renamed_references: BTreeMap<String, String>,
    /// Prior leaf branches removed from nets offered for partial reuse.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub partial_nets: BTreeMap<String, KiCadRouteReusePartialNet>,
    pub skipped_nets: BTreeMap<String, usize>,
    /// Prior nets withheld because their copper enters a current keepout.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub keepout_skipped_nets: Vec<String>,
    pub segments: usize,
    pub arcs: usize,
    pub vias: usize,
    pub trial_work_seconds: f64,
    pub remaining_budget_seconds: f64,
    pub native_checks: usize,
    pub native_check_wall_seconds: f64,
    /// Findings in the current unseeded schematic parity check.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub baseline_schematic_parity_by_type: BTreeMap<String, usize>,
    /// The selected route's strict DRC also supplied the final comparison.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub shared_final_drc: bool,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub new_findings: BTreeMap<String, usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub new_finding_examples: Vec<KiCadRouteReuseFinding>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadRouteReusePartialNet {
    pub pruned_terminal: String,
    pub retained_terminals: usize,
    pub pending_terminals: usize,
    pub retained_segments: usize,
    pub pruned_segments: usize,
    #[serde(skip_serializing_if = "no_splice")]
    pub spliced_segments: usize,
}

fn no_splice(count: &usize) -> bool {
    *count == 0
}

type PadId = (String, String);

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct PadContact {
    footprint_side: String,
    footprint_pose: [u64; 3],
    pad_pose: [u64; 3],
    pad_forms: Vec<String>,
}

fn pose(item: &Expr) -> Option<[u64; 3]> {
    let mut pose = form_at(item).ok()?;
    if !pose.iter().all(|coordinate| coordinate.is_finite()) {
        return None;
    }
    pose[2] = pose[2].rem_euclid(360.0);
    // Use exact finite values in a sortable key. Both spellings of zero
    // are the same physical coordinate.
    Some(pose.map(|coordinate| {
        if coordinate == 0.0 {
            0
        } else {
            coordinate.to_bits()
        }
    }))
}

fn physical_form(field: &Expr) -> String {
    match field {
        // KiCad saves some atoms (for example pad layer names) with quotes
        // that the originating writer may omit.
        Expr::Atom(_) => quote(field.atom().unwrap_or_default()),
        Expr::List(items) => format!(
            "({})",
            items
                .iter()
                .map(physical_form)
                .collect::<Vec<_>>()
                .join(" ")
        ),
    }
}

fn physical_pad_field(field: &Expr) -> String {
    if field.head() == Some("layers") && field.children().iter().all(|item| item.atom().is_some()) {
        // KiCad versions can serialize the same pad layer set in different
        // order. Other pad fields can contain order-sensitive geometry.
        let mut names: Vec<_> = field.children()[1..].iter().map(physical_form).collect();
        names.sort_unstable();
        names.insert(0, quote("layers"));
        return format!("({})", names.join(" "));
    }
    physical_form(field)
}

fn contact(footprint: &Expr, pad: &Expr) -> Option<PadContact> {
    let mut pad_forms: Vec<String> = pad
        .children()
        .iter()
        .skip(2)
        .filter(|field| {
            !matches!(
                field.head(),
                Some("at" | "net" | "uuid" | "tstamp" | "pinfunction" | "pintype" | "locked")
            ) && field.atom() != Some("locked")
                && !(field.head() == Some("remove_unused_layers")
                    && field.children().get(1).and_then(Expr::atom) == Some("no"))
        })
        .map(physical_pad_field)
        .collect();
    pad_forms.sort_unstable();
    Some(PadContact {
        footprint_side: form_atom(footprint, "layer", 1)?.to_string(),
        footprint_pose: pose(footprint)?,
        pad_pose: pose(pad)?,
        pad_forms,
    })
}

#[derive(Default, PartialEq, Eq)]
struct PadNet {
    terminals: BTreeMap<PadId, Vec<PadContact>>,
    native_net: Option<Expr>,
    ambiguous: bool,
}

impl PadNet {
    fn contact_count(&self) -> usize {
        self.terminals.values().map(Vec::len).sum()
    }
}

#[derive(Default, PartialEq, Eq)]
struct PadGroups {
    nets: BTreeMap<String, PadNet>,
    owners: BTreeMap<PadId, String>,
}

enum ContactSide<'a> {
    Native,
    Previous(&'a RouteReuseIdentities),
    Current(&'a RouteReuseIdentities),
}

fn pad_groups(pcb: &Expr, codes: &BTreeMap<String, String>, side: ContactSide<'_>) -> PadGroups {
    let mut groups = PadGroups::default();
    let footprints = || {
        pcb.children()
            .iter()
            .filter(|item| matches!(item.head(), Some("footprint" | "module")))
    };
    let mut ref_counts = BTreeMap::new();
    let mut pad_counts = BTreeMap::new();
    for footprint in footprints() {
        let reference = footprint_reference(footprint).unwrap_or_default();
        let canonical = match side {
            ContactSide::Previous(identities) => identities.prior_contact_reference(&reference),
            ContactSide::Current(identities) if !identities.current_contact_allowed(&reference) => {
                None
            }
            _ => Some(reference.as_str()),
        };
        if let Some(canonical) = canonical {
            *ref_counts.entry(canonical.to_string()).or_insert(0usize) += 1;
        }
        for pad in footprint
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
        {
            let number = pad
                .children()
                .get(1)
                .and_then(Expr::atom)
                .unwrap_or_default();
            let id = canonical.map(|reference| (reference.to_string(), number.to_string()));
            if let Some(id) = &id {
                *pad_counts.entry(id.clone()).or_insert(0usize) += 1;
            }
            let Some(name) = item_net(pad, codes).filter(|name| !name.trim().is_empty()) else {
                continue;
            };
            let group = groups.nets.entry(name.to_string()).or_default();
            let Some(id) = id else {
                group.ambiguous = true;
                continue;
            };
            let native_net = pad.child("net").and_then(|field| field.children().get(1));
            if let Some(native_net) = native_net {
                if group
                    .native_net
                    .as_ref()
                    .is_some_and(|known| known != native_net)
                {
                    group.ambiguous = true;
                }
                group.native_net.get_or_insert_with(|| native_net.clone());
            } else {
                group.ambiguous = true;
            }
            groups.owners.insert(id.clone(), name.to_string());
            if let Some(contact) = contact(footprint, pad) {
                group.terminals.entry(id).or_default().push(contact);
            } else {
                group.ambiguous = true;
            }
        }
    }
    for group in groups.nets.values_mut() {
        for (id, contacts) in &mut group.terminals {
            // Every copy of a numbered pad must belong to this same
            // native net. The total includes unassigned copies.
            group.ambiguous |= id.0.trim().is_empty()
                || id.1.trim().is_empty()
                || ref_counts[&id.0] != 1
                || pad_counts[id] != contacts.len();
            contacts.sort_unstable();
        }
    }
    // Net-assigned graphics might be electrical endpoints without pad IDs.
    for item in pcb
        .children()
        .iter()
        .chain(footprints().flat_map(Expr::children))
    {
        if item
            .head()
            .is_some_and(|head| head.starts_with("gr_") || head.starts_with("fp_"))
            && let Some(group) = item_net(item, codes).and_then(|name| groups.nets.get_mut(name))
        {
            group.ambiguous = true;
        }
    }
    groups
}

fn compatible_layer_stack(prior: &Expr, current: &Expr) -> Result<bool, String> {
    let stackup = |pcb: &Expr| {
        pcb.child("setup")
            .and_then(|setup| setup.child("stackup"))
            .cloned()
    };
    let thickness = |pcb: &Expr| {
        pcb.child("general")
            .and_then(|general| form_f64(general, "thickness", 1).ok())
    };
    Ok(
        LayerTable::from_pcb(prior)?.names == LayerTable::from_pcb(current)?.names
            && stackup(prior) == stackup(current)
            && thickness(prior) == thickness(current),
    )
}

fn target_group<'a>(
    prior: &PadGroups,
    current: &'a PadGroups,
    old_name: &str,
) -> Result<(&'a str, &'a PadNet), &'static str> {
    let source = prior.nets.get(old_name).ok_or("no_prior_terminals")?;
    if source.ambiguous || source.contact_count() < 2 {
        return Err("ambiguous_or_insufficient_prior_terminals");
    }
    let mut contacts = source.terminals.iter();
    let (pad, _) = contacts.next().expect("at least two old contacts");
    let name = current.owners.get(pad).ok_or("changed_terminals")?;
    let net = &current.nets[name];
    if net.ambiguous {
        return Err("ambiguous_current_terminals");
    }
    if source
        .terminals
        .keys()
        .any(|pad| current.owners.get(pad) != Some(name))
    {
        return Err("changed_terminals");
    }
    if source
        .terminals
        .iter()
        .any(|(pad, contacts)| net.terminals.get(pad) != Some(contacts))
    {
        return Err("changed_pad_contacts");
    }
    if merges_prior_nets(prior, old_name, net) {
        return Err("merged_prior_nets");
    }
    Ok((name, net))
}

// Do not pick one old net out of a merge. New pad identities, or old
// pads without a multi-terminal net, can extend one established net.
fn merges_prior_nets(prior: &PadGroups, old_name: &str, net: &PadNet) -> bool {
    net.terminals.keys().any(|pad| {
        prior.owners.get(pad).is_some_and(|other| {
            other != old_name
                && (prior.nets[other].ambiguous || prior.nets[other].contact_count() >= 2)
        })
    })
}

pub(super) fn track_items(pcb: &Expr) -> impl Iterator<Item = &Expr> {
    pcb.children()
        .iter()
        .filter(|item| matches!(item.head(), Some("segment" | "arc" | "via")))
}

pub(super) fn copper_census(pcb: &Expr) -> BTreeMap<(String, Vec<String>), usize> {
    let codes = net_codes(pcb);
    let mut census = BTreeMap::new();
    for item in track_items(pcb) {
        let name = item_net(item, &codes).unwrap_or_default().to_string();
        let mut forms: Vec<_> = item
            .children()
            .iter()
            .filter(|field| field.head() != Some("net"))
            .map(encode)
            .collect();
        forms.sort_unstable();
        *census.entry((name, forms)).or_default() += 1;
    }
    census
}

/// The routed trial must still contain all current and retained track items.
pub fn check_reused_copper(candidate: &Expr, trial_board: &Path) -> Result<(), String> {
    let source = fs::read_to_string(trial_board).map_err(|error| error.to_string())?;
    let routed = parse(&source)?;
    let actual = copper_census(&routed);
    if copper_census(candidate)
        .iter()
        .any(|(item, count)| actual.get(item).is_none_or(|kept| kept < count))
    {
        return Err("the seeded trial changed current or retained copper".into());
    }
    Ok(())
}

fn copper_item(item: &Expr, net: &Expr, id_head: &str, ids: &mut BTreeSet<String>) -> Expr {
    let mut forms: Vec<_> = item
        .children()
        .iter()
        .filter(|field| {
            !matches!(field.head(), Some("net" | "uuid" | "tstamp" | "locked"))
                && field.atom() != Some("locked")
        })
        .cloned()
        .collect();
    forms.push(Expr::List(vec![Expr::Atom("net".into()), net.clone()]));
    let key = encode(&Expr::List(forms.clone()));
    for salt in 0.. {
        let uuid = deterministic_uuid_for(&format!("route reuse {key} {salt}"));
        if ids.insert(uuid.clone()) {
            forms.push(Expr::List(vec![
                Expr::Atom(id_head.into()),
                Expr::Atom(quote(&uuid)),
            ]));
            break;
        }
    }
    Expr::List(forms)
}

fn stable_copper(
    previous: &Expr,
    current: &Expr,
    current_pads: &PadGroups,
    old_codes: &BTreeMap<String, String>,
    id_head: &str,
) -> Option<Vec<Expr>> {
    let mut ids = BTreeSet::new();
    collect_ids(current, &mut ids);
    track_items(previous)
        .map(|item| {
            let name = item_net(item, old_codes)?;
            let net = current_pads.nets.get(name)?.native_net.as_ref()?;
            let net_form = item.child("net")?.children();
            let id_form = item.child(id_head)?.children();
            let id = id_form.get(1)?.atom()?;
            if net_form.len() != 2
                || net_form.get(1) != Some(net)
                || id_form.len() != 2
                || item
                    .children()
                    .iter()
                    .filter(|field| field.head() == Some("net"))
                    .count()
                    != 1
                || item
                    .children()
                    .iter()
                    .filter(|field| matches!(field.head(), Some("uuid" | "tstamp")))
                    .count()
                    != 1
                || item.child("locked").is_some()
                || item
                    .children()
                    .iter()
                    .any(|field| field.atom() == Some("locked"))
                || !native_uuid(id)
                || !ids.insert(id.to_string())
            {
                return None;
            }
            Some(item.clone())
        })
        .collect()
}

fn native_uuid(id: &str) -> bool {
    id.len() == 36
        && id != "00000000-0000-0000-0000-000000000000"
        && id.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
            }
        })
}

fn collect_ids(node: &Expr, ids: &mut BTreeSet<String>) {
    if matches!(node.head(), Some("uuid" | "tstamp"))
        && let Some(id) = node.children().get(1).and_then(Expr::atom)
    {
        ids.insert(id.to_ascii_lowercase());
    }
    for child in node.children() {
        collect_ids(child, ids);
    }
}

/// A single snapshot can supply both placement and copper candidates.
pub struct KiCadRouteReuseSource {
    pub board: Expr,
    path: PathBuf,
    sha256: String,
}

pub fn read_route_reuse(prior_path: &Path) -> Result<KiCadRouteReuseSource, String> {
    let path = fs::canonicalize(prior_path)
        .map_err(|error| format!("reuse routes {}: {error}", prior_path.display()))?;
    let source = fs::read_to_string(&path)
        .map_err(|error| format!("reuse routes {}: {error}", path.display()))?;
    let previous = crate::canonical_order::parse_board(&source)
        .map_err(|error| format!("reuse routes {}: {error}", path.display()))?;
    if previous.head() != Some("kicad_pcb") {
        return Err(format!(
            "reuse routes {} must be a KiCad PCB",
            path.display()
        ));
    }
    Ok(KiCadRouteReuseSource {
        board: previous,
        path,
        sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
    })
}

/// Build a scratch board from the current PCB and qualifying prior tracks,
/// arcs and vias. The returned board still needs native acceptance testing.
pub fn prepare_route_reuse(
    prior_path: &Path,
    current: &Expr,
    config: &KiCadBoardRouterConfig,
) -> Result<(Option<Expr>, KiCadRouteReuse), String> {
    prepare_route_reuse_from(&read_route_reuse(prior_path)?, current, config)
}

pub fn prepare_route_reuse_from(
    source: &KiCadRouteReuseSource,
    current: &Expr,
    config: &KiCadBoardRouterConfig,
) -> Result<(Option<Expr>, KiCadRouteReuse), String> {
    prepare_route_reuse_for_project(source, current, None, config)
}

/// Build a candidate whose current native project companions are known.
pub fn prepare_route_reuse_for_project(
    source: &KiCadRouteReuseSource,
    current: &Expr,
    current_board: Option<&Path>,
    config: &KiCadBoardRouterConfig,
) -> Result<(Option<Expr>, KiCadRouteReuse), String> {
    let previous = &source.board;
    let mut report = KiCadRouteReuse {
        path: source.path.clone(),
        sha256: source.sha256.clone(),
        status: "skipped".into(),
        reason: None,
        matched_nets: Vec::new(),
        extended_nets: Vec::new(),
        renamed_nets: Vec::new(),
        renamed_references: BTreeMap::new(),
        partial_nets: BTreeMap::new(),
        skipped_nets: BTreeMap::new(),
        keepout_skipped_nets: Vec::new(),
        segments: 0,
        arcs: 0,
        vias: 0,
        trial_work_seconds: 0.0,
        remaining_budget_seconds: 0.0,
        native_checks: 0,
        native_check_wall_seconds: 0.0,
        baseline_schematic_parity_by_type: BTreeMap::new(),
        shared_final_drc: false,
        new_findings: BTreeMap::new(),
        new_finding_examples: Vec::new(),
    };
    if !compatible_layer_stack(previous, current)? {
        report.reason = Some("the board layer stacks differ".into());
        return Ok((None, report));
    }
    if config.route_reuse_clearance_unverified {
        report.reason =
            Some("router-only clearance overrides cannot be checked by KiCad DRC".into());
        return Ok((None, report));
    }
    let mut rules = crate::net_classes::with_net_classes(config, current)?;
    rules.apply_width_floor();
    let legacy_width =
        current_board.and_then(|path| legacy_escape_width(source, current, path, &rules));
    let names = NetNames::of(current);
    let old_names = NetNames::of(previous);
    let layers = LayerTable::from_pcb(current)?;
    let keepouts = keepouts::CopperKeepouts::of(current, &layers);
    let old_codes = net_codes(previous);
    let current_codes = net_codes(current);
    let identities = RouteReuseIdentities::of(previous, current);
    report.renamed_references = identities.renamed();
    // A wholly unlinked pair uses the previous reference/pad census,
    // including its established diagnostics for duplicate references.
    let (previous_side, current_side) = if identities.has_schematic_links {
        (
            ContactSide::Previous(&identities),
            ContactSide::Current(&identities),
        )
    } else {
        (ContactSide::Native, ContactSide::Native)
    };
    let old_pads = pad_groups(previous, &old_codes, previous_side);
    let current_pads = pad_groups(current, &current_codes, current_side);
    let old_pours = partial::net_zone_layers(previous, &old_codes, &layers, &old_names);
    let current_pours = partial::net_zone_layers(current, &current_codes, &layers, &names);
    let kept: BTreeSet<_> = track_items(current)
        .filter_map(|item| item_net(item, &current_codes))
        .collect();
    let mut prior_copper = BTreeMap::<&str, Vec<&Expr>>::new();
    for item in track_items(previous) {
        if let Some(name) = item_net(item, &old_codes).filter(|name| !name.is_empty()) {
            prior_copper.entry(name).or_default().push(item);
        }
    }
    let mut ids = BTreeSet::new();
    collect_ids(current, &mut ids);
    let id_head = if form_atom(current, "version", 1)
        .and_then(|version| version.parse::<u32>().ok())
        .is_some_and(|version| version < 20240101)
    {
        "tstamp"
    } else {
        "uuid"
    };
    let mut copied = Vec::new();
    let mut partial_candidates = 0;
    let mut legacy_segments = 0;
    let mut planned = None;
    for (old_net, items) in prior_copper {
        let (name, net, mut items, partial, splice) =
            match target_group(&old_pads, &current_pads, old_net) {
                Ok((name, net)) => (name, net, items, None, None),
                Err(reason) => {
                    let candidate = matches!(reason, "changed_pad_contacts" | "changed_terminals")
                        .then(|| partial::changed_leaf(&old_pads, &current_pads, old_net))
                        .flatten();
                    let Some((name, net, leaf)) = candidate else {
                        *report.skipped_nets.entry(reason.into()).or_default() += 1;
                        continue;
                    };
                    if kept.contains(name) {
                        *report
                            .skipped_nets
                            .entry("current_copper".into())
                            .or_default() += 1;
                        continue;
                    }
                    if partial_candidates >= partial::MAX_CANDIDATES {
                        *report
                            .skipped_nets
                            .entry("partial_candidate_limit".into())
                            .or_default() += 1;
                        continue;
                    }
                    partial_candidates += 1;
                    let planned = match &planned {
                        Some(pours) => pours,
                        None => planned.insert(planned_pours(current, config)?),
                    };
                    let normalized = names.normalize(name);
                    let current_zone_layers = current_pours.get(normalized).copied().unwrap_or(0)
                        | planned
                            .iter()
                            .filter(|request| names.normalize(&request.net) == normalized)
                            .fold(0, |mask, request| {
                                mask | layers
                                    .mask_of(request.layers.iter().map(String::as_str))
                                    .unwrap_or_else(|_| layers.all())
                            });
                    let Some(leaf_edit) = partial::reuse_leaf(
                        partial::LeafBoard {
                            pcb: previous,
                            identities: identities.has_schematic_links.then_some(&identities),
                            net: &old_pads.nets[old_net],
                            zone_layers: old_pours
                                .get(old_names.normalize(old_net))
                                .copied()
                                .unwrap_or(0),
                        },
                        partial::LeafBoard {
                            pcb: current,
                            identities: None,
                            net,
                            zone_layers: current_zone_layers,
                        },
                        &layers,
                        leaf,
                        &items,
                    ) else {
                        *report
                            .skipped_nets
                            .entry("unsupported_partial_topology".into())
                            .or_default() += 1;
                        continue;
                    };
                    (
                        name,
                        net,
                        leaf_edit.retained,
                        Some(leaf_edit.evidence),
                        leaf_edit.splice,
                    )
                }
            };
        if kept.contains(name) {
            *report
                .skipped_nets
                .entry("current_copper".into())
                .or_default() += 1;
            continue;
        }
        items.extend(splice.as_ref());
        let legacy = legacy_width
            .filter(|_| {
                partial.is_none()
                    && old_net == name
                    && report.renamed_references.is_empty()
                    && old_pads.nets[old_net].terminals == net.terminals
            })
            .map(|width| (width, net));
        let eligible_legacy =
            match check_route_geometry(&rules, &layers, names.normalize(name), &items, legacy) {
                Ok(count) => count,
                Err(reason) => {
                    *report.skipped_nets.entry(reason.into()).or_default() += 1;
                    continue;
                }
            };
        if keepouts.blocks_any(&items, &layers) {
            *report
                .skipped_nets
                .entry("copper_keepout".into())
                .or_default() += 1;
            report.keepout_skipped_nets.push(name.to_string());
            continue;
        }
        let Some(net_field) = &net.native_net else {
            continue;
        };
        report.matched_nets.push(name.to_string());
        if let Some(evidence) = partial {
            report.partial_nets.insert(name.to_string(), evidence);
        }
        if old_pads.nets[old_net].terminals.len() < net.terminals.len() {
            report.extended_nets.push(name.to_string());
        }
        if old_net != name {
            report.renamed_nets.push(name.to_string());
        }
        if splice.is_some() {
            let splice = items.pop().expect("the last item is the replacement tip");
            copied.push(copper_item(splice, net_field, id_head, &mut ids));
        }
        legacy_segments += eligible_legacy;
        for item in items {
            match item.head() {
                Some("segment") => report.segments += 1,
                Some("arc") => report.arcs += 1,
                Some("via") => report.vias += 1,
                _ => unreachable!("track items"),
            }
            copied.push(copper_item(item, net_field, id_head, &mut ids));
        }
    }
    if copied.is_empty() {
        report.reason = Some("no prior copper passed the current route reuse checks".into());
        return Ok((None, report));
    }
    // A complete same-contact replay can keep the original track sequence
    // and IDs exactly. Compare the nets that actually supplied copper:
    // reannotation can change unconnected pad net names without changing
    // any retained net. The original copper set and IDs still must pass
    // every preceding reuse check and the exact emission check below.
    if track_items(current).next().is_none()
        && report
            .matched_nets
            .iter()
            .all(|net| old_pads.nets.get(net) == current_pads.nets.get(net))
        && report.partial_nets.is_empty()
        && report.extended_nets.is_empty()
        && report.renamed_nets.is_empty()
        && report.skipped_nets.is_empty()
        && copied.len() == track_items(previous).count()
        && let Some(unchanged) =
            stable_copper(previous, current, &current_pads, &old_codes, id_head)
    {
        copied = unchanged;
    }
    let mut candidate = current.clone();
    if let Expr::List(items) = &mut candidate {
        items.extend(copied);
    }
    if legacy_segments > 0 {
        eprintln!(
            "route reuse: {legacy_segments} legacy protected-escape segments matched their original UUID and geometry under the same native rules"
        );
    }
    report.status = "prepared".into();
    Ok((Some(candidate), report))
}

fn legacy_escape_width(
    source: &KiCadRouteReuseSource,
    current: &Expr,
    current_board: &Path,
    config: &KiCadBoardRouterConfig,
) -> Option<f64> {
    let rules = config.project_rules.as_ref()?;
    if !config.net_classes.is_empty()
        || config.route_reuse_clearance_unverified
        || rules.custom_rules_error.is_some()
        || rules
            .custom_rules
            .iter()
            .any(|rule| rule.constraint == "track_width")
        || source.board.child("setup") != current.child("setup")
        || fs::read(source.path.with_extension("kicad_pro")).ok()?
            != fs::read(current_board.with_extension("kicad_pro")).ok()?
    {
        return None;
    }
    let prior = fs::read(source.path.with_extension("kicad_dru"));
    let current = fs::read(current_board.with_extension("kicad_dru"));
    let same_custom = match (prior, current) {
        (Ok(a), Ok(b)) => a == b,
        (Err(a), Err(b)) => [a.kind(), b.kind()]
            .into_iter()
            .all(|kind| kind == std::io::ErrorKind::NotFound),
        _ => false,
    };
    same_custom
        .then_some(config.neck_width_mm?)
        .filter(|width| width.is_finite())
}

fn check_route_geometry(
    config: &KiCadBoardRouterConfig,
    layers: &LayerTable,
    net: &str,
    items: &[&Expr],
    legacy: Option<(f64, &PadNet)>,
) -> Result<usize, &'static str> {
    if config.diff_pairs_on() && config.diff_pair_rules.contains_key(net) {
        return Err("differential_pair");
    }
    if config.length_matching_on()
        && config
            .length_rules
            .iter()
            .any(|rule| rule.nets.iter().any(|member| member.net == net))
    {
        return Err("length_or_skew_rule");
    }
    let geometry = config
        .connection_rules
        .get(net)
        .or(config.default_rules.as_ref());
    let matches = |item: &Expr, field, required: f64| {
        // KiCad stores copper dimensions on a one-nanometre lattice.
        form_f64(item, field, 1).is_ok_and(|actual| (actual - required).abs() <= 0.000001)
    };
    let mut legacy_segments = 0;
    let floor = config.width_floor();
    for item in items {
        if item.head() == Some("via") {
            let span: Vec<_> = item
                .child("layers")
                .map(Expr::children)
                .unwrap_or_default()
                .iter()
                .skip(1)
                .filter_map(Expr::atom)
                .collect();
            if span != ["F.Cu", "B.Cu"]
                || config.via_in_pad.is_some()
                || geometry.is_some_and(|rule| {
                    !matches(item, "size", rule.via_size_mm)
                        || !matches(item, "drill", rule.via_drill_mm)
                })
            {
                return Err("incompatible_via_geometry");
            }
        } else {
            let Some(layer) = form_atom(item, "layer", 1) else {
                return Err("incompatible_track_layer");
            };
            let Some(index) = layers.index(layer) else {
                return Err("incompatible_track_layer");
            };
            if config
                .track_disallowed
                .get(net)
                .is_some_and(|mask| mask & (1 << index) != 0)
            {
                return Err("incompatible_track_layer");
            }
            if config.project_rules.as_ref().is_some_and(|project| {
                project.layer_geometry_rules.iter().any(|rule| {
                    rule.opt_mm.is_some() && rule.layers.iter().any(|name| name == layer)
                })
            }) {
                return Err("layer_geometry_rule");
            }
            if form_f64(item, "width", 1).is_ok_and(|width| width + 0.000001 < floor) {
                return Err("incompatible_track_width");
            }
            if let Some(rule) = geometry
                && !matches(item, "width", rule.trace_width_mm)
            {
                let eligible = legacy.is_some_and(|(historical, pads)| {
                    let native_minimum = config
                        .project_rules
                        .as_ref()
                        .and_then(|rules| rules.minimums.as_ref())
                        .map_or(0.0, |minimums| minimums.track_width_mm);
                    let custom_minimum = config
                        .net_rules
                        .as_ref()
                        .and_then(|rules| rules.width_floor.get(net))
                        .copied()
                        .unwrap_or(0.0);
                    config.narrows_class_of(net, rule)
                        && historical > rule.trace_width_mm
                        && historical + 0.000001 >= native_minimum.max(custom_minimum)
                        && matches(item, "width", historical)
                        && pads.terminals.keys().any(|(reference, pad)| {
                            crate::open_repair::legacy_escape_segment(item, reference, pad)
                        })
                });
                if !eligible {
                    return Err("incompatible_track_width");
                }
                legacy_segments += 1;
            }
        }
    }
    Ok(legacy_segments)
}

/// Runs the same unfiltered all-track DRC on the unseeded and routed projects.
fn write_route_reuse_drc(project: &Path, board_id: &str, report: &Path) -> Result<(), String> {
    if !project.join(format!("{board_id}.kicad_sch")).is_file() {
        return Err("schematic parity is unavailable for the current board".into());
    }
    let board = project.join(format!("{board_id}.kicad_pcb"));
    let source = fs::read_to_string(&board).map_err(|error| error.to_string())?;
    let pcb = parse(&source)?;
    let refill = pcb.child("zone").is_some();
    native_report::run_route_reuse_drc(&board, report, refill)?;
    Ok(())
}

/// Route and layout reserve the same small part of their remaining work.
pub fn route_reuse_trial_budget(available: f64) -> f64 {
    (available * 0.10).clamp(0.0, 30.0)
}

impl KiCadRouteReuse {
    /// Reports input parity before any borrowed copper is routed.
    pub fn check_baseline_parity(&mut self, report: &Path) -> Result<(), String> {
        let native = read_typed_json(report)?;
        self.baseline_schematic_parity_by_type = drc::parity_counts(&native)?;
        if self.baseline_schematic_parity_by_type.is_empty() {
            return Ok(());
        }
        let findings = self
            .baseline_schematic_parity_by_type
            .iter()
            .map(|(kind, count)| format!("{kind} {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        Err(format!(
            "KiCad found schematic parity issues in the current input ({findings})"
        ))
    }

    /// Compares the verified trial to its unseeded input and records its
    /// delta. The examples here describe the trial's findings.
    pub fn compare_drc(&mut self, baseline: &Path, trial: &Path) -> Result<(), String> {
        let delta = drc::compare_drc(baseline, trial)?;
        self.new_findings = delta.counts;
        self.new_finding_examples = delta.examples;
        Ok(())
    }

    pub fn copy_closed_drc(
        &mut self,
        routed: &KiCadBoardRouterResult,
        project: &Path,
        board_id: &str,
        output: &Path,
    ) -> bool {
        let proof = routed
            .closed_reuse_drc
            .as_ref()
            .filter(|proof| proof.copy_if_current(project, board_id, output));
        self.shared_final_drc = proof.is_some_and(|proof| proof.shares_final_verification());
        proof.is_some()
    }

    /// Checks with KiCad and records comparison-only check overhead. The
    /// normal router's own checks are accounted for separately.
    pub fn write_drc(
        &mut self,
        project: &Path,
        board_id: &str,
        report: &Path,
    ) -> Result<(), String> {
        let start = Instant::now();
        let checked = write_route_reuse_drc(project, board_id, report);
        self.native_checks += 1;
        self.native_check_wall_seconds += start.elapsed().as_secs_f64();
        checked
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn beside(output: &Path) -> Result<Self, String> {
        let parent = output
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let start = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = parent.join(format!(
            ".pcb-maker-route-reuse-{}-{start}",
            std::process::id()
        ));
        fs::create_dir(&path).map_err(|error| format!("creating {}: {error}", path.display()))?;
        Ok(Self(path))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn persist_report(
    result: &mut KiCadBoardRouterResult,
    report: KiCadRouteReuse,
    output: &Path,
) -> Result<(), String> {
    result.route_reuse = Some(report);
    crate::write_atomically(
        &output.join("board-router.json"),
        serde_json::to_string_pretty(result)
            .map_err(|error| error.to_string())?
            .as_bytes(),
    )
}

fn try_reuse(
    source: &Path,
    board_id: &str,
    current_source: &str,
    candidate: &Expr,
    scratch: &Path,
    config: &KiCadBoardRouterConfig,
    report: &mut KiCadRouteReuse,
) -> Result<KiCadBoardRouterResult, String> {
    let staged = scratch.join("current");
    copy_directory_tree(source, &staged)?;
    let staged_board = staged.join(format!("{board_id}.kicad_pcb"));
    if fs::read_to_string(&staged_board).map_err(|error| error.to_string())? != current_source {
        return Err("the current board changed while staging the reuse trial".into());
    }
    let baseline = scratch.join("input-reuse-drc.json");
    report.write_drc(&staged, board_id, &baseline)?;
    report.check_baseline_parity(&baseline)?;
    if config
        .deadline
        .is_some_and(|deadline| Instant::now() >= deadline)
    {
        return Err("the routing deadline expired during the input check".into());
    }
    fs::write(staged_board, format!("{}\n", encode(candidate)))
        .map_err(|error| error.to_string())?;
    let routed = scratch.join("routed");
    // If the ladder fails before returning a work report, debit its entire
    // reserved share, including the existing ladder's work tolerance.
    report.trial_work_seconds +=
        ladder_work_bound(config.ladder_budget_seconds.unwrap_or_default());
    let mut verified_config = config.clone();
    verified_config.route_reuse_preserve_input_layers = true;
    let result = route_kicad_board(&staged, board_id, &routed, &verified_config)?;
    report.trial_work_seconds = result.work_seconds;
    if result.native.is_none() {
        return Err("the seeded router did not complete native verification".into());
    }
    check_reused_copper(candidate, &routed.join(format!("{board_id}.kicad_pcb")))?;
    let drc = scratch.join("trial-reuse-drc.json");
    if !report.copy_closed_drc(&result, &routed, board_id, &drc) {
        report.write_drc(&routed, board_id, &drc)?;
    }
    report.compare_drc(&baseline, &drc)?;
    if !report.new_findings.is_empty() {
        return Err(format!(
            "KiCad found new DRC findings in the seeded trial ({})",
            report
                .new_findings
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for path in [&baseline, &drc] {
        fs::rename(
            path,
            routed.join(path.file_name().expect("reuse report filename")),
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(result)
}

/// In route mode, try prior copper under part of the normal work budget.
/// The current board supplies all authored rules, schematic and footprints.
/// If the independent native verification rejects reuse, restart with the
/// original board and the unspent work. Only a verified trial is published.
pub fn route_kicad_board_reusing(
    source: &Path,
    board_id: &str,
    output: &Path,
    prior_path: &Path,
    config: &KiCadBoardRouterConfig,
) -> Result<KiCadBoardRouterResult, String> {
    if config.existing_tracks == KiCadExistingTracks::Strip {
        return Err(crate::exit::input_error(
            "--reuse-routes needs the keep existing_tracks policy",
        ));
    }
    if config.tuning.engine.as_deref() == Some("topological") {
        return Err(crate::exit::input_error(
            "--reuse-routes requires the detailed router",
        ));
    }
    let mut base = config.clone();
    if base.deadline.is_none()
        && let Some(seconds) = base.deadline_seconds
    {
        let stretch = crate::work_rate::work_rate().wall_stretch(false);
        base.deadline =
            Some(Instant::now() + std::time::Duration::from_secs_f64(seconds.max(0.0) * stretch));
    }
    let source_board = source.join(format!("{board_id}.kicad_pcb"));
    let text = fs::read_to_string(&source_board)
        .map_err(|error| format!("reading {}: {error}", source_board.display()))?;
    let current = crate::canonical_order::parse_board(&text).map_err(crate::exit::input_error)?;
    let prior = read_route_reuse(prior_path).map_err(crate::exit::input_error)?;
    let (candidate, mut report) =
        prepare_route_reuse_for_project(&prior, &current, Some(&source_board), &base)
            .map_err(crate::exit::input_error)?;
    let budget = base.ladder_budget_seconds.unwrap_or(1200.0);
    report.remaining_budget_seconds = budget;
    if let Some(candidate) = candidate {
        if base.skip_native_verification || !source.join(format!("{board_id}.kicad_sch")).is_file()
        {
            report.status = "skipped".into();
            report.reason =
                Some("reuse requires native verification with current schematic parity".into());
        } else if base.pin_swaps.is_some() {
            report.status = "skipped".into();
            report.reason = Some(
                "route-mode pin swaps may change terminal membership after reuse matching".into(),
            );
        } else if base
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            report.status = "skipped".into();
            report.reason = Some("the routing deadline expired before the reuse trial".into());
        } else {
            let mut trial = base.clone();
            trial.ladder_budget_seconds = Some(route_reuse_trial_budget(budget));
            let scratch = Scratch::beside(output);
            let verdict = scratch.as_ref().map_err(Clone::clone).and_then(|scratch| {
                try_reuse(
                    source,
                    board_id,
                    &text,
                    &candidate,
                    &scratch.0,
                    &trial,
                    &mut report,
                )
            });
            report.remaining_budget_seconds = (budget - report.trial_work_seconds).max(0.0);
            match verdict {
                Ok(mut result) => {
                    report.status = "accepted".into();
                    eprintln!(
                        "route reuse: accepted prior copper on {} nets ({:.2} s of work)",
                        report.matched_nets.len(),
                        report.trial_work_seconds
                    );
                    result.work_bound_seconds = Some(ladder_work_bound(budget));
                    let scratch = scratch
                        .as_ref()
                        .expect("trial used scratch")
                        .0
                        .join("routed");
                    persist_report(&mut result, report, &scratch)?;
                    fs::rename(&scratch, output).map_err(|error| {
                        format!("publishing verified route {}: {error}", output.display())
                    })?;
                    return Ok(result);
                }
                Err(reason) => {
                    report.status = "fallback".into();
                    report.reason = Some(reason);
                    // New budget and original absolute deadline apply to unseeded work.
                    if report.trial_work_seconds > 0.0 {
                        base.ladder_budget_seconds = Some(report.remaining_budget_seconds);
                    }
                }
            }
        }
    }
    eprintln!(
        "route reuse: {} ({})",
        report.status,
        report.reason.as_deref().unwrap_or_default()
    );
    let mut result = route_kicad_board(source, board_id, output, &base)?;
    result.work_seconds += report.trial_work_seconds;
    result.work_bound_seconds = Some(ladder_work_bound(budget));
    persist_report(&mut result, report, output)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn footprint(reference: &str, net: &str, x: f64) -> String {
        format!(
            r#"(footprint "Test:Pad" (layer "F.Cu") (at {x} 4)
                (property "Reference" "{reference}")
                (pad "1" smd rect (at 0 0) (size 1 2) (layers "F.Cu" "F.Mask") (net {net})))"#
        )
    }

    fn board(items: &[String]) -> Expr {
        parse(&format!(
            r#"(kicad_pcb (version 20260206) (layers
                (0 "F.Cu" signal) (4 "In1.Cu" power) (6 "In2.Cu" power) (31 "B.Cu" signal)) {})"#,
            items.join(" ")
        ))
        .unwrap()
    }

    fn segment(net: &str, id: &str) -> String {
        format!(
            r#"(segment (start 1 4) (end 2 4) (width 0.2) (layer "F.Cu") (net {net}) (uuid "{id}"))"#
        )
    }

    #[test]
    fn unchanged_reuse_preserves_interleaved_track_order_and_unique_ids() {
        let alpha = r#""Alpha""#;
        let beta = r#""Beta""#;
        let parts = [
            footprint("A1", alpha, 1.0),
            footprint("A2", alpha, 3.0),
            footprint("B1", beta, 4.0),
            footprint("B2", beta, 5.0),
        ];
        let first_id = "afe0f3aa-6507-434d-9a36-f7cee206f343";
        let second_id = "b61cbbd8-079d-411c-9ea0-96921334930f";
        let third_id = "ced19164-3ab8-4091-bcaa-c0851ac6fe15";
        let original_tracks = [
            segment(alpha, first_id),
            segment(beta, second_id)
                .replace("1 4", "4 4")
                .replace("2 4", "5 4"),
            segment(alpha, third_id).replace("(start 1 4)", "(start 3 4)"),
        ];
        let donor = KiCadRouteReuseSource {
            board: board(&[parts.as_slice(), original_tracks.as_slice()].concat()),
            path: "donor.kicad_pcb".into(),
            sha256: "fixture".into(),
        };
        let prepare = |parts: &[String]| {
            prepare_route_reuse_from(&donor, &board(parts), &KiCadBoardRouterConfig::default())
                .unwrap()
        };
        let (saved, reuse) = prepare(&parts);
        assert_eq!(reuse.matched_nets, ["Alpha", "Beta"]);
        assert_eq!(
            track_items(&saved.unwrap()).cloned().collect::<Vec<_>>(),
            track_items(&donor.board).cloned().collect::<Vec<_>>()
        );

        let with_nc = |reference: &str| {
            [
                parts.as_slice(),
                &[footprint(
                    reference,
                    &format!(r#""unconnected-({reference}-Pad1)""#),
                    8.0,
                )],
            ]
            .concat()
        };
        let old_with_nc = KiCadRouteReuseSource {
            board: board(&[with_nc("U1").as_slice(), original_tracks.as_slice()].concat()),
            path: "donor.kicad_pcb".into(),
            sha256: "fixture".into(),
        };
        let (saved, report) = prepare_route_reuse_from(
            &old_with_nc,
            &board(&with_nc("U7")),
            &KiCadBoardRouterConfig::default(),
        )
        .unwrap();
        assert!(report.renamed_nets.is_empty());
        assert_eq!(
            track_items(&saved.unwrap()).cloned().collect::<Vec<_>>(),
            track_items(&old_with_nc.board).cloned().collect::<Vec<_>>(),
        );

        let collision = format!(r#"(gr_line (uuid "{}"))"#, first_id.to_ascii_uppercase());
        let (saved, reuse) = prepare(&[parts.as_slice(), &[collision]].concat());
        assert_eq!(reuse.segments, 3);
        let saved = saved.unwrap();
        assert!(track_items(&saved).all(|item| form_atom(item, "uuid", 1) != Some(first_id)));
        assert_eq!(
            item_net(track_items(&saved).last().unwrap(), &BTreeMap::new()),
            Some("Beta")
        );
    }

    #[test]
    fn reannotation_matches_contacts_by_instance_and_declines_a_recycled_name() {
        let pin = |reference, path, net, x| {
            footprint(reference, net, x)
                .replace("(property", &format!("(path \"/{path}\") (property"))
        };
        let alpha = r#""Alpha""#;
        let beta = r#""Beta""#;
        let donor = KiCadRouteReuseSource {
            board: board(&[
                pin("C1", "11111111", alpha, 1.0),
                pin("C2", "22222222", beta, 4.0),
                pin("U1", "33333333", alpha, 3.0),
                pin("U2", "44444444", beta, 5.0),
                segment(alpha, "e65935b2-950c-4acf-b860-d79aa03f7900"),
                segment(beta, "0ac03860-fbb2-49de-a6bd-2c5a2de6db21"),
            ]),
            path: "donor.kicad_pcb".into(),
            sha256: "fixture".into(),
        };
        let current = |alpha_path| {
            board(&[
                pin("C2", alpha_path, alpha, 1.0),
                pin("C1", "22222222", beta, 4.0),
                pin("U1", "33333333", alpha, 3.0),
                pin("U2", "44444444", beta, 5.0),
            ])
        };
        let prepare = |pcb| {
            prepare_route_reuse_from(&donor, &pcb, &KiCadBoardRouterConfig::default()).unwrap()
        };
        let (copied, report) = prepare(current("11111111"));
        assert_eq!(report.matched_nets, ["Alpha", "Beta"]);
        assert_eq!(report.segments, 2);
        assert_eq!(
            report.renamed_references,
            BTreeMap::from([("C1".into(), "C2".into()), ("C2".into(), "C1".into()),])
        );
        assert_eq!(
            footprint_reference(copied.unwrap().child("footprint").unwrap()).as_deref(),
            Some("C2")
        );

        let (copied, report) = prepare(current("aaaaaaaa"));
        assert_eq!(report.matched_nets, ["Beta"]);
        assert_eq!(
            report.renamed_references,
            BTreeMap::from([("C2".into(), "C1".into())])
        );
        assert_eq!(
            report
                .skipped_nets
                .get("ambiguous_or_insufficient_prior_terminals"),
            Some(&1)
        );
        assert_eq!(track_items(&copied.unwrap()).count(), 1);
    }

    #[test]
    fn a_moved_or_disconnected_leaf_preserves_the_unchanged_copper_tree() {
        let net = r#""Rail""#;
        let a = footprint("A1", net, 1.0);
        let junction = footprint("J1", net, 3.0)
            .replace("smd", "thru_hole")
            .replace(
                r#"(layers "F.Cu" "F.Mask")"#,
                r#"(drill 0.4) (layers "*.Cu" "*.Mask")"#,
            );
        let b = footprint("B1", net, 6.0).replace("F.Cu", "B.Cu");
        let old_leaf = footprint("C1", net, 9.0);
        let path = [
            segment(net, "a").replace("2 4", "3 4"),
            segment(net, "b")
                .replace("1 4", "3 4")
                .replace("2 4", "6 4")
                .replace("F.Cu", "B.Cu"),
            segment(net, "branch1")
                .replace("1 4", "3 4")
                .replace("2 4", "8.5 4"),
            segment(net, "branch2")
                .replace("1 4", "8.5 4")
                .replace("2 4", "9 4"),
        ];
        let donor = |extra: &[String]| KiCadRouteReuseSource {
            board: board(
                &[
                    &[a.clone(), junction.clone(), b.clone(), old_leaf.clone()][..],
                    &path,
                    extra,
                ]
                .concat(),
            ),
            path: "prior.kicad_pcb".into(),
            sha256: "fixture".into(),
        };
        let prepare = |source: KiCadRouteReuseSource, items: &[String]| {
            prepare_route_reuse_from(&source, &board(items), &KiCadBoardRouterConfig::default())
                .unwrap()
        };
        let current = [
            a.clone(),
            junction.clone(),
            b.clone(),
            footprint("C1", net, 10.0),
        ];
        let (candidate, report) = prepare(donor(&[]), &current);
        assert_eq!(report.matched_nets, ["Rail"]);
        let evidence = &report.partial_nets["Rail"];
        assert_eq!(evidence.pruned_terminal, "C1:1");
        assert_eq!(evidence.retained_terminals, 3);
        assert_eq!(evidence.pending_terminals, 0);
        assert_eq!(evidence.retained_segments, 3);
        assert_eq!(evidence.pruned_segments, 1);
        assert_eq!(evidence.spliced_segments, 1);
        assert_eq!(report.segments, 3);
        let spliced = candidate.unwrap();
        assert_eq!(track_items(&spliced).count(), 4);
        // The old pad overlaps the adjoining track at the splice joint.
        assert!(track_items(&spliced).any(|item| {
            form_xy(item, "start") == Ok([8.5, 4.0]) && form_xy(item, "end") == Ok([10.0, 4.0])
        }));

        let linked = |part: &str| part.replace("(property", r#"(path "/12345678") (property"#);
        let mut renamed_donor = donor(&[]);
        let Expr::List(items) = &mut renamed_donor.board else {
            unreachable!()
        };
        *items
            .iter_mut()
            .find(|item| footprint_reference(item).as_deref() == Some("C1"))
            .unwrap() = parse(&linked(&old_leaf)).unwrap();
        let renamed_current = [
            a.clone(),
            junction.clone(),
            b.clone(),
            linked(&footprint("C7", net, 10.0)),
        ];
        let (candidate, report) = prepare(renamed_donor, &renamed_current);
        assert!(candidate.is_some());
        assert_eq!(report.partial_nets["Rail"].pruned_terminal, "C7:1");
        assert_eq!(report.partial_nets["Rail"].spliced_segments, 1);

        for removed in [
            vec![a.clone(), junction.clone(), b.clone()],
            vec![
                a.clone(),
                junction.clone(),
                b.clone(),
                footprint("C1", r#""unconnected-(C1-Pad1)""#, 9.0),
            ],
        ] {
            let (candidate, report) = prepare(donor(&[]), &removed);
            assert_eq!(report.partial_nets["Rail"].pending_terminals, 0);
            assert_eq!(report.partial_nets["Rail"].spliced_segments, 0);
            assert_eq!(track_items(&candidate.unwrap()).count(), 2);
        }

        let moved_junction = [
            a.clone(),
            junction.replace("at 3 4", "at 3 6"),
            b.clone(),
            old_leaf.clone(),
        ];
        let (candidate, report) = prepare(donor(&[]), &moved_junction);
        assert!(candidate.is_none());
        assert_eq!(report.skipped_nets["unsupported_partial_topology"], 1);

        let (candidate, report) = prepare(
            donor(&[segment(net, "cross")
                .replace("1 4", "4 3")
                .replace("2 4", "4 5")]),
            &current,
        );
        assert!(candidate.is_none());
        assert_eq!(report.skipped_nets["unsupported_partial_topology"], 1);

        let far_move = [
            a.clone(),
            current[1].clone(),
            b.clone(),
            footprint("C1", net, 25.0),
        ];
        let (candidate, report) = prepare(donor(&[]), &far_move);
        assert!(candidate.is_none());
        assert_eq!(report.skipped_nets["unsupported_partial_topology"], 1);

        let zone = |layers: &str| {
            format!(
                r#"(zone (net "Rail") {layers}
                    (polygon (pts (xy 0 0) (xy 30 0) (xy 30 10) (xy 0 10))))"#
            )
        };
        let adjacent_layer = zone(r#"(layer "In1.Cu")"#);
        let (candidate, report) = prepare(
            donor(std::slice::from_ref(&adjacent_layer)),
            &[current.as_slice(), std::slice::from_ref(&adjacent_layer)].concat(),
        );
        assert!(candidate.is_some());
        assert_eq!(report.partial_nets["Rail"].spliced_segments, 1);

        let crossing_zone = zone(r#"(layers "F.Cu" "In1.Cu")"#);
        let (candidate, report) = prepare(
            donor(&[adjacent_layer]),
            &[current.as_slice(), &[crossing_zone]].concat(),
        );
        assert!(candidate.is_none());
        assert_eq!(report.skipped_nets["unsupported_partial_topology"], 1);

        let planned = KiCadBoardRouterConfig {
            add_pours: vec![KiCadPourRequest {
                net: "Rail".into(),
                layers: vec!["F.Cu".into()],
                region: None,
                clearance_mm: None,
                thermal_bridge_width_mm: None,
                plane: false,
                keep_whole: Vec::new(),
            }],
            ..KiCadBoardRouterConfig::default()
        };
        let (candidate, report) =
            prepare_route_reuse_from(&donor(&[]), &board(&current), &planned).unwrap();
        assert!(candidate.is_none());
        assert_eq!(report.skipped_nets["unsupported_partial_topology"], 1);

        let keepout = r#"(zone (layer "F.Cu") (keepout (tracks not_allowed))
            (polygon (pts (xy 9.39 3.8) (xy 9.61 3.8) (xy 9.61 4.2) (xy 9.39 4.2))))"#;
        let (candidate, report) = prepare(
            donor(&[]),
            &[current.as_slice(), &[keepout.to_string()]].concat(),
        );
        assert!(candidate.is_none());
        assert_eq!(report.keepout_skipped_nets, ["Rail"]);

        let authored = [current.as_slice(), &[path[0].clone()]].concat();
        let (candidate, report) = prepare(donor(&[]), &authored);
        assert!(candidate.is_none());
        assert_eq!(report.skipped_nets["current_copper"], 1);
    }

    #[test]
    fn a_tee_can_lose_its_branch_only_when_the_original_trunk_survives() {
        let net = r#""Rail""#;
        let a = footprint("A1", net, 1.0);
        let b = footprint("B1", net, 9.0);
        let c = footprint("C1", net, 5.0).replace("(at 5 4)", "(at 5 8)");
        let trunk = segment(net, "trunk").replace("2 4", "9 4");
        let twig = segment(net, "twig")
            .replace("1 4", "5 8")
            .replace("2 4", "5 4");
        let donor = |twig| KiCadRouteReuseSource {
            board: board(&[a.clone(), b.clone(), c.clone(), trunk.clone(), twig]),
            path: "prior.kicad_pcb".into(),
            sha256: "fixture".into(),
        };
        let prepare = |source: KiCadRouteReuseSource, current: &[String]| {
            prepare_route_reuse_from(&source, &board(current), &KiCadBoardRouterConfig::default())
                .unwrap()
        };
        let (candidate, report) = prepare(donor(twig.clone()), &[a.clone(), b.clone()]);
        let branch = &report.partial_nets["Rail"];
        assert_eq!(branch.pruned_terminal, "C1:1");
        assert_eq!(branch.pruned_segments, 1);
        assert_eq!(branch.retained_segments, 1);
        let candidate = candidate.unwrap();
        let saved: Vec<_> = track_items(&candidate).collect();
        assert_eq!(saved.len(), 1);
        assert_eq!(form_xy(saved[0], "start").unwrap(), [1.0, 4.0]);
        assert_eq!(form_xy(saved[0], "end").unwrap(), [9.0, 4.0]);

        let (candidate, report) = prepare(donor(twig.clone()), &[b.clone(), c.clone()]);
        assert!(candidate.is_none());
        assert_eq!(report.skipped_nets["unsupported_partial_topology"], 1);

        let nearly = twig.replace("(end 5 4)", "(end 5 4.000001)");
        let (candidate, report) = prepare(donor(nearly), &[a, b]);
        assert!(candidate.is_none());
        assert_eq!(report.skipped_nets["unsupported_partial_topology"], 1);
    }

    #[test]
    fn legacy_protected_width_needs_its_geometry_identity_and_same_current_rules() {
        let parts = [
            footprint("U1", r#""Power""#, 1.0),
            footprint("U2", r#""Power""#, 2.0),
        ];
        let forms = r#"(start 1.000000 4.000000) (end 2.000000 4.000000) (width 0.1) (layer "F.Cu") (net "Power")"#;
        let uuid = pcb_kicad_file::kicad_sexpr::deterministic_uuid_for(&format!(
            "plane escape U1.1 {forms}"
        ));
        let segment = format!(r#"(segment {forms} (uuid "{uuid}"))"#);
        let current = board(&parts);
        let temporary = Scratch::beside(&std::env::temp_dir().join("legacy-escape")).unwrap();
        let path = temporary.0.join("previous.kicad_pcb");
        let current_path = temporary.0.join("current.kicad_pcb");
        let source = |segment: String| KiCadRouteReuseSource {
            board: board(&[parts[0].clone(), parts[1].clone(), segment]),
            path: path.clone(),
            sha256: "fixture".into(),
        };
        let defaults = KiCadConnectionRoutingRules {
            trace_width_mm: 0.09,
            clearance_mm: 0.09,
            via_size_mm: 0.35,
            via_drill_mm: 0.2,
        };
        let config = KiCadBoardRouterConfig {
            default_rules: Some(defaults),
            project_rules: Some(crate::project_rules::KiCadProjectRulesReport::default()),
            neck_width_mm: Some(0.1),
            ..Default::default()
        };
        let prepare = |source: &KiCadRouteReuseSource, config: &KiCadBoardRouterConfig| {
            prepare_route_reuse_for_project(source, &current, Some(&current_path), config).unwrap()
        };
        let rejected = |source: &KiCadRouteReuseSource, config: &KiCadBoardRouterConfig| {
            let (candidate, report) = prepare(source, config);
            assert!(candidate.is_none());
            assert_eq!(report.skipped_nets["incompatible_track_width"], 1);
        };

        let genuine = source(segment.clone());
        rejected(&genuine, &config); // Missing companion projects.
        fs::write(path.with_extension("kicad_pro"), "{}").unwrap();
        fs::write(current_path.with_extension("kicad_pro"), "{}").unwrap();
        let (candidate, report) = prepare(&genuine, &config);
        assert_eq!(report.segments, 1);
        let candidate = candidate.unwrap();
        assert_eq!(
            form_atom(track_items(&candidate).next().unwrap(), "uuid", 1),
            Some(uuid.as_str())
        );
        assert!(
            prepare_route_reuse_from(&genuine, &current, &config)
                .unwrap()
                .0
                .is_none()
        );

        rejected(&source(segment.replace("2.000000", "2.000001")), &config);
        rejected(&source(segment.replace("width 0.1", "width 0.11")), &config);
        let mut explicit = config.clone();
        explicit.net_classes.push(crate::KiCadNetClassRequest {
            name: "PowerClass".into(),
            nets: vec!["Power".into()],
            track_width_mm: Some(0.09),
            clearance_mm: None,
            via_diameter_mm: None,
            via_drill_mm: None,
        });
        rejected(&genuine, &explicit);
        fs::write(current_path.with_extension("kicad_dru"), "(version 1)").unwrap();
        rejected(&genuine, &config);
        fs::write(path.with_extension("kicad_dru"), "(version 1)").unwrap();
        assert!(prepare(&genuine, &config).0.is_some());
        fs::write(
            current_path.with_extension("kicad_pro"),
            r#"{"changed":true}"#,
        )
        .unwrap();
        rejected(&genuine, &config);
    }

    #[test]
    fn a_current_keepout_withholds_the_whole_prior_net_and_keeps_authored_copper() {
        let parts = [
            footprint("A1", r#""Blocked""#, 1.0),
            footprint("A2", r#""Blocked""#, 2.0),
            footprint("B1", r#""Clear""#, 3.0),
            footprint("B2", r#""Clear""#, 4.0),
            footprint("C1", r#""Authored""#, 5.0),
            footprint("C2", r#""Authored""#, 6.0),
        ];
        let mut prior_items = parts.to_vec();
        prior_items.extend([
            segment(r#""Blocked""#, "crosses"),
            segment(r#""Blocked""#, "elsewhere").replace("1 4", "1 7"),
            segment(r#""Clear""#, "far-side").replace("F.Cu", "B.Cu"),
            segment(r#""Authored""#, "old-authored"),
        ]);
        let source = KiCadRouteReuseSource {
            board: board(&prior_items),
            path: "previous.kicad_pcb".into(),
            sha256: String::new(),
        };
        let mut current_items = parts.to_vec();
        current_items.extend([
            segment(r#""Authored""#, "current-track"),
            r#"(zone (layer "F.Cu") (keepout (tracks not_allowed))
                (polygon (pts (xy 1.4 3.9) (xy 1.6 3.9) (xy 1.6 4.1) (xy 1.4 4.1))))"#
                .into(),
        ]);
        let current = board(&current_items);
        let (candidate, report) =
            prepare_route_reuse_from(&source, &current, &KiCadBoardRouterConfig::default())
                .unwrap();
        let candidate = candidate.unwrap();
        assert_eq!(report.keepout_skipped_nets, ["Blocked"]);
        assert_eq!(report.skipped_nets["copper_keepout"], 1);
        assert_eq!(report.skipped_nets["current_copper"], 1);
        assert_eq!(report.matched_nets, ["Clear"]);
        assert_eq!(report.segments, 1);
        assert_eq!(track_items(&candidate).next(), track_items(&current).next());
        assert_eq!(track_items(&candidate).count(), 2);
    }

    #[test]
    fn reuse_admits_a_renamed_extension_and_skips_moved_ambiguous_or_already_routed_nets() {
        let old = board(&[
            r#"(net 1 "Old")"#.into(),
            footprint("A1", r#"1 "Old""#, 1.0),
            footprint("A2", r#"1 "Old""#, 2.0),
            footprint("B1", r#""Moved""#, 3.0),
            footprint("B2", r#""Moved""#, 4.0),
            footprint("C1", r#""Kept""#, 5.0),
            footprint("C2", r#""Kept""#, 6.0),
            footprint("D1", r#""Duplicate""#, 7.0),
            footprint("D2", r#""Duplicate""#, 8.0),
            segment("1", "old-uuid"),
            segment(r#""Moved""#, "moved-uuid"),
            segment(r#""Kept""#, "unused-uuid"),
            segment(r#""Duplicate""#, "duplicate-uuid"),
        ]);
        let current = board(&[
            footprint("A1", r#""New""#, 1.0),
            footprint("A2", r#""New""#, 2.0),
            footprint("A3", r#""New""#, 2.5),
            footprint("B1", r#""Moved""#, 3.0),
            footprint("B2", r#""Moved""#, 4.1),
            footprint("C1", r#""Kept""#, 5.0),
            footprint("C2", r#""Kept""#, 6.0),
            footprint("D1", r#""Duplicate""#, 7.0),
            footprint("D2", r#""Duplicate""#, 8.0),
            footprint("D2", r#""""#, 9.0),
            segment(r#""Kept""#, "current-uuid"),
        ]);
        let temporary = Scratch::beside(&std::env::temp_dir().join("output")).unwrap();
        let previous = temporary.0.join("prior.kicad_pcb");
        fs::write(&previous, encode(&old)).unwrap();

        let (candidate, report) =
            prepare_route_reuse(&previous, &current, &KiCadBoardRouterConfig::default()).unwrap();
        let candidate = candidate.unwrap();
        assert_eq!(report.matched_nets, ["New"]);
        assert_eq!(report.extended_nets, ["New"]);
        assert_eq!(report.renamed_nets, ["New"]);
        assert_eq!(report.segments, 1);
        assert_eq!(report.skipped_nets["changed_pad_contacts"], 1);
        assert_eq!(report.skipped_nets["current_copper"], 1);
        assert_eq!(report.skipped_nets["ambiguous_current_terminals"], 1);
        let mut tracks = track_items(&candidate);
        assert_eq!(tracks.next(), track_items(&current).next());
        let added = tracks.next().unwrap();
        assert_eq!(form_atom(added, "net", 1), Some("New"));
        assert_ne!(form_atom(added, "uuid", 1), Some("old-uuid"));
        assert!(tracks.next().is_none());
        let returned = temporary.0.join("returned.kicad_pcb");
        fs::write(&returned, encode(&candidate)).unwrap();
        check_reused_copper(&candidate, &returned).unwrap();
        fs::write(&returned, encode(&current)).unwrap();
        assert!(check_reused_copper(&candidate, &returned).is_err());

        let wider = KiCadBoardRouterConfig {
            net_classes: vec![crate::KiCadNetClassRequest {
                name: "Power".into(),
                nets: vec!["New".into()],
                track_width_mm: Some(0.75),
                clearance_mm: None,
                via_diameter_mm: None,
                via_drill_mm: None,
            }],
            ..Default::default()
        };
        let (candidate, report) = prepare_route_reuse(&previous, &current, &wider).unwrap();
        assert!(candidate.is_none());
        assert_eq!(report.skipped_nets["incompatible_track_width"], 1);
    }

    #[test]
    fn two_established_groups_that_merge_never_supply_reused_copper() {
        let old = board(&[
            footprint("A1", r#""OldA""#, 1.0),
            footprint("A2", r#""OldA""#, 2.0),
            footprint("B1", r#""OldB""#, 3.0),
            footprint("B2", r#""OldB""#, 4.0),
        ]);
        let current = board(&[
            footprint("A1", r#""Merged""#, 1.0),
            footprint("A2", r#""Merged""#, 2.0),
            footprint("B1", r#""Merged""#, 3.0),
            footprint("B2", r#""Merged""#, 4.0),
        ]);
        let old = pad_groups(&old, &BTreeMap::new(), ContactSide::Native);
        let current = pad_groups(&current, &BTreeMap::new(), ContactSide::Native);
        for net in ["OldA", "OldB"] {
            assert_eq!(
                target_group(&old, &current, net).err(),
                Some("merged_prior_nets")
            );
        }
    }

    #[test]
    fn reordering_pad_layers_keeps_its_contact_and_changing_membership_does_not() {
        let parts = |layers: &str, size: &str| {
            let changed = footprint("R1", r#""Signal""#, 1.0)
                .replace(r#"(layers "F.Cu" "F.Mask")"#, layers)
                .replace("(size 1 2)", size);
            let pcb = board(&[changed, footprint("R2", r#""Signal""#, 2.0)]);
            pad_groups(&pcb, &BTreeMap::new(), ContactSide::Native)
        };
        let previous = parts(r#"(layers "B.Cu" "B.Paste" "B.Mask")"#, "(size 1 2)");
        let reordered = parts(r#"(layers "B.Cu" "B.Mask" "B.Paste")"#, "(size 1 2)");
        assert!(target_group(&previous, &reordered, "Signal").is_ok());
        for changed in [
            parts(r#"(layers "B.Cu" "B.Mask")"#, "(size 1 2)"),
            parts(
                r#"(layers "B.Cu" "B.Mask" "B.Paste" "In1.Cu")"#,
                "(size 1 2)",
            ),
            parts(r#"(layers "B.Cu" "B.Mask" "B.Paste")"#, "(size 1 2.1)"),
        ] {
            assert!(target_group(&previous, &changed, "Signal").is_err());
        }
    }

    #[test]
    fn repeated_pad_numbers_need_the_same_multiset_of_contacts_on_one_net() {
        let group = |copies: &[(&str, i32, &str)], other_end: &str| {
            let pads: Vec<_> = copies
                .iter()
                .map(|(number, x, net)| {
                    format!(
                        r#"(pad "{number}" smd rect (at {x} 0) (size 1 2) (layers "F.Cu") (net "{net}"))"#
                    )
                })
                .collect();
            let pcb = board(&[
                format!(
                    r#"(footprint "Test:Pad" (layer "F.Cu") (at 1 4)
                        (property "Reference" "A1") {})"#,
                    pads.join(" ")
                ),
                footprint("A2", &quote(other_end), 6.0),
            ]);
            pad_groups(&pcb, &BTreeMap::new(), ContactSide::Native)
        };
        let previous = group(&[("1", 0, "Old"), ("1", 2, "Old")], "Old");
        let matching = group(&[("1", 2, "Renamed"), ("1", 0, "Renamed")], "Renamed");
        assert_eq!(
            target_group(&previous, &matching, "Old").unwrap().0,
            "Renamed"
        );

        for (copies, reason) in [
            (
                vec![("1", 0, "New"), ("1", 3, "New")],
                "changed_pad_contacts",
            ),
            (vec![("1", 0, "New")], "changed_pad_contacts"),
            (
                vec![("1", 0, "New"), ("1", 2, "New"), ("1", 2, "New")],
                "changed_pad_contacts",
            ),
            (
                vec![("1", 0, "New"), ("1", 2, "Other")],
                "ambiguous_current_terminals",
            ),
            (
                vec![("1", 0, "New"), ("1", 2, "")],
                "ambiguous_current_terminals",
            ),
            (
                vec![("1", 0, "New"), ("1", 2, "New"), ("", 4, "New")],
                "ambiguous_current_terminals",
            ),
        ] {
            let current = group(&copies, "New");
            assert_eq!(
                target_group(&previous, &current, "Old").err(),
                Some(reason),
                "{copies:?}"
            );
        }
        let unassigned = group(&[("1", 0, "Old"), ("1", 2, "")], "Old");
        assert_eq!(
            target_group(&unassigned, &matching, "Old").err(),
            Some("ambiguous_or_insufficient_prior_terminals")
        );
    }

    #[test]
    fn saving_a_pad_in_kicad_preserves_the_contact_but_removing_layers_does_not() {
        let authored = footprint("A1", r#""Power""#, 1.0);
        let saved = authored.replace(
            r#"(layers "F.Cu" "F.Mask")"#,
            "(layers F.Cu F.Mask) (remove_unused_layers no)",
        );
        let removed = saved.replace("remove_unused_layers no", "remove_unused_layers yes");
        let shape = |footprint: &str| {
            let footprint = parse(footprint).unwrap();
            contact(&footprint, footprint.child("pad").unwrap()).unwrap()
        };
        assert!(shape(&authored) == shape(&saved));
        assert!(shape(&authored) != shape(&removed));
    }

    #[test]
    fn reuse_gate_counts_new_dangling_warnings_and_rejects_opens_or_missing_parity() {
        let temporary = Scratch::beside(&std::env::temp_dir().join("output")).unwrap();
        let before = temporary.0.join("before.json");
        let after = temporary.0.join("after.json");
        let existing = json!({
            "type": "track_dangling", "severity": "warning", "description": "Track has unconnected end",
            "items": [{"uuid": "test-track", "description": "Track", "pos": {"x": 1, "y": 1}}]
        });
        let mut original = json!({
            "included_severities": ["error", "warning", "exclusion"],
            "violations": [existing],
            "unconnected_items": [{"type": "unconnected_items"}],
            "schematic_parity": []
        });
        write_typed_json(&before, &original).unwrap();
        original["unconnected_items"] = json!([]);
        write_typed_json(&after, &original).unwrap();
        assert!(check_route_reuse(&before, &after).unwrap().is_empty());

        original["violations"][0]["items"][0]["pos"]["x"] = json!(2);
        write_typed_json(&after, &original).unwrap();
        assert_eq!(
            check_route_reuse(&before, &after).unwrap(),
            BTreeMap::from([("track_dangling".to_string(), 1)])
        );

        original["unconnected_items"] = json!([{"type": "unconnected_items"}]);
        write_typed_json(&after, &original).unwrap();
        assert!(
            check_route_reuse(&before, &after)
                .unwrap_err()
                .contains("open connections")
        );

        original["unconnected_items"] = json!([]);
        original.as_object_mut().unwrap().remove("schematic_parity");
        write_typed_json(&after, &original).unwrap();
        assert!(
            check_route_reuse(&before, &after)
                .unwrap_err()
                .contains("schematic_parity")
        );
    }
}
