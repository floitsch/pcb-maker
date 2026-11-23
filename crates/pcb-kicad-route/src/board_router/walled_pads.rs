// Copyright (C) 2026 Florian Loitsch.

//! The pads a route wired through their own footprint's copper, as the
//! verdict reports them (`walled_pads` in verdict.json).
//!
//! The lowering cuts a corridor through a footprint's copper graphic for a
//! pad that graphic walls in (lower/own_copper.rs). KiCad keeps the graphic
//! copper of no net, so the track that crosses it there is a finding
//! (`shorting_items`, or `clearance` at 0 mm) whatever the route does. Here
//! the same lowering of the written board, its routed copper left out,
//! names those pads and their corridors, and the written board's tracks and
//! vias of each pad's net that run in its corridors are its crossings:
//! the verdict sets a finding apart only when it pairs one of those
//! graphics with one of those crossings (`forced_by_footprint`).

use super::*;
use pcb_router as core;

/// A copper graphic a walled pad's corridors cut: its uuid as KiCad's DRC
/// report names it (None where the board file gives none) and its kind
/// (`fp_poly`, `fp_line`, ...).
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct KiCadWalledGraphic {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    pub kind: String,
}

/// A pad walled in by its own footprint's copper and wired through it
/// (docs/router.md, "A pad walled by its own footprint's copper").
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct KiCadWalledPad {
    /// The footprint's reference and the pad's number.
    pub part: String,
    pub pad: String,
    pub net: String,
    /// The copper layer of its corridors.
    pub layer: String,
    /// The graphics its corridors cut.
    pub graphics: Vec<KiCadWalledGraphic>,
    /// The written board's tracks and vias of the pad's net in its
    /// corridors, by uuid.
    pub crossings: Vec<String>,
    /// The footprint's pads its graphics touch (numbers): two or more can
    /// be made a net tie (`net_tie_pad_groups`), one a custom pad.
    pub touching: Vec<String>,
    /// KiCad's findings between one of `graphics` and one of `crossings`
    /// (type and description), set apart by the verdict.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
}

/// The walled pads of a written board (`pcb`) under `config`, each with the
/// written tracks and vias that cross its corridors; empty with
/// `walled_pad_exits` false or on a board without such pads.
pub fn walled_pads_of(
    pcb: &Expr,
    config: &KiCadBoardRouterConfig,
) -> Result<Vec<KiCadWalledPad>, String> {
    if config.walled_pad_exits == Some(false) || !has_footprint_copper(pcb) {
        return Ok(Vec::new());
    }
    // The pads as the lowering met them: without the routed copper, which
    // joins the pads it reaches.
    let mut bare = pcb.clone();
    strip_tracks(&mut bare);
    let lowered = lower(&bare, config, true)?;
    if lowered.walled_pads.is_empty() {
        return Ok(Vec::new());
    }
    let layers = LayerTable::from_pcb(pcb)?;
    let copper = routed_copper(pcb, &layers);
    note_net_names(pcb);
    Ok(lowered
        .walled_pads
        .into_iter()
        .map(|walled| {
            let crossings = copper
                .iter()
                .filter(|item| {
                    item.net == walled.net
                        && item.layers & (1 << walled.layer) != 0
                        && walled.corridors.iter().any(|(start, end, radius)| {
                            item.points.windows(2).any(|pair| {
                                core::geometry::segment_segment_distance(
                                    pair[0], pair[1], *start, *end,
                                ) < *radius
                            }) || (item.points.len() == 1
                                && core::geometry::point_segment_distance(
                                    item.points[0],
                                    *start,
                                    *end,
                                ) < *radius)
                        })
                })
                .map(|item| item.uuid.clone())
                .collect();
            KiCadWalledPad {
                part: walled.reference,
                pad: walled.pad,
                net: written_net_name(&walled.net),
                layer: layers.names[walled.layer].clone(),
                graphics: walled
                    .graphics
                    .into_iter()
                    .map(|(uuid, kind)| KiCadWalledGraphic { uuid, kind })
                    .collect(),
                crossings,
                touching: walled.touching,
                findings: Vec::new(),
            }
        })
        .collect())
}

/// `walled_pads_of` the board file at `board`; none where it cannot be
/// read.
pub fn walled_pads_of_file(board: &Path, config: &KiCadBoardRouterConfig) -> Vec<KiCadWalledPad> {
    fs::read_to_string(board)
        .ok()
        .and_then(|text| parse(&text).ok())
        .and_then(|pcb| walled_pads_of(&pcb, config).ok())
        .unwrap_or_default()
}

