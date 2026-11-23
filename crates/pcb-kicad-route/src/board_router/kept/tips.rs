// Copyright (C) 2026 Florian Loitsch.

use super::*;
use crate::kicad_nets::{footprint_reference, item_net, net_codes};
use pcb_router::{Point, geometry::distance};

const MAX_LENGTH_MM: f64 = 2.5;
const NM: f64 = 0.000_001;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct RetainedPadTips {
    pub used: Vec<RetainedPadTip>,
    pub ignored: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RetainedPadTip {
    pub pad: String,
    pub net: String,
    pub layer: String,
    pub at_mm: Point,
    pub track_uuid: String,
}

type SegmentKey = (String, usize, [[i64; 2]; 2], i64);

pub(super) struct RequestedTips {
    labels: BTreeMap<String, String>,
    original_tracks: BTreeMap<SegmentKey, Option<String>>,
    layers: Vec<String>,
    report: RetainedPadTips,
}

impl RequestedTips {
    pub(super) fn new(pcb: &Expr, requested: &[String]) -> Self {
        let mut pads = BTreeMap::<String, (String, usize)>::new();
        for fp in pcb
            .children()
            .iter()
            .filter(|form| matches!(form.head(), Some("footprint" | "module")))
        {
            let Some(reference) = footprint_reference(fp) else {
                continue;
            };
            for pad in fp
                .children()
                .iter()
                .filter(|form| form.head() == Some("pad"))
            {
                let Some(number) = pad.children().get(1).and_then(Expr::atom) else {
                    continue;
                };
                pads.entry(format!("{reference}.{number}"))
                    .and_modify(|(_, count)| *count += 1)
                    .or_insert_with(|| (format!("{reference}:{number}"), 1));
            }
        }
        let layer_table = LayerTable::from_pcb(pcb).expect("board was lowered with a layer table");
        let codes = net_codes(pcb);
        let names = note_net_names(pcb);
        let mut original_tracks = BTreeMap::new();
        for item in pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("segment"))
        {
            let Some((net, layer)) = item_net(item, &codes)
                .zip(form_atom(item, "layer", 1).and_then(|layer| layer_table.index(layer)))
            else {
                continue;
            };
            let (Ok(start), Ok(end), Ok(width)) = (
                form_xy(item, "start"),
                form_xy(item, "end"),
                form_f64(item, "width", 1),
            ) else {
                continue;
            };
            let key = segment_key(names.normalize(net), layer, start, end, width);
            original_tracks
                .entry(key)
                .and_modify(|entry| *entry = None)
                .or_insert_with(|| crate::open_repair::item_id(item));
        }
        let mut result = Self {
            labels: BTreeMap::new(),
            original_tracks,
            layers: layer_table.names,
            report: RetainedPadTips::default(),
        };
        for key in requested {
            let label = key.replacen(':', ".", 1);
            if pads
                .get(&label)
                .is_some_and(|(raw, count)| raw == key && *count == 1)
            {
                result.labels.insert(label, key.clone());
                result.report.ignored.insert(
                    key.clone(),
                    "no unique short retained track attached directly to an unmerged surface pad"
                        .into(),
                );
            } else {
                result
                    .report
                    .ignored
                    .insert(key.clone(), "pad identity is unknown or ambiguous".into());
            }
        }
        result
    }

    pub(super) fn retarget(
        &mut self,
        board: &mut core::Board,
        net: core::NetId,
        terminal_index: usize,
        obstacle: usize,
    ) {
        let connection = &board.nets[net as usize];
        let terminal = &connection.terminals[terminal_index];
        let Some(label) = self.labels.get(&terminal.label) else {
            return;
        };
        let trace = &board.obstacles[obstacle];
        let pad = &board.obstacles[terminal.pad];
        let core::Shape::Capsule { start, end, radius } = trace.shape else {
            return;
        };
        if terminal.contact.is_some()
            || pad.plated
            || terminal.layers.count_ones() != 1
            || trace.layers != terminal.layers
            || !pad.shape.contains(terminal.anchor)
            || trace.label != "existing track"
            || radius <= 0.0
            || distance(start, end) > MAX_LENGTH_MM
            || board
                .merged_pads
                .iter()
                .any(|merged| merged.net == net && merged.host == terminal_index)
            || board
                .terminal_copper
                .iter()
                .any(|extra| extra.net == net && extra.terminal == terminal_index)
        {
            return;
        }
        let Some(tip) = [(start, end), (end, start)]
            .into_iter()
            .find_map(|(base, tip)| {
                (distance(base, terminal.anchor) <= NM
                    && pad.shape.distance_to_point(tip) > radius + NM)
                    .then_some(tip)
            })
        else {
            return;
        };
        let layer = terminal.layers.trailing_zeros() as usize;
        let key = segment_key(&connection.name, layer, start, end, radius * 2.0);
        let Some(Some(track_uuid)) = self.original_tracks.get(&key) else {
            return;
        };
        self.report.ignored.remove(label);
        self.report.used.push(RetainedPadTip {
            pad: label.clone(),
            net: connection.name.clone(),
            layer: self.layers[layer].clone(),
            at_mm: tip,
            track_uuid: track_uuid.clone(),
        });
        let terminal = &mut board.nets[net as usize].terminals[terminal_index];
        terminal.pad = obstacle;
        terminal.anchor = tip;
        terminal.contact = Some(core::Shape::Circle {
            center: tip,
            radius,
        });
    }

    pub(super) fn report(mut self) -> RetainedPadTips {
        self.report
            .used
            .sort_by(|left, right| left.pad.cmp(&right.pad));
        self.report
    }
}

