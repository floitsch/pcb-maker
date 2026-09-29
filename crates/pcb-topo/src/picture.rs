// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! SVG pictures of the engine's stages, for debugging. Set `PCB_TOPO_SVG`
//! to a directory and every stage writes one: the triangulation with the
//! topological wires (drawn through their crossing points, coloured by
//! layer once layers are assigned), and the realized copper with the
//! verifier's violations.

use crate::mesh::Mesh;
use crate::topo::Topology;
use pcb_router::{Board, NetRoute, ObstacleKind, Point, Shape, Violation};
use std::fmt::Write;
use std::path::PathBuf;

/// Where pictures go, if anywhere.
pub fn directory() -> Option<PathBuf> {
    std::env::var_os("PCB_TOPO_SVG").map(PathBuf::from)
}

pub fn layer_colour(layer: usize) -> &'static str {
    ["#d23c3c", "#3c64d2", "#2ca05a", "#d28c1e", "#8c3cd2", "#1eaab4"][layer % 6]
}

/// A colour per net, for wires without a layer.
pub fn net_colour(net: u32) -> String {
    let hue = (net as f64 * 137.508) % 360.0;
    format!("hsl({hue:.0},70%,45%)")
}

pub struct Picture {
    minimum: Point,
    maximum: Point,
    body: String,
}

impl Picture {
    pub fn new(board: &Board) -> Self {
        let mut minimum = [f64::INFINITY; 2];
        let mut maximum = [f64::NEG_INFINITY; 2];
        for point in &board.outline {
            for axis in 0..2 {
                minimum[axis] = minimum[axis].min(point[axis]);
                maximum[axis] = maximum[axis].max(point[axis]);
            }
        }
        if !minimum[0].is_finite() {
            minimum = [0.0, 0.0];
            maximum = [100.0, 100.0];
        }
        let mut picture = Self { minimum, maximum, body: String::new() };
        picture.polygon(&board.outline, "none", "#888", 0.1);
        picture
    }

    pub fn polygon(&mut self, points: &[Point], fill: &str, stroke: &str, width: f64) {
        let list: Vec<String> = points.iter().map(|p| format!("{:.4},{:.4}", p[0], p[1])).collect();
        let _ = write!(
            self.body,
            r#"<polygon points="{}" fill="{fill}" stroke="{stroke}" stroke-width="{width}"/>"#,
            list.join(" ")
        );
    }

    pub fn polyline(&mut self, points: &[Point], stroke: &str, width: f64, opacity: f64) {
        let list: Vec<String> = points.iter().map(|p| format!("{:.4},{:.4}", p[0], p[1])).collect();
        let _ = write!(
            self.body,
            r#"<polyline points="{}" fill="none" stroke="{stroke}" stroke-width="{width}" stroke-opacity="{opacity}" stroke-linecap="round" stroke-linejoin="round"/>"#,
            list.join(" ")
        );
    }

    pub fn circle(&mut self, center: Point, radius: f64, fill: &str, stroke: &str, width: f64) {
        let _ = write!(
            self.body,
            r#"<circle cx="{:.4}" cy="{:.4}" r="{radius:.4}" fill="{fill}" stroke="{stroke}" stroke-width="{width}"/>"#,
            center[0], center[1]
        );
    }

    pub fn text(&mut self, at: Point, size: f64, colour: &str, text: &str) {
        let escaped = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let _ = write!(
            self.body,
            r#"<text x="{:.4}" y="{:.4}" font-size="{size}" fill="{colour}" font-family="sans-serif">{escaped}</text>"#,
            at[0], at[1]
        );
    }

    pub fn shape(&mut self, shape: &Shape, fill: &str, stroke: &str) {
        match shape {
            Shape::Circle { center, radius } => self.circle(*center, *radius, fill, stroke, 0.03),
            Shape::Capsule { start, end, radius } => {
                self.polyline(&[*start, *end], fill, 2.0 * radius, 1.0);
            }
            Shape::Polygon { points } => self.polygon(points, fill, stroke, 0.03),
            Shape::Union { parts } => parts.iter().for_each(|part| self.shape(part, fill, stroke)),
        }
    }