/// Whether any footprint draws copper of its own (a graphic on a copper
/// layer): the boards that can have walled pads.
fn has_footprint_copper(pcb: &Expr) -> bool {
    pcb.children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .flat_map(|footprint| footprint.children().iter())
        .filter(|child| {
            matches!(
                child.head(),
                Some("fp_line" | "fp_arc" | "fp_rect" | "fp_circle" | "fp_poly")
            )
        })
        .any(|graphic| {
            item_layer_names(graphic)
                .iter()
                .any(|name| name.ends_with(".Cu"))
        })
}

/// A written track, arc or via: its uuid, net name, copper layers and
/// points (a track's ends, an arc's start, middle and end, a via's centre).
struct RoutedItem {
    uuid: String,
    net: String,
    layers: core::LayerMask,
    points: Vec<[f64; 2]>,
}

/// The board's tracks, arcs and vias with a uuid and a net.
fn routed_copper(pcb: &Expr, layers: &LayerTable) -> Vec<RoutedItem> {
    let names = NetNames::of(pcb);
    let codes = net_codes(pcb);
    let mut items = Vec::new();
    for item in pcb.children() {
        let head = item.head();
        if !matches!(head, Some("segment" | "arc" | "via")) {
            continue;
        }
        let Some(uuid) = form_atom(item, "uuid", 1).map(|uuid| uuid.trim_matches('"').to_string())
        else {
            continue;
        };
        let Some(net) =
            item_net(item, &codes).map(|raw| names.normalize(raw.trim_matches('"')).to_string())
        else {
            continue;
        };
        let point = |name: &str| form_xy(item, name).ok();
        let (points, mask) = match head {
            Some("via") => (
                point("at").into_iter().collect(),
                layers.via_mask(item).max(1),
            ),
            Some("arc") => (
                ["start", "mid", "end"]
                    .into_iter()
                    .filter_map(point)
                    .collect(),
                layers.mask_of_item(item).unwrap_or(0),
            ),
            _ => (
                ["start", "end"].into_iter().filter_map(point).collect(),
                layers.mask_of_item(item).unwrap_or(0),
            ),
        };
        items.push(RoutedItem {
            uuid,
            net,
            layers: mask,
            points,
        });
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A board with AE1 at (10, 10): pad 1 of /A inside its polygon (which
    /// reaches 1.5 mm to the right), pad 2 of /B away; `tracks` beside it.
    fn board(tracks: &str) -> Expr {
        parse(&format!(
            r#"(kicad_pcb (version 20240108)
              (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (net 0 "") (net 1 "/A") (net 2 "/B")
              (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
              (footprint "Antenna" (layer "F.Cu") (at 10 10)
                (property "Reference" "AE1" (at 0 0) (layer "F.Fab"))
                (fp_poly (pts (xy -0.4 -0.4) (xy 1.5 -0.4) (xy 1.5 0.4) (xy -0.4 0.4))
                  (stroke (width 0) (type solid)) (fill solid) (layer "F.Cu") (uuid "g1"))
                (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net 1 "/A"))
                (pad "2" smd rect (at 4 0) (size 0.5 0.5) (layers "F.Cu") (net 2 "/B")))
              {tracks})"#
        ))
        .unwrap()
    }

    fn config() -> KiCadBoardRouterConfig {
        KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        }
    }

    #[test]
    fn a_walled_pad_names_its_graphic_and_the_tracks_in_its_corridors() {
        // t1 leaves pad 1 upwards along a corridor; t2 of the same net
        // runs over the polygon's far end, in no corridor; t3 is /B's.
        let pcb = board(
            r#"(segment (start 10 10) (end 10 8) (width 0.2) (layer "F.Cu") (net 1) (uuid "t1"))
               (segment (start 11.3 9) (end 11.3 11) (width 0.2) (layer "F.Cu") (net 1) (uuid "t2"))
               (segment (start 10 10.1) (end 10 12) (width 0.2) (layer "B.Cu") (net 1) (uuid "t4"))
               (segment (start 9.8 8) (end 9.8 9.5) (width 0.2) (layer "F.Cu") (net 2) (uuid "t3"))"#,
        );
        let pads = walled_pads_of(&pcb, &config()).unwrap();
        assert_eq!(pads.len(), 1);
        let pad = &pads[0];
        assert_eq!(
            (
                pad.part.as_str(),
                pad.pad.as_str(),
                pad.net.as_str(),
                pad.layer.as_str()
            ),
            ("AE1", "1", "/A", "F.Cu")
        );
        assert_eq!(
            pad.graphics,
            [KiCadWalledGraphic {
                uuid: Some("g1".into()),
                kind: "fp_poly".into()
            }]
        );
        assert_eq!(pad.crossings, ["t1"]);
        assert_eq!(pad.touching, ["1"]);
        // Off: no walled pad is wired, none is reported.
        let off = KiCadBoardRouterConfig {
            walled_pad_exits: Some(false),
            ..config()
        };
        assert!(walled_pads_of(&pcb, &off).unwrap().is_empty());
    }
}
