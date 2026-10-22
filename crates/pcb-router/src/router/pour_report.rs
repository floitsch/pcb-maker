// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The pours' pieces as the pour model sees them on a routed board: for
//! the verdict's pour-quality line (how whole a pour stayed), not for
//! routing.

use super::*;

/// One pour net's fill on one layer.
#[derive(Clone, Debug, PartialEq)]
pub struct PourLayerReport {
    pub net: NetId,
    pub layer: usize,
    /// The areas of the pieces that hold copper of the net (a pad the fill
    /// joins, a track or a via), in mm², largest first: what KiCad keeps.
    pub pieces_mm2: Vec<f64>,
    /// Pieces without copper of the net, which KiCad removes (islands),
    /// and their area in mm².
    pub islands: usize,
    pub islands_mm2: f64,
    /// The pour's area on the layer: lattice nodes inside its polygons
    /// and the outline, outside the pours of a higher priority, times the
    /// pitch squared, in mm² (what its copper's share is of).
    pub inside_mm2: f64,
}

impl Router {
    /// Every pour net's pieces per layer, from the pour analysis of the
    /// copper this router holds (its routes, or a routed board's tracks
    /// and vias taken over by `adopt_existing_copper`). A piece's area is
    /// its free lattice nodes (where the pour's brush fits) times the
    /// pitch squared, plus the brush's half width along its rim: KiCad's
    /// fill reaches that far beyond the brush's centres. Not counted as
    /// work: it runs after routing.
    pub fn pour_report(&self) -> Vec<PourLayerReport> {
        let mut reports = Vec::new();
        let nx = self.grid.nx;
        let cell = self.grid.pitch * self.grid.pitch;
        for net in 0..self.nets.len() as NetId {
            let state = &self.nets[net as usize];
            if state.plane.iter().all(|mask| mask.is_empty()) {
                continue;
            }
            let (pours, mut parent, _, _) = self.analyze_pours_uncounted(net, &|_| false, None);
            // Elements past the pieces are the net's terminals and
            // branches: a piece in their set holds copper of the net.
            let mut with_copper: HashSet<usize> = HashSet::new();
            for element in pours.pieces + 1..parent.len() {
                with_copper.insert(find(&mut parent, element));
            }
            for layer in 0..self.board.layer_count {
                let labels = &pours.label[layer];
                if state.plane[layer].is_empty() || labels.is_empty() {
                    continue;
                }
                let half_brush = self.board.classes[state.plane_class[layer]].trace_width / 2.0;
                let mut nodes: HashMap<u32, (usize, usize)> = HashMap::new();
                for (index, label) in labels.iter().enumerate() {
                    if *label == 0 {
                        continue;
                    }
                    // Rim nodes: a side neighbour outside the piece.
                    let rim = [index - 1, index + 1, index - nx, index + nx]
                        .iter()
                        .filter(|other| labels[**other] != *label)
                        .count();
                    let entry = nodes.entry(*label).or_default();
                    entry.0 += 1;
                    entry.1 += rim;
                }
                let mut pieces_mm2 = Vec::new();
                let (mut islands, mut islands_mm2) = (0, 0.0);
                let mut labels_sorted: Vec<(u32, (usize, usize))> = nodes.into_iter().collect();
                labels_sorted.sort_unstable_by_key(|(label, _)| *label);
                for (label, (count, rim)) in labels_sorted {
                    let area = count as f64 * cell + rim as f64 * self.grid.pitch * half_brush;
                    if with_copper.contains(&find(&mut parent, label as usize)) {
                        pieces_mm2.push(area);
                    } else {
                        islands += 1;
                        islands_mm2 += area;
                    }
                }
                pieces_mm2.sort_by(|a, b| b.total_cmp(a));
                reports.push(PourLayerReport {
                    net,
                    layer,
                    pieces_mm2,
                    islands,
                    islands_mm2,
                    inside_mm2: self.pour_area(net, layer),
                });
            }
        }
        reports
    }

    /// The area of `net`'s pours on `layer` (`PourLayerReport::inside_mm2`).
    fn pour_area(&self, net: NetId, layer: usize) -> f64 {
        let mut inside = vec![false; self.grid.cells()];
        for plane in self
            .board
            .planes
            .iter()
            .filter(|plane| plane.net == net && plane.layer == layer)
        {
            let Some((x0, y0, x1, y1)) = self.grid.node_range(
                crate::geometry::Shape::Polygon {
                    points: plane.polygon.clone(),
                }
                .aabb(),
            ) else {
                continue;
            };
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let center = self.grid.center(x, y);
                    if crate::geometry::point_in_polygon(center, &plane.polygon)
                        && crate::geometry::point_in_polygon(center, &self.board.outline)
                        && !plane
                            .excluded
                            .iter()
                            .any(|other| crate::geometry::point_in_polygon(center, other))
                    {
                        inside[self.grid.index(x, y)] = true;
                    }
                }
            }
        }
        inside.iter().filter(|inside| **inside).count() as f64 * self.grid.pitch * self.grid.pitch
    }
}
