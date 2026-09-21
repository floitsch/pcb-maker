// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The placement problem, independent of any board file format.

pub type Point = [f64; 2];

/// Which board sides a component's body occupies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Front,
    Back,
    /// Through-hole parts and mechanical features block both sides.
    Both,
}

impl Side {
    pub fn collides(self, other: Side) -> bool {
        self == Side::Both || other == Side::Both || self == other
    }
}

#[derive(Clone, Debug)]
pub struct Pin {
    /// Position in the component's own unrotated frame.
    pub offset: Point,
    pub net: usize,
}

#[derive(Clone, Debug)]
pub struct Component {
    pub name: String,
    /// Body (courtyard) rectangle in the component's unrotated frame.
    pub body_center: Point,
    pub body_size: Point,
    /// The body is a disc of diameter `body_size[0]` instead of a rectangle.
    pub round: bool,
    /// Extra room kept free around the body for escape routing. It counts
    /// as body area for density and adds to the spacing towards neighbours.
    pub halo: f64,
    pub pins: Vec<Pin>,
    pub side: Side,
    pub fixed: bool,
    /// Angles the placer may choose from, in degrees.
    pub angle_options: Vec<f64>,
}

/// Position of the component origin and its rotation in degrees. Following
/// KiCad, a local point maps to `position + rotate(local, -angle)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub position: Point,
    pub angle: f64,
}

#[derive(Clone, Debug)]
pub struct Problem {
    pub outline: Vec<Point>,
    pub components: Vec<Component>,
    /// One weight per net; pins refer to nets by index.
    pub net_weights: Vec<f64>,
    /// Initial poses; fixed components keep theirs.
    pub poses: Vec<Pose>,
    /// Minimum gap between two bodies on colliding sides.
    pub spacing: f64,
    /// Component origins are snapped to multiples of this.
    pub grid: f64,
    /// Movable bodies keep this distance from the board edge.
    pub edge_margin: f64,
}

pub fn rotate(local: Point, angle_degrees: f64) -> Point {
    // Exact for the common quarter turns.
    let quarter = (angle_degrees / 90.0).round();
    let (sin, cos) = if (angle_degrees - quarter * 90.0).abs() < 1.0e-9 {
        match (quarter as i64).rem_euclid(4) {
            0 => (0.0, 1.0),
            1 => (1.0, 0.0),
            2 => (0.0, -1.0),
            _ => (-1.0, 0.0),
        }
    } else {
        angle_degrees.to_radians().sin_cos()
    };
    [
        local[0] * cos - local[1] * sin,
        local[0] * sin + local[1] * cos,
    ]
}

impl Component {
    /// Board-space offset of a local point for a rotation.
    pub fn offset(&self, local: Point, angle: f64) -> Point {
        rotate(local, -angle)
    }

    /// Axis-aligned half extents of the body at `angle`.
    pub fn half_extent(&self, angle: f64) -> Point {
        let (sin, cos) = (-angle).to_radians().sin_cos();
        let half = [self.body_size[0] / 2.0, self.body_size[1] / 2.0];
        [
            (cos.abs() * half[0] + sin.abs() * half[1]),
            (sin.abs() * half[0] + cos.abs() * half[1]),
        ]
    }

    /// Board-space body centre for a pose.
    pub fn center(&self, pose: Pose) -> Point {
        let offset = self.offset(self.body_center, pose.angle);
        [pose.position[0] + offset[0], pose.position[1] + offset[1]]
    }

    pub fn position_for_center(&self, center: Point, angle: f64) -> Point {
        let offset = self.offset(self.body_center, angle);
        [center[0] - offset[0], center[1] - offset[1]]
    }

    pub fn pin_position(&self, pin: &Pin, pose: Pose) -> Point {
        let offset = self.offset(pin.offset, pose.angle);
        [pose.position[0] + offset[0], pose.position[1] + offset[1]]
    }
}

impl Problem {
    pub fn bounds(&self) -> [f64; 4] {
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for point in &self.outline {
            bounds[0] = bounds[0].min(point[0]);
            bounds[1] = bounds[1].min(point[1]);
            bounds[2] = bounds[2].max(point[0]);
            bounds[3] = bounds[3].max(point[1]);
        }
        bounds
    }

    /// Weighted half-perimeter wirelength with true pin positions.
    pub fn wirelength(&self, poses: &[Pose]) -> f64 {
        let mut boxes = vec![
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY
            ];
            self.net_weights.len()
        ];
        for (component, pose) in self.components.iter().zip(poses) {
            for pin in &component.pins {
                let at = component.pin_position(pin, *pose);
                let bounds = &mut boxes[pin.net];
                bounds[0] = bounds[0].min(at[0]);
                bounds[1] = bounds[1].min(at[1]);
                bounds[2] = bounds[2].max(at[0]);
                bounds[3] = bounds[3].max(at[1]);
            }
        }
        boxes
            .iter()
            .zip(&self.net_weights)
            .filter(|(bounds, _)| bounds[0].is_finite())
            .map(|(bounds, weight)| weight * ((bounds[2] - bounds[0]) + (bounds[3] - bounds[1])))
            .sum()
    }
}

pub fn point_in_polygon(point: Point, polygon: &[Point]) -> bool {
    let mut inside = false;
    for index in 0..polygon.len() {
        let a = polygon[index];
        let b = polygon[(index + 1) % polygon.len()];
        if (a[1] > point[1]) != (b[1] > point[1]) {
            let x = a[0] + (point[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if point[0] < x {
                inside = !inside;
            }
        }
    }
    inside
}
