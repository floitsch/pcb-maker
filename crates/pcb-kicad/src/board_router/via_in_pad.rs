// Copyright (C) 2026 Toit contributors.

//! Vias in the pads of a fine-pitch BGA (docs/router.md, "Vias in BGA pads
//! (2026-10-28)").
//!
//! No via goes into an SMD pad by default (`Board::no_via_pads`): an open
//! hole takes the pad's solder. A fine-pitch BGA's inner balls have no
//! other way out, though: at 0.5 mm pitch no track passes between two
//! balls, so a ball inside the outer ring is reached only through a via in
//! its pad, which the fab fills and caps (IPC-4761 type VII). Designers do
//! that where their fab can; the router does it where the designer did
//! (the board's own vias in a footprint's pads, the `--reference` board's)
//! or where the settings say so (`via_in_pad`), and only in the pads that
//! need it: an SMD pad of a routable net that no track at the neck width
//! leaves between the footprint's other pads on its layer (`enclosed`).
//!
//! The plan (`plan`) is a function of the board and the settings, so every
//! lowering of one board places the same vias: each a fixed via of the
//! pad's net at the pad's centre (the designer's vias in MIDAS's U106 sit
//! there), of the designer's size where it fits, else of the largest size
//! the project names (its predefined vias, the net's class) that keeps
//! every clearance to the copper around it; the lowering makes it copper
//! of the net on every layer and the pad's terminal reachable there, and
//! the board is written with it filled and capped.

use super::*;
use pcb_router as core;

/// `via_in_pad` in the router settings (`KiCadBoardRouterConfig`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum KiCadViaInPad {
    /// `true`: vias may go into every SMD pad, and the enclosed pads of
    /// every footprint get one; `false`: none, not even where the
    /// designer's board has them.
    All(bool),
    /// These parts or footprints' enclosed pads, besides the designer's.
    Parts(Vec<KiCadViaInPadEntry>),
}

/// A footprint whose enclosed pads hold a via (`via_in_pad`'s list, or the
/// designer's board's vias in its pads).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct KiCadViaInPadEntry {
    /// A reference (`U106`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// A footprint name, with or without its library
    /// (`TFBGA-153_11.5x13mm`, `Package_TFBGA:TFBGA-153_11.5x13mm`). With
    /// `part`, a footprint matching either is admitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footprint: Option<String>,
    /// The via's diameter and drill; default: the largest of the project's
    /// via sizes that fits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via_mm: Option<[f64; 2]>,
    /// Why the footprint is admitted (the designer's vias it is read from).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl<'de> Deserialize<'de> for KiCadViaInPadEntry {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Object {
            #[serde(default)]
            part: Option<String>,
            #[serde(default)]
            footprint: Option<String>,
            #[serde(default)]
            via_mm: Option<[f64; 2]>,
            #[serde(default)]
            source: Option<String>,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Name(String),
            Object(Object),
        }
        let entry = match Raw::deserialize(deserializer).map_err(|_| {
            serde::de::Error::custom(
                "a via_in_pad entry is a reference or footprint name (\"U106\") or {\"part\": ..., \"footprint\": ..., \"via_mm\": [diameter, drill]}",
            )
        })? {
            // A bare name is a reference or a footprint name.
            Raw::Name(name) => Self {
                part: Some(name.clone()),
                footprint: Some(name),
                ..Self::default()
            },
            Raw::Object(object) => Self {
                part: object.part,
                footprint: object.footprint,
                via_mm: object.via_mm,
                source: object.source,
            },
        };
        if entry.part.is_none() && entry.footprint.is_none() {
            return Err(serde::de::Error::custom(
                "a via_in_pad entry names a part or a footprint",
            ));
        }
        if let Some([diameter, drill]) = entry.via_mm
            && !(drill > 0.0 && diameter > drill)
        {
            return Err(serde::de::Error::custom(format!(
                "via_in_pad's via_mm [{diameter}, {drill}]: a diameter larger than a positive drill"
            )));
        }
        Ok(entry)
    }
}

impl KiCadViaInPadEntry {
    /// Whether the footprint `footprint` (its library name) of part
    /// `reference` is this entry's.
    pub(crate) fn matches(&self, reference: &str, footprint: &str) -> bool {
        let name = footprint
            .rsplit_once(':')
            .map_or(footprint, |(_, name)| name);
        self.part.as_deref() == Some(reference)
            || self
                .footprint
                .as_deref()
                .is_some_and(|wanted| wanted == footprint || wanted == name)
    }
}