fn segment_key(net: &str, layer: usize, start: Point, end: Point, width: f64) -> SegmentKey {
    let mut endpoints =
        [start, end].map(|point| point.map(|coordinate| (coordinate / NM).round() as i64));
    endpoints.sort_unstable();
    (net.into(), layer, endpoints, (width / NM).round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PCB: &str = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
      (general (thickness 1.6)) (paper "A4")
      (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
      (gr_rect (start 0 0) (end 30 20) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))
      (footprint "pad" (layer "F.Cu") (at 3 5)
        (property "Reference" "J1" (at 0 -2) (layer "F.SilkS"))
        (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "A")))
      (footprint "pad" (layer "F.Cu") (at 20 5)
        (property "Reference" "J2" (at 0 -2) (layer "F.SilkS"))
        (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "A")))
      (segment (start 3 5) (end 4.2 5) (width 0.23) (layer "F.Cu") (net "A")
        (uuid "447219ea-4922-4d30-8c2c-e5bb7c92c009")))"#;

    #[test]
    fn keeps_the_ordinary_anchor_and_only_retargets_an_explicit_solitary_source_track() {
        let run = |text, retained_pad_tips: Vec<String>| {
            let pcb = parse(text).unwrap();
            let config = KiCadBoardRouterConfig {
                retained_pad_tips,
                default_rules: Some(crate::KiCadConnectionRoutingRules {
                    trace_width_mm: 0.25,
                    clearance_mm: 0.20,
                    via_size_mm: 0.60,
                    via_drill_mm: 0.30,
                }),
                ..Default::default()
            };
            let mut board = lower(&pcb, &config, false).unwrap().board;
            let report = crate::board_router::kept::keep_for_route(&mut board, &pcb, &config);
            let pad = board
                .nets
                .iter()
                .find(|net| net.name == "A")
                .unwrap()
                .terminals
                .iter()
                .find(|term| term.label == "J1.1")
                .unwrap()
                .clone();
            (pad, report.retained_pad_tips)
        };
        let (pad, result) = run(PCB, Vec::new());
        assert_eq!(pad.anchor, [3.0, 5.0]);
        assert!(result.is_none());
        let (pad, result) = run(PCB, vec!["J1:1".into(), "J9:1".into(), "J2:1".into()]);
        assert_eq!(pad.anchor, [4.2, 5.0]);
        let result = result.unwrap();
        assert_eq!(result.used.len(), 1);
        assert_eq!(
            result.used[0].track_uuid,
            "447219ea-4922-4d30-8c2c-e5bb7c92c009"
        );
        assert!(result.ignored["J9:1"].contains("unknown"));
        assert!(result.ignored["J2:1"].contains("no unique"));
        let longer = format!(
            "{} (segment (start 4.2 5) (end 5 5) (width 0.23) (layer \"F.Cu\") (net \"A\")))",
            PCB.strip_suffix(')').unwrap()
        );
        let (pad, result) = run(&longer, vec!["J1:1".into()]);
        assert_eq!(pad.anchor, [3.0, 5.0]);
        assert!(result.unwrap().used.is_empty());
    }
}
