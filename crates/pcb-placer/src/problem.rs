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
    /// Artwork without pads or courtyard (a logo): it occupies nothing.
    Neither,
}

impl Side {
    /// Front against back: only far-side boxes can meet.
    pub fn opposite(self, other: Side) -> bool {
        matches!((self, other), (Side::Front, Side::Back) | (Side::Back, Side::Front))
    }

    pub fn collides(self, other: Side) -> bool {
        if self == Side::Neither || other == Side::Neither {
            return false;
        }
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
    /// Boxes ([min x, min y, max x, max y], own frame) a through-hole part
    /// occupies on the other side of the board: its holes and plated pads.
    /// The rest of its body leaves that side free.
    pub far_side: Vec<[f64; 4]>,
    /// Boxes ([min x, min y, max x, max y], own frame) that block the
    /// part's own side in place of its body: a hollow part (a shield's
    /// outline around its header pads) lets other parts sit inside it.
    /// Empty: the whole body blocks.
    pub hollow: Vec<[f64; 4]>,
    /// The body without the courtyard's margin ([min x, min y, max x, max
    /// y], own frame: fabrication outline and pads), for boards whose rules
    /// let courtyards overlap. Used only when nothing else fits.
    pub tight: Option<[f64; 4]>,
    /// How far the part's copper stays inside its body on every side: the
    /// body may come that much closer to the board edge than the copper.
    pub edge_inset: f64,
    /// Boxes ([min x, min y, max x, max y], own frame) of the separate
    /// shapes of a courtyard drawn as several (a board outline footprint
    /// marking only its connectors); empty when it is one shape.
    pub courtyards: Vec<[f64; 4]>,
    /// Other parts' holes may lie inside the courtyard: KiCad cannot build
    /// it (a malformed outline) and does not check it.
    pub holes_inside: bool,
    /// The part's pads (and holes), [min x, min y, max x, max y] in its own
    /// frame.
    pub pads: Vec<[f64; 4]>,
    /// A hole in the board (a cutout): it keeps other parts' copper an edge
    /// margin away, while their bodies may reach over it (a connector's
    /// pegs, a shield tab).
    pub copper_only: bool,
    /// A cutout's own outline in board coordinates (its body is the
    /// bounding box): copper keeps the edge margin from the hole, not from
    /// the box (a long diagonal cutout's box covers much more board).
    pub cutout_outline: Vec<Point>,
    /// `hollow` at the tight level: a channel's members by their tight
    /// bodies (own frame), used when the tight body is.
    pub tight_hollow: Vec<[f64; 4]>,
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
    /// Spacing never relaxed below this: pads may sit on a body's edge, so
    /// bodies closer than the copper clearance put pads too close.
    pub min_spacing: f64,
    pub constraints: crate::constraints::Constraints,
    /// Another part's pads on the far side (a through-hole part's pins, a
    /// hot-swap socket's pads) keep clear of this part's pads only, not of
    /// its whole body: set at the tight levels when the caller allows it.
    pub far_side_pads_only: bool,
    /// Bounding boxes of the board's pieces when it has several (two boards
    /// side by side): a part held at an edge is held at its piece's edge.
    pub pieces: Vec<[f64; 4]>,
}

impl Problem {
    /// Moves every follower (a row's parts) to where its leader carries it.
    pub fn sync_followers(&self, poses: &mut [Pose]) {
        for follower in &self.constraints.followers {
            let leader = poses[follower.leader];
            let offset = rotate(follower.offset, -leader.angle);
            poses[follower.part] = Pose {
                position: [leader.position[0] + offset[0], leader.position[1] + offset[1]],
                angle: (leader.angle + follower.angle).rem_euclid(360.0),
            };
        }
    }
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

    /// Board-space boxes (centre, half extent) of `far_side` at a pose.
    pub fn far_boxes(&self, pose: Pose) -> Vec<(Point, Point)> {
        self.boxes(&self.far_side, pose)
    }

    /// Board-space boxes (centre, half extent) of the pads at a pose.
    pub fn pad_boxes(&self, pose: Pose) -> Vec<(Point, Point)> {
        self.boxes(&self.pads, pose)
    }

    /// Board-space boxes (centre, half extent) of `hollow` at a pose.
    pub fn hollow_boxes(&self, pose: Pose) -> Vec<(Point, Point)> {
        self.boxes(&self.hollow, pose)
    }

    /// How far the body stays from the board edge for the copper to keep
    /// `margin` (the problem's edge margin); negative when the copper sits
    /// deeper inside the body than that: the body may then overhang the
    /// edge (a header wider than a narrow board).
    pub fn edge_margin(&self, margin: f64) -> f64 {
        margin - self.edge_inset
    }

    /// Switches the body to the tight one, if the part has one.
    pub fn use_tight_body(&mut self) {
        if let Some(tight) = self.tight {
            // The tight body holds the pads at its edges.
            self.edge_inset = 0.0;
            self.body_center = [(tight[0] + tight[2]) / 2.0, (tight[1] + tight[3]) / 2.0];
            self.body_size = [tight[2] - tight[0], tight[3] - tight[1]];
            // A disc stays a disc (a radial capacitor's outline).
            self.round = self.round && (self.body_size[0] - self.body_size[1]).abs() < 1.0e-6;
            if !self.tight_hollow.is_empty() {
                self.hollow = self.tight_hollow.clone();
            }
        }
    }

    /// Whether the part has holes (a through-hole part, a mounting hole).
    /// Holes may not lie inside another part's courtyard, even a hollow
    /// one's (KiCad's `pth_inside_courtyard`).
    pub fn has_holes(&self) -> bool {
        !self.far_side.is_empty()
    }

    /// Whether the part blocks `other` with boxes rather than its whole
    /// body: a hollow part with its pads, or, against a part with holes, its
    /// pads and its courtyard's separate shapes (holes may not lie inside a
    /// courtyard).
    pub fn hollow_for(&self, other: &Component) -> bool {
        !self.hollow.is_empty() && (!other.has_holes() || self.holes_inside || self.courtyards.len() > 1)
    }

    /// Board-space boxes (centre, half extent) the part blocks `other`
    /// with; see `hollow_for`.
    pub fn blocking_boxes(&self, other: &Component, pose: Pose) -> Vec<(Point, Point)> {
        let mut boxes = self.boxes(&self.hollow, pose);
        if other.has_holes() && !self.holes_inside {
            boxes.extend(self.boxes(&self.courtyards, pose));
        }
        boxes
    }

    /// The area the part takes on its own side.
    pub fn blocking_area(&self) -> f64 {
        if self.hollow.is_empty() {
            self.body_size[0] * self.body_size[1]
        } else {
            self.hollow.iter().map(|b| (b[2] - b[0]) * (b[3] - b[1])).sum()
        }
    }

    fn boxes(&self, boxes: &[[f64; 4]], pose: Pose) -> Vec<(Point, Point)> {
        boxes
            .iter()
            .map(|local| {
                let center = self.offset([(local[0] + local[2]) / 2.0, (local[1] + local[3]) / 2.0], pose.angle);
                let half = [(local[2] - local[0]) / 2.0, (local[3] - local[1]) / 2.0];
                let (sin, cos) = (-pose.angle).to_radians().sin_cos();
                (
                    [pose.position[0] + center[0], pose.position[1] + center[1]],
                    [
                        cos.abs() * half[0] + sin.abs() * half[1],
                        sin.abs() * half[0] + cos.abs() * half[1],
                    ],
                )
            })
            .collect()
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
