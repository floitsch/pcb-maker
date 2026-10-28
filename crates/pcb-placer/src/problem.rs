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
        matches!(
            (self, other),
            (Side::Front, Side::Back) | (Side::Back, Side::Front)
        )
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
    /// Copper the part has on the other side that blocks only copper there
    /// (own frame): pads of a part without leads through the board (an SMD
    /// part's pads and vias on both sides, a non-plated hole with its
    /// clearance ring), copper and mask graphics. KiCad checks them against
    /// other copper, not against courtyards, so they keep the copper
    /// clearance from the other side's pads, not from its bodies.
    pub far_copper: Vec<[f64; 4]>,
    /// The part's courtyard on the other side, where it has one on both
    /// ([min x, min y, max x, max y], own frame; `far_side` holds it too):
    /// a back-mounted hot-swap switch's 14 mm square on the front. KiCad
    /// checks it against that side's courtyards and holes, including
    /// those of parts on the part's own side (two back switches' front
    /// squares may not overlap).
    pub far_courtyard: Vec<[f64; 4]>,
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
    /// The non-plated holes with the clearance ring they keep from other
    /// parts' copper beyond the copper clearance (own frame): another
    /// part's pads keep the copper clearance from them at every level,
    /// also where the bodies may come closer than the hole clearance (a
    /// mouse bite's 0.5 mm holes in a 0.7 mm courtyard: OpenRX's pad came
    /// 0.19 mm to a hole where the project asks 0.2).
    pub hole_rings: Vec<[f64; 4]>,
    /// The footprint's copper graphics (own frame, boxes with their
    /// stroke): an antenna's meander, a logo on copper. KiCad 10.0.7's
    /// copper_edge_clearance checks them against every edge, a cutout's
    /// too, as it checks pads (osprey's AE1: its B.Cu polygon over the
    /// battery cutout's edge, 0.0 mm against 0.25).
    pub copper_graphics: Vec<[f64; 4]>,
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
    /// Two parts on one side keep the larger of their routing halos
    /// between them, not the sum: the channel between neighbours carries
    /// the escapes of both (two 0603s 1.8 mm apart at a 0.65 mm halo
    /// each, where designers put them side by side).
    pub shared_halos: bool,
    /// Bounding boxes of the board's pieces when it has several (two boards
    /// side by side): a part held at an edge is held at its piece's edge.
    pub pieces: Vec<[f64; 4]>,
    /// Escape demand (`crate::escape`, an experiment): where parts' tracks
    /// leave their bodies and whether the channels between parts carry
    /// them. `None`: off.
    pub escape: Option<std::sync::Arc<crate::escape::Escape>>,
    /// Ratsnest crossings (`crate::crossings`, on by default): the
    /// legaliser and the refinement pay for the crossings of the moved
    /// parts' nets' spanning trees. `None`: off.
    pub crossings: Option<std::sync::Arc<crate::crossings::Crossings>>,
}

impl Problem {
    /// At KiCad's legality: the board's marks that are no obstacle there
    /// (`Constraints::passable_marks`) take no side.
    pub fn pass_marks(&mut self) {
        for &index in &self.constraints.passable_marks {
            if let Some(component) = self.components.get_mut(index) {
                component.side = Side::Neither;
            }
        }
    }

    /// Moves every follower (a row's parts) to where its leader carries it.
    pub fn sync_followers(&self, poses: &mut [Pose]) {
        for follower in &self.constraints.followers {
            let leader = poses[follower.leader];
            let offset = rotate(follower.offset, -leader.angle);
            poses[follower.part] = Pose {
                position: [
                    leader.position[0] + offset[0],
                    leader.position[1] + offset[1],
                ],
                angle: (leader.angle + follower.angle).rem_euclid(360.0),
            };
        }
    }
}

pub fn rotate(local: Point, angle_degrees: f64) -> Point {
    rotate_by(local, rotation(angle_degrees))
}

/// The (sin, cos) `rotate` turns by: exact for the quarter turns.
pub fn rotation(angle_degrees: f64) -> (f64, f64) {
    // The quarter turns parts sit at, without the division and the
    // rounding below (`round` is a call into libm): the same answers.
    if angle_degrees == 0.0 || angle_degrees == 360.0 || angle_degrees == -360.0 {
        return (0.0, 1.0);
    }
    if angle_degrees == 90.0 || angle_degrees == -270.0 {
        return (1.0, 0.0);
    }
    if angle_degrees == 180.0 || angle_degrees == -180.0 {
        return (0.0, -1.0);
    }
    if angle_degrees == 270.0 || angle_degrees == -90.0 {
        return (-1.0, 0.0);
    }
    // Exact for the common quarter turns.
    let quarter = (angle_degrees / 90.0).round();
    if (angle_degrees - quarter * 90.0).abs() < 1.0e-9 {
        match (quarter as i64).rem_euclid(4) {
            0 => (0.0, 1.0),
            1 => (1.0, 0.0),
            2 => (0.0, -1.0),
            _ => (-1.0, 0.0),
        }
    } else {
        sin_cos_degrees(angle_degrees)
    }
}

