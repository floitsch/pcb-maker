// Copyright (C) 2026 Florian Loitsch.

//! Locate the native opens that remain on each physical panel piece.

use super::*;
use pcb_kicad_file::outline_pieces::{NativeFindingLocation, NativePanelScope, Pieces};
use serde_json::Value;

const NET_SAMPLES: usize = 8;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadPanelOpens {
    by_piece: Vec<PieceOpens>,
    retained_across_pieces: usize,
    retained_unknown_piece: usize,
    reported_excluded_across: usize,
    native_unconnected_report_capped: bool,
    errors_across_pieces: BTreeMap<String, usize>,
    errors_without_piece: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Serialize)]
struct PieceOpens {
    piece: usize,
    bounds_mm: [f64; 4],
    retained_within_piece: usize,
    errors_by_type: BTreeMap<String, usize>,
    affected_nets: usize,
    net_samples: Vec<OpenNet>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    recorded_dead_pads: Vec<Value>,
}

#[derive(Clone, Debug, Serialize)]
struct OpenNet {
    net: String,
    count: usize,
    sample_items: [Value; 2],
}

pub(super) fn summarize(pcb: &Expr, drc: &Value, router: &Value) -> Option<KiCadPanelOpens> {
    if drc["coordinate_units"] != "mm" {
        return None;
    }
    let pieces = Pieces::of_board(pcb).ok()??;
    summarize_with_pieces(pcb, &pieces, drc, router)
}

fn summarize_with_pieces(
    pcb: &Expr,
    pieces: &Pieces,
    drc: &Value,
    router: &Value,
) -> Option<KiCadPanelOpens> {
    let opens = drc["unconnected_items"].as_array()?;
    let mut scope = NativePanelScope::new(pcb, pieces);
    let counts = scope.counts(drc)?;
    let mut by_piece = vec![BTreeMap::<String, OpenNet>::new(); pieces.count()];
    for open in opens {
        let Some(items @ [first, second]) = open["items"].as_array().map(Vec::as_slice) else {
            continue;
        };
        let NativeFindingLocation::Piece(piece) = scope.location(items) else {
            continue;
        };
        let a_net = first["description"].as_str().and_then(described_net);
        let b_net = second["description"].as_str().and_then(described_net);
        if let Some(net) = a_net.filter(|name| b_net.as_ref() == Some(name)) {
            let net = net.into_owned();
            by_piece[piece]
                .entry(net.clone())
                .or_insert_with(|| OpenNet {
                    net,
                    count: 0,
                    sample_items: [first.clone(), second.clone()],
                })
                .count += 1;
        }
    }
    let mut dead_by_piece = vec![Vec::new(); pieces.count()];
    for dead in router["diagnostics"]["dead_pads"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let Some(at) = dead["at"].as_array().map(Vec::as_slice) else {
            continue;
        };
        let [x, y] = at else { continue };
        if let (Some(x), Some(y), Some(net)) = (x.as_f64(), y.as_f64(), dead["net"].as_str())
            && let Some(piece) = pieces.piece_at([x, y])
            && by_piece[piece].contains_key(net)
            && dead_by_piece[piece].len() < NET_SAMPLES
        {
            dead_by_piece[piece].push(dead.clone());
        }
    }
    let by_piece = by_piece
        .into_iter()
        .zip(dead_by_piece)
        .zip(counts.pieces)
        .enumerate()
        .map(|(index, ((nets, dead), counts))| {
            let affected_nets = nets.len();
            let mut net_samples: Vec<_> = nets.into_values().collect();
            net_samples.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.net.cmp(&b.net)));
            net_samples.truncate(NET_SAMPLES);
            PieceOpens {
                piece: index + 1,
                bounds_mm: pieces.boxes[index],
                retained_within_piece: counts.opens,
                errors_by_type: counts.errors,
                affected_nets,
                net_samples,
                recorded_dead_pads: dead,
            }
        })
        .collect();
    Some(KiCadPanelOpens {
        by_piece,
        retained_across_pieces: counts.retained_across,
        retained_unknown_piece: counts.unresolved_opens,
        reported_excluded_across: counts.excluded_across,
        native_unconnected_report_capped: counts.opens_may_be_capped,
        errors_across_pieces: counts.errors_across,
        errors_without_piece: counts.errors_without_piece,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn report_separates_physical_opens_and_does_not_guess_zone_islands() {
        let board = parse("(kicad_pcb)").unwrap();
        let pieces = Pieces::of_boxes(vec![[0.0, 0.0, 10.0, 10.0], [20.0, 0.0, 30.0, 10.0]]);
        let first = json!({"description": "Pad 1 [GND] of U1 on F.Cu", "pos": {"x": 2, "y": 2}});
        let nearby = json!({"description": "Pad 1 [GND] of U2 on F.Cu", "pos": {"x": 3, "y": 2}});
        let over = json!({"description": "Pad 1 [GND] of U3 on F.Cu", "pos": {"x": 22, "y": 2}});
        let zone = json!({"description": "Zone [GND] on F.Cu", "pos": {"x": 3, "y": 2}, "uuid": "unknown"});
        let native = json!({"coordinate_units": "mm",
        "unconnected_items": [{"items": [first, nearby]}, {"items": [first, over]}, {"items": [first, zone]}],
        "unconnected_across_sub_boards": [{"items": [nearby, over]}],
        "violations": [
            {"severity": "error", "type": "clearance", "items": [first, nearby]},
            {"severity": "error", "type": "edge_clearance", "items": [nearby, over]},
            {"severity": "error", "type": "hole_clearance", "items": [zone]},
            {"severity": "warning", "type": "silk_overlap", "items": [nearby, over]}
        ]});
        let router = json!({"diagnostics": {"dead_pads": [{"at": [2, 2], "pad": "U1.1", "net": "GND"},
                                                        {"at": [2, 3], "pad": "U2.2", "net": "ignored"}]}});
        let report = summarize_with_pieces(&board, &pieces, &native, &router).unwrap();
        assert_eq!(report.retained_across_pieces, 1);
        assert_eq!(report.retained_unknown_piece, 1);
        assert_eq!(report.reported_excluded_across, 1);
        assert_eq!(report.by_piece[0].retained_within_piece, 1);
        assert_eq!(report.by_piece[0].net_samples[0].net, "GND");
        assert_eq!(report.by_piece[0].recorded_dead_pads.len(), 1);
        assert_eq!(report.by_piece[0].errors_by_type["clearance"], 1);
        assert_eq!(report.errors_across_pieces["edge_clearance"], 1);
        assert_eq!(report.errors_without_piece["hole_clearance"], 1);
        assert_eq!(report.by_piece[1].retained_within_piece, 0);
    }
}
