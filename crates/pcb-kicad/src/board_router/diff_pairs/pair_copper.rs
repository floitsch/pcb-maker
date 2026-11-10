// Copyright (C) 2026 Toit contributors.

//! The pairs' copper of a written board (`finish_pairs`) and its writing
//! as tracks, vias and tuning patterns of the pairs' nets.

use super::*;
use pcb_router as core;

/// The pair copper of a written board (`finish_pairs`), to write as
/// tracks and vias of the pairs' nets (`write_pair_copper`).
pub(crate) struct PairCopper {
    /// (net name, width, via diameter, via drill, segments, vias).
    #[allow(clippy::type_complexity)]
    pub(super) items: Vec<(
        String,
        f64,
        f64,
        f64,
        Vec<(usize, Point, Point)>,
        Vec<Point>,
    )>,
    /// Meanders, written as KiCad tuning patterns (`generated`, type
    /// `tuning_pattern`: KiCad's gap check skips their tracks).
    pub(super) patterns: Vec<Meander>,
}

impl PairCopper {
    /// The copper as routes (tracks and vias of no net in particular), for
    /// what keeps off copper (`place_copper_texts`).
    pub(crate) fn routes(&self) -> Vec<core::NetRoute> {
        let mut routes: Vec<core::NetRoute> = self
            .items
            .iter()
            .map(|(_, width, size, drill, segments, vias)| core::NetRoute {
                segments: segments
                    .iter()
                    .map(|&(layer, start, end)| core::Segment {
                        layer,
                        start,
                        end,
                        width: *width,
                    })
                    .collect(),
                vias: vias
                    .iter()
                    .map(|&at| core::Via {
                        at,
                        diameter: *size,
                        drill: *drill,
                        span: None,
                    })
                    .collect(),
            })
            .collect();
        routes.extend(self.patterns.iter().map(|meander| {
            core::NetRoute {
                segments: meander
                    .points
                    .windows(2)
                    .map(|pair| core::Segment {
                        layer: meander.layer,
                        start: pair[0],
                        end: pair[1],
                        width: meander.width,
                    })
                    .collect(),
                vias: Vec::new(),
            }
        }));
        routes
    }
}

/// A kept link's run (`match_skew`): its pair, its copper's items in
/// `PairCopper::items` (positive, negative), their run ranges
/// (`LinkCopper::run`) and the positive net's side.
pub(super) type KeptRun = (usize, [usize; 2], [std::ops::Range<usize>; 2], f64);

/// A meander on one net of a pair's coupled run (`match_skew`).
#[derive(Clone, Debug)]
pub(super) struct Meander {
    pub net: String,
    pub width: f64,
    pub layer: usize,
    /// The meander's polyline, from where it leaves the run's track to
    /// where it rejoins it.
    pub points: Vec<Point>,
    pub gap: f64,
    pub amplitude: f64,
    pub spacing: f64,
    /// The length it adds.
    pub added: f64,
}

/// Writes the pairs' copper into `pcb` as tracks and vias of their nets.
pub(crate) fn write_pair_copper(
    pcb: &mut Expr,
    copper: &PairCopper,
    layer_names: &[String],
) -> Result<(), String> {
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    for (name, width, size, drill, segments, vias) in &copper.items {
        for &(layer, start, end) in segments {
            items.push(supplemental_segment_expr(&SupplementalSegment {
                connection: name.clone(),
                start: nanometres(start),
                end: nanometres(end),
                width: *width,
                layer: layer_names[layer].clone(),
            }));
        }
        for &at in vias {
            items.push(supplemental_via_expr(&SupplementalVia {
                connection: name.clone(),
                at: nanometres(at),
                size: *size,
                drill: *drill,
                layers: ["F.Cu".into(), "B.Cu".into()],
                kind: None,
            }));
        }
    }
    // The meanders: their tracks, grouped as KiCad's tuning patterns (a
    // diff pair skew pattern; KiCad's gap check skips a pattern's tracks).
    for meander in &copper.patterns {
        let layer = layer_names[meander.layer].clone();
        let mut members = Vec::new();
        for pair in meander.points.windows(2) {
            let segment = supplemental_segment_expr(&SupplementalSegment {
                connection: meander.net.clone(),
                start: nanometres(pair[0]),
                end: nanometres(pair[1]),
                width: meander.width,
                layer: layer.clone(),
            });
            if let Some(uuid) = segment
                .child("uuid")
                .and_then(|uuid| uuid.children().get(1))
                .and_then(Expr::atom)
            {
                members.push(Expr::Atom(quote(uuid)));
            }
            items.push(segment);
        }
        let tuned = Tuned {
            net: &meander.net,
            width: meander.width,
            points: &meander.points,
            amplitude: meander.amplitude,
            spacing: meander.spacing,
            added: meander.added,
            tuning: Tuning::PairSkew { gap: meander.gap },
        };
        items.push(tuning_pattern_expr(&tuned, &layer, members));
    }
    Ok(())
}