/// `local` turned by a `rotation`.
pub fn rotate_by(local: Point, (sin, cos): (f64, f64)) -> Point {
    [
        local[0] * cos - local[1] * sin,
        local[0] * sin + local[1] * cos,
    ]
}

/// The angles `sin_cos_degrees` looks up, the most frequent first.
const TABLED_ANGLES: [f64; 18] = [
    -0.0, 0.0, -90.0, 90.0, -180.0, 180.0, -270.0, 270.0, -45.0, 45.0, -135.0, 135.0, -225.0,
    225.0, -315.0, 315.0, -360.0, 360.0,
];

/// `angle_degrees.to_radians().sin_cos()`, bit for bit. The quarter and
/// eighth turns (either sign, either zero) come from a table that the same
/// expression filled once: the placer turned every box and half extent
/// with a call into libm (3 to 6 % of a layout's CPU, 2026-10-25).
pub fn sin_cos_degrees(angle_degrees: f64) -> (f64, f64) {
    static TABLE: std::sync::OnceLock<[(u64, (f64, f64)); TABLED_ANGLES.len()]> =
        std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        TABLED_ANGLES.map(|angle| {
            // Computed at run time as at every call site, never folded at
            // compile time by another libm.
            let angle = std::hint::black_box(angle);
            (angle.to_bits(), angle.to_radians().sin_cos())
        })
    });
    let bits = angle_degrees.to_bits();
    for (key, value) in table {
        if *key == bits {
            return *value;
        }
    }
    angle_degrees.to_radians().sin_cos()
}

impl Component {
    /// Board-space offset of a local point for a rotation.
    pub fn offset(&self, local: Point, angle: f64) -> Point {
        rotate(local, -angle)
    }

    /// Axis-aligned half extents of the body at `angle`.
    pub fn half_extent(&self, angle: f64) -> Point {
        let (sin, cos) = sin_cos_degrees(-angle);
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

    /// Board-space boxes (centre, half extent) of `far_side` at a pose
    /// (`far_copper` meets only copper: the legality test's business).
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

    /// The room this body keeps around itself at the courtyard-spacing
    /// level: half the copper clearance less how far its copper stays
    /// inside it, and a hair at least (KiCad's courtyard test).
    pub fn copper_share(&self, min_spacing: f64) -> f64 {
        let inset = if self.round { 0.0 } else { self.edge_inset };
        (min_spacing / 2.0 - inset).max(0.005)
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
        !self.far_side.is_empty() || !self.far_copper.is_empty()
    }

    /// Whether the part blocks `other` with boxes rather than its whole
    /// body: a hollow part with its pads, or, against a part with holes, its
    /// pads and its courtyard's separate shapes (holes may not lie inside a
    /// courtyard).
    pub fn hollow_for(&self, other: &Component) -> bool {
        !self.hollow.is_empty()
            && (!other.has_holes() || self.holes_inside || self.courtyards.len() > 1)
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
            self.hollow
                .iter()
                .map(|b| (b[2] - b[0]) * (b[3] - b[1]))
                .sum()
        }
    }

    /// Board-space boxes (centre, half extent) of boxes of the part's own
    /// frame at a pose.
    pub fn boxes_of(&self, boxes: &[[f64; 4]], pose: Pose) -> Vec<(Point, Point)> {
        self.boxes(boxes, pose)
    }

    fn boxes(&self, boxes: &[[f64; 4]], pose: Pose) -> Vec<(Point, Point)> {
        let mut out = Vec::with_capacity(boxes.len());
        self.boxes_into(boxes, pose, &mut out);
        out
    }

