// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Dumping the pour maps (`PCB_ROUTER_POUR_DUMP`, dump-pour-map).

use super::*;

impl Router {
    pub fn dump_pours(&self, directory: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        std::fs::create_dir_all(directory)?;
        let layers = self.board.layer_count;
        let cells = self.grid.cells();
        for net in 0..self.nets.len() as NetId {
            let state = &self.nets[net as usize];
            if state.plane.is_empty() {
                continue;
            }
            let mut spokes = Vec::new();
            let (pours, _, _, _) = self.analyze_pours_reporting(net, &|_| false, Some(&mut spokes));
            let mut rings: Vec<Vec<bool>> = (0..layers)
                .map(|layer| {
                    if state.plane[layer].is_empty() {
                        Vec::new()
                    } else {
                        vec![true; cells]
                    }
                })
                .collect();
            self.cut_thermal_rings(&self.thermal_ring_pads(net), &mut rings);
            let own = OwnStamps::new(
                &state.stamped,
                &(0..layers)
                    .filter(|layer| !state.plane[*layer].is_empty())
                    .map(|layer| self.map_index(state.plane_class[layer], layer))
                    .collect::<Vec<_>>(),
                cells,
            );
            let name = &self.board.nets[net as usize].name;
            let file_name: String = name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            // Per pad and layer, the spokes as the model sees them: one
            // JSON object a line.
            {
                let mut file = std::io::BufWriter::new(std::fs::File::create(
                    directory.join(format!("{file_name}.spokes")),
                )?);
                for record in &spokes {
                    let terminal = &self.board.nets[net as usize].terminals[record.terminal];
                    let outcomes: Vec<String> = record
                        .outcomes
                        .iter()
                        .map(|outcome| match outcome {
                            SpokeOutcome::NoFill => "\"none\"".to_string(),
                            SpokeOutcome::Blocked => "\"blocked\"".to_string(),
                            SpokeOutcome::Island(piece) => format!("\"island {piece}\""),
                            SpokeOutcome::Good(piece) => format!("\"good {piece}\""),
                            SpokeOutcome::Pad(other) => format!("\"good pad {other}\""),
                        })
                        .collect();
                    writeln!(
                        file,
                        "{{\"net\": \"{}\", \"pad\": \"{}\", \"layer\": {}, \"anchor\": [{}, {}], \"round\": {}, \"spokes\": [{}], \"tracks\": {}, \"crossings\": {}, \"seam_short\": {}}}",
                        name.replace('\\', "\\\\").replace('"', "\\\""),
                        terminal.label.replace('\\', "\\\\").replace('"', "\\\""),
                        record.layer,
                        record.anchor[0],
                        record.anchor[1],
                        record.round,
                        outcomes.join(", "),
                        record.tracks,
                        record.crossings,
                        record.seam_short,
                    )?;
                }
            }
            for (layer, ring) in rings.iter().enumerate() {
                if state.plane[layer].is_empty() {
                    continue;
                }
                let mut flags = vec![0u8; cells];
                for plane in self
                    .board
                    .planes
                    .iter()
                    .filter(|plane| plane.net == net && plane.layer == layer && plane.connect)
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
                                && !plane
                                    .excluded
                                    .iter()
                                    .any(|other| crate::geometry::point_in_polygon(center, other))
                            {
                                flags[self.grid.index(x, y)] |= 1;
                            }
                        }
                    }
                }
                let statics = &self.statics[state.plane_class[layer]];
                let map = self.map_index(state.plane_class[layer], layer);
                for cell in 0..cells {
                    if statics.fill_allowed(layer, cell, net) {
                        flags[cell] |= 2;
                    }
                    if self.occupancy[map][cell] as usize == own.contains(map, cell) as usize {
                        flags[cell] |= 4;
                    }
                    if !ring[cell] {
                        flags[cell] |= 8;
                    }
                    if pours.solid[layer][cell] {
                        flags[cell] |= 16;
                    }
                }
                for branch in &state.branches {
                    for node in branch
                        .nodes
                        .iter()
                        .filter(|node| node.layer as usize == layer)
                    {
                        flags[node.cell as usize] |= 32;
                    }
                }
                let path = directory.join(format!("{file_name}-L{layer}.pour"));
                let mut file = std::io::BufWriter::new(std::fs::File::create(&path)?);
                writeln!(
                    file,
                    "{{\"net\": \"{}\", \"layer\": {layer}, \"nx\": {}, \"ny\": {}, \"origin\": [{}, {}], \"pitch\": {}, \"pieces\": {}}}",
                    name.replace('\\', "\\\\").replace('"', "\\\""),
                    self.grid.nx,
                    self.grid.ny,
                    self.grid.origin[0],
                    self.grid.origin[1],
                    self.grid.pitch,
                    pours.pieces,
                )?;
                file.write_all(&flags)?;
                for label in &pours.label[layer] {
                    file.write_all(&label.to_le_bytes())?;
                }
            }
        }
        Ok(())
    }
}
