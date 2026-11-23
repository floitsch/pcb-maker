// Copyright (C) 2026 Florian Loitsch.

//! Observe track lengths for simple pairs routed as separate nets.

use super::*;
use pcb_kicad_file::copper_geometry::lower_pad;

type Coordinate = [i64; 2];

fn coordinate(at: [f64; 2]) -> Coordinate {
    at.map(|mm| (mm * 1.0e6).round() as i64)
}

#[derive(Default)]
struct Conductor<'a> {
    name: &'a str,
    pads: BTreeMap<Coordinate, String>,
    tracks: BTreeMap<(Coordinate, &'a str), Vec<(Coordinate, &'a str)>>,
    vias: BTreeMap<Coordinate, [&'a str; 2]>,
    length_mm: f64,
    unsupported: bool,
}

impl<'a> Conductor<'a> {
    fn add_item(&mut self, item: &'a Expr) -> Option<()> {
        match item.head()? {
            "segment" => {
                let start = form_xy(item, "start").ok()?;
                let end = form_xy(item, "end").ok()?;
                let layer = form_atom(item, "layer", 1)?;
                let a = (coordinate(start), layer);
                let b = (coordinate(end), layer);
                if a == b {
                    return None;
                }
                self.tracks.entry(a).or_default().push(b);
                self.tracks.entry(b).or_default().push(a);
                self.length_mm += pcb_router::geometry::distance(start, end);
            }
            "via" => {
                let at = coordinate(form_xy(item, "at").ok()?);
                let span = [form_atom(item, "layers", 1)?, form_atom(item, "layers", 2)?];
                if self.vias.insert(at, span).is_some() {
                    return None;
                }
            }
            _ => return None,
        }
        Some(())
    }

    /// A simple two-pad centreline chain. KiCad already found no opens.
    /// Branches, loops, duplicate tracks and vias with ambiguous joins
    /// don't provide comparable conductor lengths.
    fn terminals(&self, layers: &LayerTable) -> Option<[String; 2]> {
        if self.unsupported || self.pads.len() != 2 || self.length_mm <= 0.0 {
            return None;
        }
        let mut graph = self.tracks.clone();
        for (&at, [first, last]) in &self.vias {
            let a = layers.index(first)?;
            let b = layers.index(last)?;
            let span = a.min(b)..=a.max(b);
            let ends: Vec<_> = graph
                .range((at, "")..)
                .map(|(node, _)| *node)
                .take_while(|(point, _)| *point == at)
                .filter(|(_, layer)| {
                    layers
                        .index(layer)
                        .is_some_and(|index| span.contains(&index))
                })
                .collect();
            let [a, b] = ends.as_slice() else {
                return None;
            };
            graph.get_mut(a)?.push(*b);
            graph.get_mut(b)?.push(*a);
        }
        let tips: Vec<_> = graph
            .iter()
            .filter_map(|(node, neighbours)| (neighbours.len() == 1).then_some(*node))
            .collect();
        let [first, last] = tips.as_slice() else {
            return None;
        };
        if graph.values().any(|neighbours| neighbours.len() > 2) || first.0 == last.0 {
            return None;
        }
        let mut walked = BTreeSet::new();
        let mut current = *first;
        while walked.insert(current) {
            let Some(&next) = graph[&current].iter().find(|node| !walked.contains(*node)) else {
                break;
            };
            current = next;
        }
        if current != *last || walked.len() != graph.len() {
            return None;
        }
        Some([
            self.pads.get(&first.0)?.clone(),
            self.pads.get(&last.0)?.clone(),
        ])
    }
}

