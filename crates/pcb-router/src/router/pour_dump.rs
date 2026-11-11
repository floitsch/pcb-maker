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
            let (pours, mut parent, main, _) =
                self.analyze_pours_reporting(net, &|_| false, Some(&mut spokes));
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
                        "{{\"net\": \"{}\", \"pad\": \"{}\", \"layer\": {}, \"anchor\": [{}, {}], \"round\": {}, \"spokes\": [{}], \"tracks\": {}, \"crossings\": {}, \"seam_short\": {}, \"required\": {}}}",
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
                        record.required,
                    )?;
                }
            }
            // Per terminal, whether the model joins it to the main piece
            // (`pour_stranded_of`'s test): what the router counts as
            // routed, beside KiCad's connectivity
            // (experiments/pour-fidelity/agreement.py).
            // On a board's own copper (`adopt_existing_copper`, one branch
            // per track segment and via) segments that meet at a node are
            // joined here as KiCad joins copper that touches: the
            // analysis's junctions are the router's own branches'.
            {
                let mut file = std::io::BufWriter::new(std::fs::File::create(
                    directory.join(format!("{file_name}.terminals")),
                )?);
                let terminal_base = pours.pieces + 1;
                let branch_base = terminal_base + state.terminal_nodes.len();
                let mut first: HashMap<Node, usize> = HashMap::new();
                for (index, branch) in state.branches.iter().enumerate() {
                    for node in &branch.nodes {
                        let other = *first.entry(*node).or_insert(index);
                        let (a, b) = (
                            find(&mut parent, branch_base + index),
                            find(&mut parent, branch_base + other),
                        );
                        parent[a] = b;
                    }
                }
                let mut votes: HashMap<usize, usize> = HashMap::new();
                for terminal in 0..state.terminal_nodes.len() {
                    *votes
                        .entry(find(&mut parent, terminal_base + terminal))
                        .or_default() += 1;
                }
                let main = votes
                    .iter()
                    .max_by_key(|(root, count)| (**count, usize::MAX - **root))
                    .map_or(main, |(root, _)| *root);
                for (index, nodes) in state.terminal_nodes.iter().enumerate() {
                    let terminal = &self.board.nets[net as usize].terminals[index];
                    let layers: std::collections::BTreeSet<u8> =
                        nodes.iter().map(|node| node.layer).collect();
                    writeln!(
                        file,
                        "{{\"net\": \"{}\", \"pad\": \"{}\", \"anchor\": [{}, {}], \"layers\": {:?}, \"connected\": {}}}",
                        name.replace('\\', "\\\\").replace('"', "\\\""),
                        terminal.label.replace('\\', "\\\\").replace('"', "\\\""),
                        terminal.anchor[0],
                        terminal.anchor[1],
                        layers.iter().collect::<Vec<_>>(),
                        find(&mut parent, terminal_base + index) == main,
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
                    if self.occupancy.get(map, cell) as usize == own.contains(map, cell) as usize {
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