    /// `boxes_of` into `out` (cleared first): the same boxes, without a
    /// new list.
    pub fn boxes_into(&self, boxes: &[[f64; 4]], pose: Pose, out: &mut Vec<(Point, Point)>) {
        out.clear();
        // The pose's turns once for all its boxes (`offset` and the extent).
        let turn = rotation(-pose.angle);
        let (sin, cos) = sin_cos_degrees(-pose.angle);
        out.extend(boxes.iter().map(|local| {
            let center = rotate_by(
                [(local[0] + local[2]) / 2.0, (local[1] + local[3]) / 2.0],
                turn,
            );
            let half = [(local[2] - local[0]) / 2.0, (local[3] - local[1]) / 2.0];
            (
                [pose.position[0] + center[0], pose.position[1] + center[1]],
                [
                    cos.abs() * half[0] + sin.abs() * half[1],
                    sin.abs() * half[0] + cos.abs() * half[1],
                ],
            )
        }));
    }

    pub fn pin_position(&self, pin: &Pin, pose: Pose) -> Point {
        let offset = self.offset(pin.offset, pose.angle);
        [pose.position[0] + offset[0], pose.position[1] + offset[1]]
    }
}

impl Problem {
    /// How far any part's far-side boxes reach beyond its body, at any
    /// angle: a neighbour search around a body must look that much farther
    /// (a hot-swap switch on the back: the switch's courtyard on the front
    /// is larger than the socket's on the back).
    /// The extra reach a neighbour search needs where two parts on one
    /// side may meet with their courtyards on the other side
    /// (`Component::far_courtyard`): both reach beyond their bodies, so
    /// `far_reach` once more; nothing on boards without such parts.
    pub fn far_courtyard_reach(&self) -> f64 {
        if self
            .components
            .iter()
            .any(|component| !component.far_courtyard.is_empty())
        {
            self.far_reach()
        } else {
            0.0
        }
    }

    pub fn far_reach(&self) -> f64 {
        self.components
            .iter()
            .flat_map(|component| {
                let shortest = component.body_size[0].min(component.body_size[1]) / 2.0;
                // Hollow and shaped boxes too: a part without a courtyard
                // blocks with copper graphics beyond its body (WLED_Matrix).
                component
                    .far_side
                    .iter()
                    .chain(&component.far_copper)
                    .chain(&component.hollow)
                    .chain(&component.courtyards)
                    .chain(&component.tight_hollow)
                    .map(move |far| {
                        let farthest = [
                            [far[0], far[1]],
                            [far[2], far[1]],
                            [far[2], far[3]],
                            [far[0], far[3]],
                        ]
                        .iter()
                        .map(|corner| {
                            (corner[0] - component.body_center[0])
                                .hypot(corner[1] - component.body_center[1])
                        })
                        .fold(0.0, f64::max);
                        // Inside the body at every angle: no reach.
                        let inside = far[0]
                            >= component.body_center[0] - component.body_size[0] / 2.0
                            && far[1] >= component.body_center[1] - component.body_size[1] / 2.0
                            && far[2] <= component.body_center[0] + component.body_size[0] / 2.0
                            && far[3] <= component.body_center[1] + component.body_size[1] / 2.0;
                        if inside {
                            0.0
                        } else {
                            (farthest - shortest).max(0.0)
                        }
                    })
            })
            .fold(0.0, f64::max)
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    /// `rotate` before the table and the quarter-turn shortcut.
    fn rotate_before(local: Point, angle_degrees: f64) -> Point {
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

    fn angles() -> Vec<f64> {
        let mut angles: Vec<f64> = TABLED_ANGLES.to_vec();
        angles.extend((-1440..=1440).map(|step| step as f64 * 0.25));
        angles.extend([
            30.0,
            -30.0,
            1.0e-10,
            -1.0e-10,
            90.0 + 1.0e-10,
            359.9999999,
            720.0,
        ]);
        angles
    }

    #[test]
    fn the_table_gives_the_bits_of_the_call() {
        for angle in angles() {
            let (sin, cos) = sin_cos_degrees(angle);
            let (want_sin, want_cos) = std::hint::black_box(angle).to_radians().sin_cos();
            assert_eq!(sin.to_bits(), want_sin.to_bits(), "sin at {angle}");
            assert_eq!(cos.to_bits(), want_cos.to_bits(), "cos at {angle}");
        }
    }

    #[test]
    fn rotate_gives_the_bits_it_gave_before() {
        for angle in angles() {
            for local in [[1.25, -0.5], [-3.0, 7.125], [0.0, -0.0], [-0.0, 2.0]] {
                let now = rotate(local, angle);
                let before = rotate_before(local, angle);
                assert_eq!(
                    now[0].to_bits(),
                    before[0].to_bits(),
                    "{local:?} at {angle}"
                );
                assert_eq!(
                    now[1].to_bits(),
                    before[1].to_bits(),
                    "{local:?} at {angle}"
                );
            }
        }
    }
}