/// The footprints whose small SMD pads the board's own vias lie in
/// (score-kicad-board's vias in pads, filled or not), one entry per part
/// with its footprint, its vias' most common size and `source` naming the
/// pads: the designer's sign that the fab fills and caps such vias.
pub(crate) fn designer_entries(pcb: &Expr, board: &str) -> Vec<KiCadViaInPadEntry> {
    let Ok(sites) = crate::quality::vias_in_small_smd_pads(pcb) else {
        return Vec::new();
    };
    let mut by_part: BTreeMap<(String, String), Vec<&crate::quality::KiCadViaInPadSite>> =
        BTreeMap::new();
    for site in &sites {
        by_part
            .entry((site.part.clone(), site.footprint.clone()))
            .or_default()
            .push(site);
    }
    by_part
        .into_iter()
        .map(|((part, footprint), sites)| {
            // The most common size, the smaller first on a tie.
            let mut sizes: BTreeMap<(u64, u64), usize> = BTreeMap::new();
            for site in &sites {
                let key = (
                    (site.diameter_mm * 1.0e6).round() as u64,
                    (site.drill_mm * 1.0e6).round() as u64,
                );
                *sizes.entry(key).or_default() += 1;
            }
            let ((diameter, drill), _) = sizes
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
                .map(|(key, count)| (*key, *count))
                .unwrap_or_default();
            let mut pads: Vec<&str> = sites.iter().map(|site| site.pad.as_str()).collect();
            pads.sort_unstable();
            pads.dedup();
            KiCadViaInPadEntry {
                part: Some(part.clone()),
                footprint: Some(footprint),
                via_mm: (drill > 0 && diameter > drill)
                    .then(|| [diameter as f64 / 1.0e6, drill as f64 / 1.0e6]),
                source: Some(format!(
                    "{board} has {} via{} in {part}'s pads {}",
                    sites.len(),
                    if sites.len() == 1 { "" } else { "s" },
                    pads.join(", ")
                )),
            }
        })
        .collect()
}

/// What the via-in-pad rule did on a board (`vias_in_pads` in
/// board-router.json).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KiCadViaInPadReport {
    /// Vias the rule put in pads (written filled and capped).
    pub placed: usize,
    /// The footprints admitted that have enclosed pads: the rule, the via
    /// and the pads.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub footprints: Vec<KiCadViaInPadFootprint>,
    /// Footprints with enclosed pads that the rule does not admit: their
    /// pads (with their nets) have no way out but a via in the pad.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub closed: Vec<KiCadClosedPads>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KiCadViaInPadFootprint {
    pub part: String,
    pub footprint: String,
    /// Why vias may go there: `via_in_pad: true`, `via_in_pad: ["U106"]`,
    /// or the designer's vias (`KiCadViaInPadEntry::source`).
    pub rule: String,
    /// The vias' diameter and drill, when any was placed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via_mm: Option<[f64; 2]>,
    /// The pads that got a via (`B2 (/MEM_DAT3)`).
    pub pads: Vec<String>,
    /// Enclosed pads that already hold a via (the board's own, kept).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub kept: Vec<String>,
    /// Why the enclosed pads got none, when no size fits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KiCadClosedPads {
    pub part: String,
    pub footprint: String,
    /// `B2 (/MEM_DAT3)`.
    pub pads: Vec<String>,
    /// The pads' nets (as the router names them).
    pub nets: Vec<String>,
}

/// A via the rule puts in a pad.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlannedVia {
    /// The pad's label (`U106.B2`), as the lowering names it.
    pub label: String,
    pub at: [f64; 2],
    pub diameter: f64,
    pub drill: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ViaInPadPlan {
    pub vias: Vec<PlannedVia>,
    pub report: KiCadViaInPadReport,
}

/// A pad of the board as the plan sees it.
struct BoardPad {
    footprint: usize,
    number: String,
    center: [f64; 2],
    shape: core::Shape,
    layers: core::LayerMask,
    smd: bool,
    /// The routable net (as the router names it).
    net: Option<String>,
    /// The clearance other copper keeps from it: its override, else its
    /// net class's.
    clearance: f64,
    /// A plated or unplated hole's centre and radius.
    hole: Option<([f64; 2], f64)>,
}

/// The board's vias: centre, radius, drill radius.
fn board_vias(pcb: &Expr) -> Vec<([f64; 2], f64, f64)> {
    pcb.children()
        .iter()
        .filter(|item| item.head() == Some("via"))
        .filter_map(|item| {
            let at = form_at(item).ok()?;
            Some((
                [at[0], at[1]],
                form_f64(item, "size", 1).ok()? / 2.0,
                form_f64(item, "drill", 1).unwrap_or(0.0) / 2.0,
            ))
        })
        .collect()
}

/// The vias the rule puts in pads on `pcb` with `config`, and the report.
pub(crate) fn plan(pcb: &Expr, config: &KiCadBoardRouterConfig) -> ViaInPadPlan {
    plan_of(pcb, config).unwrap_or_default()
}

/// A net's routing rules (its class's, else the default's).
fn rules_of<'a>(
    config: &'a KiCadBoardRouterConfig,
    name: Option<&str>,
) -> Option<&'a KiCadConnectionRoutingRules> {
    name.and_then(|name| config.connection_rules.get(name))
        .or(config.default_rules.as_ref())
}