/// Called on the actual final PCB after native DRC has checked connectivity.
/// This reports an observation without prescribing an electrical limit.
pub fn measure_fallback_lengths(report: &mut KiCadDiffPairs, pcb: &Expr) {
    let mut conductors: BTreeMap<_, Conductor<'_>> = report
        .pairs
        .iter()
        .filter(|pair| pair.links_coupled == 0)
        .flat_map(|pair| [&pair.positive, &pair.negative])
        .map(|name| (name.as_str(), Conductor::default()))
        .collect();
    let mut measurements = Vec::new();
    if !conductors.is_empty()
        && let Ok(layers) = LayerTable::from_pcb(pcb)
    {
        let codes = net_codes(pcb);
        let names = NetNames::of(pcb);
        for item in pcb.children() {
            if matches!(item.head(), Some("footprint" | "module")) {
                for part in item.children() {
                    if let Some(raw) = item_net(part, &codes)
                        && let Some(conductor) = conductors.get_mut(names.normalize(raw))
                    {
                        conductor.name = raw;
                        conductor.unsupported |= match part.head() {
                            Some("pad") => {
                                let pad =
                                    form_at(item).ok().and_then(|at| lower_pad(part, at).ok());
                                match (
                                    footprint_reference(item),
                                    part.children().get(1).and_then(Expr::atom),
                                    pad,
                                ) {
                                    (Some(reference), Some(pin), Some(pad)) if !pin.is_empty() => {
                                        conductor
                                            .pads
                                            .insert(
                                                coordinate(pad.center),
                                                format!("{reference}:{pin}"),
                                            )
                                            .is_some()
                                    }
                                    _ => true,
                                }
                            }
                            _ => true,
                        };
                    }
                }
            } else if let Some(raw) = item_net(item, &codes)
                && let Some(conductor) = conductors.get_mut(names.normalize(raw))
            {
                conductor.unsupported |= conductor.add_item(item).is_none();
            }
        }
        let evidence = |connection: &str| {
            let conductor = conductors.get(connection)?;
            Some(KiCadFallbackConductor {
                net: conductor.name.to_owned(),
                pads: conductor.terminals(&layers)?,
                planar_track_length_mm: conductor.length_mm,
            })
        };
        measurements.extend(
            report
                .pairs
                .iter()
                .filter(|pair| pair.links_coupled == 0)
                .filter_map(|pair| {
                    let positive = evidence(&pair.positive)?;
                    let negative = evidence(&pair.negative)?;
                    Some(KiCadFallbackPairLength {
                        planar_difference_mm: (positive.planar_track_length_mm
                            - negative.planar_track_length_mm)
                            .abs(),
                        positive,
                        negative,
                    })
                }),
        );
        measurements.sort_by(|a, b| {
            b.planar_difference_mm
                .total_cmp(&a.planar_difference_mm)
                .then(a.positive.net.cmp(&b.positive.net))
        });
    }
    report.fallback_lengths = measurements;
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOARD: &str = r#"(kicad_pcb
        (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (footprint "start" (at 0 0)
          (property "Reference" "J1")
          (pad "1" smd circle (at 0 0) (size 1 1) (net "/TX_P"))
          (pad "2" smd circle (at 0 1) (size 1 1) (net "/TX_N")))
        (footprint "end" (at 12 0)
          (property "Reference" "J2")
          (pad "1" smd circle (at 0 0) (size 1 1) (net "/TX_P"))
          (pad "2" smd circle (at -1 1) (size 1 1) (net "/TX_N")))
        (segment (start 0 0) (end 5 0) (layer "F.Cu") (net "/TX_P"))
        (via (at 5 0) (layers "F.Cu" "B.Cu") (net "/TX_P"))
        (segment (start 5 0) (end 12 0) (layer "B.Cu") (net "/TX_P"))
        (segment (start 0 1) (end 11 1) (layer "F.Cu") (net "/TX_N")))"#;

    #[test]
    fn fallback_measures_a_simple_track_chain_with_an_exact_via_join() {
        let mut report = KiCadDiffPairs {
            pairs: vec![KiCadDiffPair {
                positive: "TX_P".into(),
                negative: "TX_N".into(),
                links: 1,
                ..KiCadDiffPair::default()
            }],
            ..KiCadDiffPairs::default()
        };
        measure_fallback_lengths(&mut report, &parse(BOARD).unwrap());
        let pair = &report.fallback_lengths[0];
        assert_eq!(pair.positive.net, "/TX_P");
        assert_eq!(pair.positive.pads, ["J1:1", "J2:1"]);
        assert_eq!(pair.negative.pads, ["J1:2", "J2:2"]);
        assert_eq!(pair.positive.planar_track_length_mm, 12.0);
        assert_eq!(pair.negative.planar_track_length_mm, 11.0);
        assert_eq!(pair.planar_difference_mm, 1.0);
        assert!(report.skews.is_empty());

        for extra in [
            r#"(segment (start 5 0) (end 5 4) (layer "B.Cu") (net "/TX_P"))"#,
            r#"(zone (net "/TX_P") (layer "F.Cu"))"#,
        ] {
            let pcb = parse(&format!("{} {extra})", BOARD.strip_suffix(')').unwrap())).unwrap();
            measure_fallback_lengths(&mut report, &pcb);
            assert!(report.fallback_lengths.is_empty());
        }
    }
}