    /// Pads, holes and keepouts, tinted by the layers they are on.
    pub fn obstacles(&mut self, board: &Board) {
        for obstacle in &board.obstacles {
            let fill = match obstacle.kind {
                ObstacleKind::Hole => "#555",
                ObstacleKind::Keepout => "#f0e0a0",
                ObstacleKind::Copper if obstacle.layers == board.all_layers() => "#b8b8b8",
                ObstacleKind::Copper if obstacle.layers & 1 != 0 => "#f0b4b4",
                ObstacleKind::Copper => "#b4c4f0",
            };
            self.shape(&obstacle.shape, fill, "#777");
        }
    }

    /// The shared triangulation: constraint edges darker, via sites dotted.
    pub fn mesh(&mut self, mesh: &Mesh) {
        for (edge, [a, b]) in mesh.edges.iter().enumerate() {
            let (colour, width) = if mesh.constraint[edge] { ("#999", 0.03) } else { ("#ddd", 0.02) };
            self.polyline(&[mesh.points[*a], mesh.points[*b]], colour, width, 1.0);
        }
        for (vertex, &point) in mesh.points.iter().enumerate() {
            if mesh.via_site[vertex] {
                self.circle(point, 0.06, "#bbb", "none", 0.0);
            }
        }
    }

    /// Topological wires through their crossing points; `layer_of(wire,
    /// step)` colours a stretch by the layer it is on, if known.
    pub fn wires(&mut self, board: &Board, mesh: &Mesh, topology: &Topology, layer_of: &dyn Fn(usize, usize) -> Option<usize>) {
        for (wire, path) in topology.wires.iter().enumerate() {
            if !path.routed {
                self.polyline(&[path.from, path.to], "#f0f", 0.05, 0.6);
                continue;
            }
            let mut points = vec![path.from];
            for &portal in &path.portals {
                points.push(topology.place(board, mesh, wire, portal));
            }
            points.push(path.to);
            for (step, pair) in points.windows(2).enumerate() {
                let colour = match layer_of(wire, step) {
                    Some(layer) => layer_colour(layer).to_string(),
                    None => net_colour(path.net),
                };
                self.polyline(pair, &colour, 0.08, 0.9);
            }
        }
    }

    /// Routed copper and vias, then the violations as rings.
    pub fn copper(&mut self, routes: &[NetRoute], violations: &[Violation]) {
        for route in routes {
            for segment in &route.segments {
                self.polyline(&[segment.start, segment.end], layer_colour(segment.layer), segment.width, 0.55);
            }
            for via in &route.vias {
                self.circle(via.at, via.diameter / 2.0, "#e8c040", "#806010", 0.03);
            }
        }
        for violation in violations {
            self.circle(violation.at, 0.35, "none", "#f0f", 0.08);
        }
    }

    pub fn save(&self, name: &str) {
        let Some(directory) = directory() else {
            return;
        };
        let _ = std::fs::create_dir_all(&directory);
        let margin = 1.0;
        let (x, y) = (self.minimum[0] - margin, self.minimum[1] - margin);
        let (width, height) = (self.maximum[0] - self.minimum[0] + 2.0 * margin, self.maximum[1] - self.minimum[1] + 2.0 * margin);
        let pixels = 2400.0;
        let text = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{x:.3} {y:.3} {width:.3} {height:.3}" width="{pixels}" height="{:.0}"><rect x="{x:.3}" y="{y:.3}" width="{width:.3}" height="{height:.3}" fill="white"/>{}</svg>"#,
            pixels * height / width,
            self.body
        );
        let _ = std::fs::write(directory.join(format!("{name}.svg")), text);
    }
}