fn plan_of(pcb: &Expr, config: &KiCadBoardRouterConfig) -> Result<ViaInPadPlan, String> {
    let names = NetNames::of(pcb);
    let layers = LayerTable::from_pcb(pcb)?;
    let rules_of = |name: Option<&str>| rules_of(config, name);
    let mut footprints: Vec<(String, String)> = Vec::new();
    let mut pads: Vec<BoardPad> = Vec::new();
    for item in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let footprint = footprints.len();
        footprints.push((
            footprint_reference(item).unwrap_or_default(),
            item.children()
                .get(1)
                .and_then(Expr::atom)
                .unwrap_or("")
                .trim_matches('"')
                .to_string(),
        ));
        let at = form_at(item)?;
        for pad in item
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            let pad_type = pad.children().get(2).and_then(Expr::atom).unwrap_or("");
            let pad_layers = layers.pad_mask(pad)?;
            let lowered = lower_pad(pad, at)?;
            let raw = node_net(pad);
            let net = raw
                .filter(|raw| names.routable(raw))
                .map(|raw| names.normalize(raw).to_string());
            let class = rules_of(raw.map(|raw| names.normalize(raw)));
            let clearance = Some(local_clearance::pad_clearance(pad, item)?)
                .filter(|value| *value > 0.0)
                .or(class.map(|rules| rules.clearance_mm))
                .unwrap_or(0.0);
            let hole = if matches!(pad_type, "thru_hole" | "np_thru_hole") {
                form_f64(pad.child("drill").unwrap_or(pad), "drill", 1)
                    .ok()
                    .map(|drill| (lowered.center, drill / 2.0))
            } else {
                None
            };
            if pad_layers == 0 && hole.is_none() {
                continue;
            }
            pads.push(BoardPad {
                footprint,
                number: pad
                    .children()
                    .get(1)
                    .and_then(Expr::atom)
                    .unwrap_or("")
                    .trim_matches('"')
                    .to_string(),
                center: lowered.center,
                shape: shape(&lowered.geometry),
                layers: if pad_type == "np_thru_hole" {
                    0
                } else {
                    pad_layers
                },
                smd: matches!(pad_type, "smd" | "connect"),
                net,
                clearance,
                hole,
            });
        }
    }
    let vias = board_vias(pcb);
    // A net of one pad needs no way out.
    let mut pad_counts: BTreeMap<String, usize> = BTreeMap::new();
    for pad in &pads {
        if let Some(net) = &pad.net {
            *pad_counts.entry(net.clone()).or_default() += 1;
        }
    }
    for pad in &mut pads {
        if pad.net.as_deref().is_some_and(|net| pad_counts[net] < 2) {
            pad.net = None;
        }
    }
    let designer: Vec<KiCadViaInPadEntry> = config
        .project_rules
        .as_ref()
        .map(|report| report.via_in_pad_designer.clone())
        .unwrap_or_default();
    let listed: &[KiCadViaInPadEntry] = match &config.via_in_pad {
        Some(KiCadViaInPad::Parts(entries)) => entries,
        _ => &[],
    };
    let neck = config.neck_width_mm.unwrap_or(f64::INFINITY);
    let mut plan = ViaInPadPlan::default();
    for (footprint, (reference, name)) in footprints.iter().enumerate() {
        let own: Vec<usize> = (0..pads.len())
            .filter(|index| pads[*index].footprint == footprint)
            .collect();
        let enclosed = enclosed_pads(&pads, &own, &|net: &str| {
            rules_of(Some(net)).map_or((neck.min(0.2), 0.0, 0.6), |rules| {
                (
                    neck.min(rules.trace_width_mm),
                    rules.clearance_mm,
                    rules.via_size_mm,
                )
            })
        });
        if enclosed.is_empty() {
            continue;
        }
        let pad_text = |index: usize| {
            format!(
                "{} ({})",
                pads[index].number,
                pads[index].net.as_deref().unwrap_or("")
            )
        };
        let admitted: Option<(String, Option<[f64; 2]>)> = match &config.via_in_pad {
            Some(KiCadViaInPad::All(true)) => Some(("via_in_pad: true".into(), None)),
            Some(KiCadViaInPad::All(false)) => None,
            _ => listed
                .iter()
                .find(|entry| entry.matches(reference, name))
                .map(|entry| {
                    (
                        entry.source.clone().unwrap_or_else(|| {
                            format!(
                                "via_in_pad: [{:?}]",
                                entry
                                    .part
                                    .as_deref()
                                    .or(entry.footprint.as_deref())
                                    .unwrap_or("")
                            )
                        }),
                        entry.via_mm,
                    )
                })
                .or_else(|| {
                    designer
                        .iter()
                        .find(|entry| entry.matches(reference, name))
                        .map(|entry| (entry.source.clone().unwrap_or_default(), entry.via_mm))
                }),
        };
        let Some((rule, wanted)) = admitted else {
            plan.report.closed.push(KiCadClosedPads {
                part: reference.clone(),
                footprint: name.clone(),
                pads: enclosed.iter().map(|&index| pad_text(index)).collect(),
                nets: {
                    let mut nets: Vec<String> = enclosed
                        .iter()
                        .filter_map(|&index| pads[index].net.clone())
                        .collect();
                    nets.sort();
                    nets.dedup();
                    nets
                },
            });
            continue;
        };
        // A pad that holds a via already (the board's own, kept in route
        // mode, or this rule's on a board read back) keeps it.
        let (kept, open): (Vec<usize>, Vec<usize>) = enclosed.iter().partition(|&&index| {
            vias.iter()
                .any(|(at, _, _)| pads[index].shape.distance_to_point(*at) <= 0.0)
        });
        let mut entry = KiCadViaInPadFootprint {
            part: reference.clone(),
            footprint: name.clone(),
            rule,
            kept: kept.iter().map(|&index| pad_text(index)).collect(),
            ..KiCadViaInPadFootprint::default()
        };
        if !open.is_empty() {
            // Each pad the first size that fits beside the vias placed
            // before it.
            let sizes = size_candidates(config, wanted, &pads, &open);
            let mut placed: Vec<([f64; 2], Option<String>, f64, f64)> = Vec::new();
            let mut refused = Vec::new();
            for &index in &open {
                match sizes
                    .iter()
                    .find(|size| fits(**size, &pads, index, &vias, &placed, config))
                {
                    Some(&[diameter, drill]) => {
                        placed.push((pads[index].center, pads[index].net.clone(), diameter, drill));
                        entry.via_mm.get_or_insert([diameter, drill]);
                        entry.pads.push(pad_text(index));
                        plan.vias.push(PlannedVia {
                            label: format!("{reference}.{}", pads[index].number),
                            at: pads[index].center,
                            diameter,
                            drill,
                        });
                    }
                    None => refused.push(pad_text(index)),
                }
            }
            if !refused.is_empty() {
                entry.refused = Some(format!(
                    "no via size keeps its clearances in {} (tried {})",
                    refused.join(", "),
                    sizes
                        .iter()
                        .map(|[d, drill]| format!("{d}/{drill} mm"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        plan.report.footprints.push(entry);
    }
    plan.report.placed = plan.vias.len();
    Ok(plan)
}

/// The via sizes to try, best first: the wanted one (the designer's), then
/// the project's predefined sizes and the pads' nets' class vias, the
/// largest first, each within the board's via minimums.
fn size_candidates(
    config: &KiCadBoardRouterConfig,
    wanted: Option<[f64; 2]>,
    pads: &[BoardPad],
    open: &[usize],
) -> Vec<[f64; 2]> {
    let (predefined, minimums) = config
        .project_rules
        .as_ref()
        .map_or((Vec::new(), [0.0; 3]), |report| {
            (report.via_sizes.clone(), report.via_minimums)
        });
    let mut sizes: Vec<[f64; 2]> = predefined;
    for &index in open {
        if let Some(rules) = rules_of(config, pads[index].net.as_deref()) {
            sizes.push([rules.via_size_mm, rules.via_drill_mm]);
        }
    }
    let legal = |[diameter, drill]: [f64; 2]| {
        drill > 0.0
            && diameter > drill
            && diameter + 1.0e-9 >= minimums[0]
            && drill + 1.0e-9 >= minimums[1]
            && (diameter - drill) / 2.0 + 1.0e-9 >= minimums[2]
    };
    sizes.retain(|size| legal(*size));
    sizes.sort_by(|a, b| b[0].total_cmp(&a[0]).then(b[1].total_cmp(&a[1])));
    sizes.dedup_by(|a, b| (a[0] - b[0]).abs() < 1.0e-9 && (a[1] - b[1]).abs() < 1.0e-9);
    if let Some(size) = wanted.filter(|size| legal(*size)) {
        sizes.insert(0, size);
    }
    sizes
}

/// Whether a via of `size` at the centre of pad `index` keeps every rule:
/// the drill inside the pad's copper; its copper the larger clearance of
/// both from every other net's pad on any layer (a through via meets the
/// back's parts too), from the board's other vias and from the vias placed
/// before it (`placed`: centre, net, diameter, drill); its drill the hole
/// clearance from that copper and the hole-to-hole spacing from every hole.
fn fits(
    [diameter, drill]: [f64; 2],
    pads: &[BoardPad],
    index: usize,
    vias: &[([f64; 2], f64, f64)],
    placed: &[([f64; 2], Option<String>, f64, f64)],
    config: &KiCadBoardRouterConfig,
) -> bool {
    const SLACK: f64 = 1.0e-6;
    let radius = diameter / 2.0;
    let hole = drill / 2.0;
    let pad = &pads[index];
    let center = pad.center;
    if !pad.shape.contains_disc(center, hole) {
        return false;
    }
    // Far enough that no rule reaches.
    let probe = core::geometry::Aabb {
        minimum: center,
        maximum: center,
    }
    .inflated(radius + 2.0);
    let pads_clear = pads.iter().enumerate().all(|(other_index, other)| {
        if other_index == index || !other.shape.aabb().intersects(probe) {
            return true;
        }
        if let Some((at, other_hole)) = other.hole
            && distance(at, center) + SLACK < hole + other_hole + config.hole_to_hole_clearance_mm
        {
            return false;
        }
        if other.layers == 0 || (other.net.is_some() && other.net == pad.net) {
            return true;
        }
        let gap = other.shape.distance_to_point(center);
        gap + SLACK >= radius + pad.clearance.max(other.clearance)
            && gap + SLACK >= hole + config.hole_clearance_mm
    });
    let vias_clear = vias.iter().all(|(at, other_radius, other_hole)| {
        let apart = distance(*at, center);
        apart + SLACK >= hole + other_hole + config.hole_to_hole_clearance_mm
            && apart + SLACK >= radius + other_radius + pad.clearance
    });
    let placed_clear = placed.iter().all(|(at, net, other_diameter, other_drill)| {
        let apart = distance(*at, center);
        apart + SLACK >= (drill + other_drill) / 2.0 + config.hole_to_hole_clearance_mm
            && (*net == pad.net
                || apart + SLACK >= (diameter + other_diameter) / 2.0 + pad.clearance)
    });
    pads_clear && vias_clear && placed_clear
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// The SMD pads of a footprint (`own`, indices into `pads`) with a routable
/// net that no track leaves on their layer: every path from the pad out of
/// the footprint's pad field, or to a place inside it where the net's own
/// via fits (a BGA's empty middle), passes closer to another net's pad
/// than the net's neck width's half plus the larger clearance. Only a pad
/// whose centre lies strictly inside the box of the footprint's other
/// pads' centres on its layer can be (a pad of an outer row has the open
/// side), and an exposed pad never is. `class(net)` gives a net's
/// narrowest track, its clearance and its via's diameter.
fn enclosed_pads(
    pads: &[BoardPad],
    own: &[usize],
    class: &dyn Fn(&str) -> (f64, f64, f64),
) -> Vec<usize> {
    let boxes: Vec<PadBox> = own
        .iter()
        .map(|&index| {
            let bounds = pads[index].shape.aabb();
            PadBox {
                number: pads[index].number.clone(),
                center: pads[index].center,
                smd: pads[index].smd,
                bounds: [bounds.minimum, bounds.maximum],
            }
        })
        .collect();
    let exposed: Vec<usize> = exposed_pad(&boxes)
        .into_iter()
        .map(|position| own[position])
        .collect();
    let mut enclosed = Vec::new();
    // Per layer of the footprint's SMD pads.
    let mut layer_masks: Vec<core::LayerMask> = own
        .iter()
        .filter(|&&index| pads[index].smd)
        .map(|&index| pads[index].layers)
        .filter(|layers| layers.count_ones() == 1)
        .collect();
    layer_masks.sort_unstable();
    layer_masks.dedup();
    for layer in layer_masks {
        let on_layer: Vec<usize> = own
            .iter()
            .copied()
            .filter(|&index| pads[index].layers & layer != 0)
            .collect();
        if on_layer.len() < 9 {
            continue;
        }
        let inside_box = |index: usize| {
            let others = on_layer.iter().filter(|&&other| other != index);
            let mut bounds = [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ];
            for &other in others {
                let center = pads[other].center;
                bounds = [
                    bounds[0].min(center[0]),
                    bounds[1].min(center[1]),
                    bounds[2].max(center[0]),
                    bounds[3].max(center[1]),
                ];
            }
            const MARGIN: f64 = 1.0e-3;
            let center = pads[index].center;
            center[0] > bounds[0] + MARGIN
                && center[0] < bounds[2] - MARGIN
                && center[1] > bounds[1] + MARGIN
                && center[1] < bounds[3] - MARGIN
        };
        let candidates: Vec<usize> = on_layer
            .iter()
            .copied()
            .filter(|&index| {
                pads[index].smd
                    && pads[index].layers == layer
                    && pads[index].net.is_some()
                    && !exposed.contains(&index)
                    && inside_box(index)
            })
            .collect();
        if candidates.is_empty() {
            continue;
        }
        // The narrowest track, the smallest clearance and via of the
        // candidates' nets: the blocked cells are where no candidate's
        // track passes, the via cells where its via fits.
        let (width, clearance, via) = candidates.iter().fold(
            (f64::INFINITY, f64::INFINITY, f64::INFINITY),
            |(width, clearance, via), &index| {
                let (w, c, v) = class(pads[index].net.as_deref().unwrap_or(""));
                (width.min(w), clearance.min(c), via.min(v))
            },
        );
        let grid = FloodGrid::new(pads, &on_layer, width, clearance, via);
        let mut by_net: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for &index in &candidates {
            by_net
                .entry(pads[index].net.as_deref().unwrap_or(""))
                .or_default()
                .push(index);
        }
        for (net, members) in by_net {
            let reached = grid.flood(pads, net);
            for index in members {
                if !grid.reaches(&reached, &pads[index].shape) {
                    enclosed.push(index);
                }
            }
        }
    }
    enclosed.sort_unstable();
    enclosed
}

/// A raster of a footprint's pad field on one layer for `enclosed_pads`:
/// per cell the nets of the pads it lies too close to for a track.
struct FloodGrid {
    origin: [f64; 2],
    step: f64,
    size: [usize; 2],
    /// Per cell: `None` free, `Some(Some(pad))` blocked by pads of one net
    /// only (that pad's), `Some(None)` blocked by pads of several nets or
    /// of none.
    blocked: Vec<Option<Option<usize>>>,
    /// Per cell: whether a via fits there (its clearance from every pad).
    via_fits: Vec<bool>,
}

impl FloodGrid {
    fn new(pads: &[BoardPad], on_layer: &[usize], width: f64, clearance: f64, via: f64) -> Self {
        // A twentieth of the pitch, between 5 and 25 um.
        let mut pitch = f64::INFINITY;
        for (position, &a) in on_layer.iter().enumerate() {
            for &b in &on_layer[position + 1..] {
                pitch = pitch.min(distance(pads[a].center, pads[b].center));
            }
        }
        let step = (pitch / 20.0).clamp(0.005, 0.025);
        let reach_of = |index: usize| width / 2.0 + clearance.max(pads[index].clearance);
        let via_reach_of = |index: usize| via / 2.0 + clearance.max(pads[index].clearance);
        let margin = on_layer
            .iter()
            .map(|&index| reach_of(index).max(via_reach_of(index)))
            .fold(0.0, f64::max)
            * 2.0
            + 2.0 * step;
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for &index in on_layer {
            let aabb = pads[index].shape.aabb();
            bounds = [
                bounds[0].min(aabb.minimum[0]),
                bounds[1].min(aabb.minimum[1]),
                bounds[2].max(aabb.maximum[0]),
                bounds[3].max(aabb.maximum[1]),
            ];
        }
        let origin = [bounds[0] - margin, bounds[1] - margin];
        let size = [
            ((bounds[2] + margin - origin[0]) / step).ceil() as usize + 1,
            ((bounds[3] + margin - origin[1]) / step).ceil() as usize + 1,
        ];
        let mut blocked: Vec<Option<Option<usize>>> = vec![None; size[0] * size[1]];
        let mut via_fits = vec![true; size[0] * size[1]];
        for &index in on_layer {
            let pad = &pads[index];
            let reach = reach_of(index);
            let via_reach = via_reach_of(index);
            let aabb = pad.shape.aabb().inflated(reach.max(via_reach));
            let x0 = ((aabb.minimum[0] - origin[0]) / step).floor().max(0.0) as usize;
            let y0 = ((aabb.minimum[1] - origin[1]) / step).floor().max(0.0) as usize;
            let x1 = (((aabb.maximum[0] - origin[0]) / step).ceil() as usize).min(size[0] - 1);
            let y1 = (((aabb.maximum[1] - origin[1]) / step).ceil() as usize).min(size[1] - 1);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let point = [origin[0] + x as f64 * step, origin[1] + y as f64 * step];
                    // A cell's width short of the reach is free: a gap
                    // that fits a track exactly (Glasgow's 0.15 mm tracks
                    // at 0.1499 mm between 0.8 mm balls of 0.35 mm) has
                    // no cell centre in it otherwise. The test errs
                    // towards a way out (no via) by up to a cell.
                    let gap = pad.shape.distance_to_point(point) + step;
                    if gap < via_reach {
                        via_fits[y * size[0] + x] = false;
                    }
                    if gap >= reach {
                        continue;
                    }
                    let cell = &mut blocked[y * size[0] + x];
                    *cell = match (*cell, &pad.net) {
                        (None, Some(_)) => Some(Some(index)),
                        (Some(Some(other)), Some(net))
                            if pads[other].net.as_deref() == Some(net.as_str()) =>
                        {
                            Some(Some(other))
                        }
                        _ => Some(None),
                    };
                }
            }
        }
        Self {
            origin,
            step,
            size,
            blocked,
            via_fits,
        }
    }

    /// The cells a track of `net` reaches from outside the pad field or
    /// from a cell where a via fits (eight neighbours).
    fn flood(&self, pads: &[BoardPad], net: &str) -> Vec<bool> {
        let [width, height] = self.size;
        let free = |cell: usize| match self.blocked[cell] {
            None => true,
            Some(Some(pad)) => pads[pad].net.as_deref() == Some(net),
            Some(None) => false,
        };
        let mut reached = vec![false; width * height];
        let mut queue = std::collections::VecDeque::new();
        for x in 0..width {
            for y in [0, height - 1] {
                queue.push_back(y * width + x);
            }
        }
        for y in 0..height {
            for x in [0, width - 1] {
                queue.push_back(y * width + x);
            }
        }
        queue.extend((0..width * height).filter(|cell| self.via_fits[*cell]));
        while let Some(cell) = queue.pop_front() {
            if reached[cell] || !free(cell) {
                continue;
            }
            reached[cell] = true;
            let (x, y) = ((cell % width) as isize, (cell / width) as isize);
            for (dx, dy) in [
                (1, 0),
                (-1, 0),
                (0, 1),
                (0, -1),
                (1, 1),
                (1, -1),
                (-1, 1),
                (-1, -1),
            ] {
                let (nx, ny) = (x + dx, y + dy);
                if nx >= 0 && ny >= 0 && (nx as usize) < width && (ny as usize) < height {
                    let next = ny as usize * width + nx as usize;
                    if !reached[next] {
                        queue.push_back(next);
                    }
                }
            }
        }
        reached
    }

    /// Whether a reached cell lies in `shape`.
    fn reaches(&self, reached: &[bool], shape: &core::Shape) -> bool {
        let aabb = shape.aabb();
        let x0 = ((aabb.minimum[0] - self.origin[0]) / self.step)
            .floor()
            .max(0.0) as usize;
        let y0 = ((aabb.minimum[1] - self.origin[1]) / self.step)
            .floor()
            .max(0.0) as usize;
        let x1 = (((aabb.maximum[0] - self.origin[0]) / self.step).ceil() as usize)
            .min(self.size[0] - 1);
        let y1 = (((aabb.maximum[1] - self.origin[1]) / self.step).ceil() as usize)
            .min(self.size[1] - 1);
        (y0..=y1).any(|y| {
            (x0..=x1).any(|x| {
                let point = [
                    self.origin[0] + x as f64 * self.step,
                    self.origin[1] + y as f64 * self.step,
                ];
                reached[y * self.size[0] + x] && shape.distance_to_point(point) <= 0.0
            })
        })
    }
}

/// The vias the lowering put in pads (`pad_vias` in the lowering: the
/// obstacles it labels `<pad> via in pad hole`, `... ring` and `... via in
/// pad`): per via its net, centre, diameter and drill, in the board's
/// order.
pub(crate) fn pad_vias_of(board: &core::Board) -> Vec<(core::NetId, [f64; 2], f64, f64)> {
    let mut diameters: BTreeMap<&str, f64> = BTreeMap::new();
    for obstacle in &board.obstacles {
        if obstacle.kind != core::ObstacleKind::Copper {
            continue;
        }
        let pad = obstacle
            .label
            .strip_suffix(" via in pad ring")
            .or_else(|| obstacle.label.strip_suffix(" via in pad"));
        if let (Some(pad), core::Shape::Circle { radius, .. }) = (pad, &obstacle.shape) {
            diameters.insert(pad, 2.0 * radius);
        }
    }
    board
        .obstacles
        .iter()
        .filter(|obstacle| obstacle.kind == core::ObstacleKind::Hole)
        .filter_map(|obstacle| {
            let pad = obstacle.label.strip_suffix(" via in pad hole")?;
            let core::Shape::Circle { center, radius } = &obstacle.shape else {
                return None;
            };
            Some((obstacle.net?, *center, *diameters.get(pad)?, 2.0 * radius))
        })
        .collect()
}

/// Adds the `--reference` board's vias in small SMD pads to the designer's
/// via-in-pad entries (`KiCadProjectRulesReport::via_in_pad_designer`): the
/// designer's board as the reference says what its fab does. A reference
/// that cannot be read adds nothing (the commands check it elsewhere).
pub fn admit_reference_vias_in_pads(
    config: &mut KiCadBoardRouterConfig,
    reference: &Path,
    board_id: &str,
) {
    let path = reference.join(format!("{board_id}.kicad_pcb"));
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    let Ok(pcb) = parse(&text) else {
        return;
    };
    let entries = designer_entries(&pcb, "the reference board");
    if entries.is_empty() {
        return;
    }
    config
        .project_rules
        .get_or_insert_with(Default::default)
        .via_in_pad_designer
        .extend(entries);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 5 x 5 ball grid at 0.5 mm pitch (0.3 mm balls) at `at`: the
    /// centre ball C3 on `X`, B2 on `Y`, A3 (outer row) on `Z`, the others
    /// unconnected.
    fn bga(name: &str, reference: &str, at: [f64; 2]) -> String {
        let mut pads = String::new();
        for (row, letter) in ["A", "B", "C", "D", "E"].iter().enumerate() {
            for column in 0..5 {
                let number = format!("{letter}{}", column + 1);
                let net = match number.as_str() {
                    "C3" => "X".to_string(),
                    "B2" => "Y".to_string(),
                    "A3" => "Z".to_string(),
                    other => format!("unconnected-({reference}-{other})"),
                };
                pads.push_str(&format!(
                    r#"(pad "{number}" smd circle (at {} {}) (size 0.3 0.3) (layers "F.Cu" "F.Mask") (net "{net}"))"#,
                    (column as f64 - 2.0) * 0.5,
                    (row as f64 - 2.0) * 0.5
                ));
            }
        }
        format!(
            r#"(footprint "{name}" (at {} {}) (property "Reference" "{reference}") {pads})"#,
            at[0], at[1]
        )
    }

    /// U1 and U2 (footprint BGA-25) and U3 (BGA-25B), each with its nets'
    /// other ends on R pads, and `extra` (the designer's vias).
    fn board(extra: &str) -> Expr {
        let ends = |reference: &str, at: f64| {
            format!(
                r#"(footprint "R" (at {at} 2) (property "Reference" "{reference}")
                     (pad "1" smd rect (at 0 0) (size 0.6 0.6) (layers "F.Cu") (net "X"))
                     (pad "2" smd rect (at 1 0) (size 0.6 0.6) (layers "F.Cu") (net "Y"))
                     (pad "3" smd rect (at 2 0) (size 0.6 0.6) (layers "F.Cu") (net "Z")))"#
            )
        };
        parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
                 (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
                 {} {} {} {} {extra})"#,
            bga("Package_BGA:BGA-25", "U1", [5.0, 10.0]),
            bga("Package_BGA:BGA-25", "U2", [15.0, 10.0]),
            bga("Package_BGA:BGA-25B", "U3", [25.0, 10.0]),
            ends("R1", 5.0)
        ))
        .unwrap()
    }

    fn config(via_in_pad: Option<KiCadViaInPad>, pcb: &Expr) -> KiCadBoardRouterConfig {
        KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.1,
                clearance_mm: 0.1,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            neck_width_mm: Some(0.1),
            hole_clearance_mm: 0.1,
            hole_to_hole_clearance_mm: 0.2,
            via_in_pad,
            project_rules: Some(crate::project_rules::KiCadProjectRulesReport {
                via_sizes: vec![[0.35, 0.2], [0.8, 0.4]],
                via_minimums: [0.25, 0.15, 0.05],
                via_in_pad_designer: designer_entries(pcb, "the board"),
                ..Default::default()
            }),
            ..KiCadBoardRouterConfig::default()
        }
    }

    fn planned(plan: &ViaInPadPlan) -> Vec<&str> {
        plan.vias.iter().map(|via| via.label.as_str()).collect()
    }

    #[test]
    fn a_designer_via_in_a_pad_of_a_footprint_admits_its_enclosed_pads_and_no_other_footprint() {
        // The designer's via in U1's C3 (centred, 0.35/0.2).
        let pcb =
            board(r#"(via (at 5 10) (size 0.35) (drill 0.2) (layers "F.Cu" "B.Cu") (net "X"))"#);
        let designer = designer_entries(&pcb, "the board");
        assert_eq!(designer.len(), 1);
        assert_eq!(designer[0].part.as_deref(), Some("U1"));
        assert_eq!(designer[0].via_mm, Some([0.35, 0.2]));
        let plan = plan(&pcb, &config(None, &pcb));
        // U1: B2 gets the via, C3 keeps the designer's; U2 (the same
        // footprint) both; U3 (another footprint) none. A3 is on the outer
        // row: its track leaves outwards.
        assert_eq!(planned(&plan), ["U1.B2", "U2.B2", "U2.C3"]);
        assert!(
            plan.vias
                .iter()
                .all(|via| (via.diameter, via.drill) == (0.35, 0.2))
        );
        assert_eq!(plan.vias[0].at, [4.5, 9.5]);
        assert_eq!(plan.report.placed, 3);
        assert_eq!(plan.report.footprints[0].kept, ["C3 (X)"]);
        assert_eq!(plan.report.closed.len(), 1);
        assert_eq!(plan.report.closed[0].part, "U3");
        assert_eq!(plan.report.closed[0].pads, ["B2 (Y)", "C3 (X)"]);
        assert_eq!(plan.report.closed[0].nets, ["X", "Y"]);
        // `false` admits none, the designer's neither.
        let none = super::plan(&pcb, &config(Some(KiCadViaInPad::All(false)), &pcb));
        assert!(none.vias.is_empty());
        assert_eq!(none.report.closed.len(), 3);
    }

    #[test]
    fn the_opt_in_admits_a_part_or_a_footprint_by_name() {
        let pcb = board("");
        // No designer's via: every BGA closed.
        let closed = plan(&pcb, &config(None, &pcb));
        assert!(closed.vias.is_empty());
        assert_eq!(closed.report.closed.len(), 3);
        let entries: Vec<KiCadViaInPadEntry> =
            serde_json::from_str(r#"["U3", {"footprint": "BGA-25", "via_mm": [0.3, 0.15]}]"#)
                .unwrap();
        let opted = plan(&pcb, &config(Some(KiCadViaInPad::Parts(entries)), &pcb));
        assert_eq!(
            planned(&opted),
            ["U1.B2", "U1.C3", "U2.B2", "U2.C3", "U3.B2", "U3.C3"]
        );
        // The entry's size where it fits, else the project's largest that
        // fits (0.8 does not).
        assert_eq!((opted.vias[0].diameter, opted.vias[0].drill), (0.3, 0.15));
        assert_eq!((opted.vias[4].diameter, opted.vias[4].drill), (0.35, 0.2));
        assert!(opted.report.closed.is_empty());
        assert_eq!(opted.report.footprints[2].rule, "via_in_pad: [\"U3\"]");
    }

    #[test]
    fn via_in_pad_reads_a_switch_or_a_list() {
        let read = |text: &str| serde_json::from_str::<KiCadViaInPad>(text);
        assert_eq!(read("true").unwrap(), KiCadViaInPad::All(true));
        let KiCadViaInPad::Parts(entries) = read(r#"["U106", {"part": "U7"}]"#).unwrap() else {
            panic!("a list");
        };
        assert!(entries[0].matches("U106", "x:y"));
        assert!(entries[0].matches("U1", "Lib:U106"));
        assert!(entries[1].matches("U7", "x:y") && !entries[1].matches("U8", "x:U7"));
        assert!(read(r#"[{"via_mm": [0.4, 0.2]}]"#).is_err());
        assert!(read(r#"[{"part": "U1", "via_mm": [0.2, 0.4]}]"#).is_err());
        assert!(read(r#"[{"part": "U1", "size": 1}]"#).is_err());
    }

    #[test]
    fn the_via_map_admits_an_admitted_pads_via_for_its_own_net_only() {
        let pcb = board("");
        let entries = vec![KiCadViaInPadEntry {
            part: Some("U3".into()),
            ..Default::default()
        }];
        let config = config(Some(KiCadViaInPad::Parts(entries)), &pcb);
        let board = lower(&pcb, &config, false).unwrap().board;
        let net =
            |name: &str| board.nets.iter().position(|net| net.name == name).unwrap() as core::NetId;
        let terminal = |label: &str| {
            board
                .nets
                .iter()
                .flat_map(|net| &net.terminals)
                .find(|terminal| terminal.label == label)
                .unwrap()
        };
        // U3's enclosed pads reach every layer through their vias; U1's
        // (not admitted) only F.Cu.
        assert_eq!(terminal("U3.C3").layers, 0b11);
        assert_eq!(terminal("U1.C3").layers, 0b01);
        let vias = pad_vias_of(&board);
        assert_eq!(vias.len(), 2);
        assert_eq!(vias[0], (net("Y"), [24.5, 9.5], 0.35, 0.2));
        assert_eq!(vias[1], (net("X"), [25.0, 10.0], 0.35, 0.2));
        // The pads stay closed to the search's vias.
        assert!(
            board
                .no_via_pads
                .iter()
                .any(|&index| board.obstacles[index].label == "U3.C3")
        );
        let grid = core::grid::Grid::choose(&board, &[0.05]);
        let statics = core::grid::StaticMaps::build(&board, &grid, 0, true);
        let cell = grid.nearest_node([25.0, 10.0]).unwrap();
        // On B.Cu the via is X's copper: X's tracks end on it, Y's keep
        // clear; U1's centre ball has no copper there.
        assert!(statics.trace_allowed(1, cell, net("X")));
        assert!(!statics.trace_allowed(1, cell, net("Y")));
        let free = grid.nearest_node([5.0, 10.0]).unwrap();
        assert!(statics.trace_allowed(1, free, net("Y")));
    }
}
