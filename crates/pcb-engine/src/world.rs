use pcb_core::{Bounds, ParticleRole, Vec2};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MobilityMask(u8);

impl MobilityMask {
    pub const FIXED: Self = Self(0);
    pub const X: Self = Self(1);
    pub const Y: Self = Self(2);
    pub const XY: Self = Self(3);

    pub const fn from_axes(x: bool, y: bool) -> Self {
        match (x, y) {
            (false, false) => Self::FIXED,
            (true, false) => Self::X,
            (false, true) => Self::Y,
            (true, true) => Self::XY,
        }
    }

    pub fn allows_x(self) -> bool {
        self.0 & Self::X.0 != 0
    }

    pub fn allows_y(self) -> bool {
        self.0 & Self::Y.0 != 0
    }

    pub fn project(self, value: Vec2) -> Vec2 {
        Vec2::new(
            if self.allows_x() { value.x } else { 0.0 },
            if self.allows_y() { value.y } else { 0.0 },
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct ParticleSoa {
    pub position_x: Vec<f32>,
    pub position_y: Vec<f32>,
    pub previous_x: Vec<f32>,
    pub previous_y: Vec<f32>,
    pub field_weight: Vec<f32>,
    pub inverse_mass: Vec<f32>,
    pub mobility: Vec<MobilityMask>,
    pub role: Vec<ParticleRole>,
    pub stable_id: Vec<u32>,
    pub label: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ParticleInit {
    pub position: Vec2,
    pub field_weight: f32,
    pub inverse_mass: f32,
    pub mobility: MobilityMask,
    pub role: ParticleRole,
    pub stable_id: u32,
    pub label: String,
}

impl ParticleSoa {
    pub fn len(&self) -> usize {
        self.position_x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.position_x.is_empty()
    }

    pub fn push(&mut self, particle: ParticleInit) -> u32 {
        let index = self.len() as u32;
        self.position_x.push(particle.position.x);
        self.position_y.push(particle.position.y);
        self.previous_x.push(particle.position.x);
        self.previous_y.push(particle.position.y);
        self.field_weight.push(particle.field_weight);
        self.inverse_mass.push(particle.inverse_mass);
        self.mobility.push(particle.mobility);
        self.role.push(particle.role);
        self.stable_id.push(particle.stable_id);
        self.label.push(particle.label);
        index
    }

    pub fn position(&self, index: usize) -> Vec2 {
        Vec2::new(self.position_x[index], self.position_y[index])
    }

    pub fn previous(&self, index: usize) -> Vec2 {
        Vec2::new(self.previous_x[index], self.previous_y[index])
    }

    pub fn set_position(&mut self, index: usize, position: Vec2) {
        self.position_x[index] = position.x;
        self.position_y[index] = position.y;
    }

    pub fn snapshot_previous(&mut self) {
        self.previous_x.clone_from(&self.position_x);
        self.previous_y.clone_from(&self.position_y);
    }
}

#[derive(Clone, Debug, Default)]
pub struct BodySoa {
    pub position_x: Vec<f32>,
    pub position_y: Vec<f32>,
    pub previous_x: Vec<f32>,
    pub previous_y: Vec<f32>,
    pub angle_radians: Vec<f32>,
    pub previous_angle_radians: Vec<f32>,
    pub half_width: Vec<f32>,
    pub half_height: Vec<f32>,
    pub inverse_mass: Vec<f32>,
    pub inverse_inertia: Vec<f32>,
    pub mobility: Vec<MobilityMask>,
    pub rotate: Vec<bool>,
    pub stable_id: Vec<u32>,
    pub label: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct BodyInit {
    pub position: Vec2,
    pub angle_radians: f32,
    pub half_size: Vec2,
    pub inverse_mass: f32,
    pub inverse_inertia: f32,
    pub mobility: MobilityMask,
    pub rotate: bool,
    pub stable_id: u32,
    pub label: String,
}

impl BodySoa {
    pub fn len(&self) -> usize {
        self.position_x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.position_x.is_empty()
    }

    pub fn push(&mut self, body: BodyInit) -> u32 {
        let index = self.len() as u32;
        self.position_x.push(body.position.x);
        self.position_y.push(body.position.y);
        self.previous_x.push(body.position.x);
        self.previous_y.push(body.position.y);
        self.angle_radians.push(body.angle_radians);
        self.previous_angle_radians.push(body.angle_radians);
        self.half_width.push(body.half_size.x);
        self.half_height.push(body.half_size.y);
        self.inverse_mass.push(body.inverse_mass);
        self.inverse_inertia.push(body.inverse_inertia);
        self.mobility.push(body.mobility);
        self.rotate.push(body.rotate);
        self.stable_id.push(body.stable_id);
        self.label.push(body.label);
        index
    }

    pub fn position(&self, index: usize) -> Vec2 {
        Vec2::new(self.position_x[index], self.position_y[index])
    }

    pub fn previous(&self, index: usize) -> Vec2 {
        Vec2::new(self.previous_x[index], self.previous_y[index])
    }

    pub fn set_position(&mut self, index: usize, position: Vec2) {
        self.position_x[index] = position.x;
        self.position_y[index] = position.y;
    }

    pub fn half_size(&self, index: usize) -> Vec2 {
        Vec2::new(self.half_width[index], self.half_height[index])
    }

    pub fn snapshot_previous(&mut self) {
        self.previous_x.clone_from(&self.position_x);
        self.previous_y.clone_from(&self.position_y);
        self.previous_angle_radians.clone_from(&self.angle_radians);
    }
}

#[derive(Clone, Debug, Default)]
pub struct AttachmentSoa {
    pub particle: Vec<u32>,
    pub body: Vec<u32>,
    pub local_x: Vec<f32>,
    pub local_y: Vec<f32>,
    pub label: Vec<String>,
}

impl AttachmentSoa {
    pub fn push(&mut self, particle: u32, body: u32, local: Vec2, label: String) {
        self.particle.push(particle);
        self.body.push(body);
        self.local_x.push(local.x);
        self.local_y.push(local.y);
        self.label.push(label);
    }

    pub fn len(&self) -> usize {
        self.particle.len()
    }

    pub fn is_empty(&self) -> bool {
        self.particle.is_empty()
    }

    pub fn local(&self, index: usize) -> Vec2 {
        Vec2::new(self.local_x[index], self.local_y[index])
    }
}

#[derive(Clone, Debug, Default)]
pub struct EqualityDistanceSoa {
    pub first: Vec<u32>,
    pub second: Vec<u32>,
    pub distance: Vec<f32>,
    pub label: Vec<String>,
}

impl EqualityDistanceSoa {
    pub fn push(&mut self, first: u32, second: u32, distance: f32, label: String) {
        self.first.push(first);
        self.second.push(second);
        self.distance.push(distance);
        self.label.push(label);
    }

    pub fn len(&self) -> usize {
        self.first.len()
    }

    pub fn is_empty(&self) -> bool {
        self.first.is_empty()
    }
}

#[derive(Clone, Debug, Default)]
pub struct MaximumDistanceSoa {
    pub first: Vec<u32>,
    pub second: Vec<u32>,
    pub maximum: Vec<f32>,
    pub label: Vec<String>,
}

/// Trace-length objective edges. Each row contributes equal and opposite
/// forces along one polyline segment before hard constraints project the
/// proposal back into the feasible set. The flat arrays are directly suitable
/// for a GPU edge kernel plus particle-delta reduction.
#[derive(Clone, Debug, Default)]
pub struct TraceTensionSoa {
    pub first: Vec<u32>,
    pub second: Vec<u32>,
    pub weight: Vec<f32>,
    pub label: Vec<String>,
}

impl TraceTensionSoa {
    pub fn push(&mut self, first: u32, second: u32, weight: f32, label: String) {
        self.first.push(first);
        self.second.push(second);
        self.weight.push(weight);
        self.label.push(label);
    }

    pub fn len(&self) -> usize {
        self.first.len()
    }

    pub fn is_empty(&self) -> bool {
        self.first.is_empty()
    }
}

/// Full-width copper separation between two line segments. The four particle
/// references deliberately remain flat SoA data so the same contract can be
/// uploaded to a GPU backend without representation-specific branching.
#[derive(Clone, Debug, Default)]
pub struct SegmentClearanceSoa {
    pub first_start: Vec<u32>,
    pub first_end: Vec<u32>,
    pub second_start: Vec<u32>,
    pub second_end: Vec<u32>,
    pub minimum: Vec<f32>,
    pub label: Vec<String>,
}

/// Full-width copper separation between a line segment and an oriented
/// rectangular body. Keeping the body handle and segment endpoints in flat
/// arrays makes this a separate data-parallel constraint family instead of a
/// component-type branch inside the solver.
#[derive(Clone, Debug, Default)]
pub struct SegmentBodyClearanceSoa {
    pub start: Vec<u32>,
    pub end: Vec<u32>,
    pub body: Vec<u32>,
    pub minimum: Vec<f32>,
    pub label: Vec<String>,
}

/// Separation between two oriented rectangular bodies. This stays a distinct
/// flat constraint family so a GPU backend can process body pairs without
/// branching on component kind.
#[derive(Clone, Debug, Default)]
pub struct BodyBodyClearanceSoa {
    pub first: Vec<u32>,
    pub second: Vec<u32>,
    /// Signed surface distance. Positive values require a gap; zero prevents
    /// overlap; negative values can preserve an intentional initial overlap.
    pub minimum: Vec<f32>,
    pub label: Vec<String>,
}

impl BodyBodyClearanceSoa {
    pub fn push(&mut self, first: u32, second: u32, minimum: f32, label: String) {
        self.first.push(first);
        self.second.push(second);
        self.minimum.push(minimum);
        self.label.push(label);
    }

    pub fn len(&self) -> usize {
        self.minimum.len()
    }

    pub fn is_empty(&self) -> bool {
        self.minimum.is_empty()
    }
}

impl SegmentBodyClearanceSoa {
    pub fn push(&mut self, segment: [u32; 2], body: u32, minimum: f32, label: String) {
        self.start.push(segment[0]);
        self.end.push(segment[1]);
        self.body.push(body);
        self.minimum.push(minimum);
        self.label.push(label);
    }

    pub fn len(&self) -> usize {
        self.minimum.len()
    }

    pub fn is_empty(&self) -> bool {
        self.minimum.is_empty()
    }
}

impl SegmentClearanceSoa {
    pub fn push(&mut self, first: [u32; 2], second: [u32; 2], minimum: f32, label: String) {
        self.first_start.push(first[0]);
        self.first_end.push(first[1]);
        self.second_start.push(second[0]);
        self.second_end.push(second[1]);
        self.minimum.push(minimum);
        self.label.push(label);
    }

    pub fn len(&self) -> usize {
        self.minimum.len()
    }

    pub fn is_empty(&self) -> bool {
        self.minimum.is_empty()
    }
}

impl MaximumDistanceSoa {
    pub fn push(&mut self, first: u32, second: u32, maximum: f32, label: String) {
        self.first.push(first);
        self.second.push(second);
        self.maximum.push(maximum);
        self.label.push(label);
    }

    pub fn len(&self) -> usize {
        self.first.len()
    }

    pub fn is_empty(&self) -> bool {
        self.first.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct Segment {
    pub first: u32,
    pub second: u32,
    pub layer: usize,
    pub width: f32,
    pub connection: String,
}

#[derive(Clone, Debug)]
pub struct World {
    pub bounds: Bounds,
    pub particles: ParticleSoa,
    pub bodies: BodySoa,
    pub attachments: AttachmentSoa,
    pub equality_distance: EqualityDistanceSoa,
    pub maximum_distance: MaximumDistanceSoa,
    pub trace_tension: TraceTensionSoa,
    pub segment_clearance: SegmentClearanceSoa,
    pub segment_body_clearance: SegmentBodyClearanceSoa,
    pub body_body_clearance: BodyBodyClearanceSoa,
    pub segments: Vec<Segment>,
}

impl World {
    pub fn new(bounds: Bounds) -> Self {
        Self {
            bounds,
            particles: ParticleSoa::default(),
            bodies: BodySoa::default(),
            attachments: AttachmentSoa::default(),
            equality_distance: EqualityDistanceSoa::default(),
            maximum_distance: MaximumDistanceSoa::default(),
            trace_tension: TraceTensionSoa::default(),
            segment_clearance: SegmentClearanceSoa::default(),
            segment_body_clearance: SegmentBodyClearanceSoa::default(),
            body_body_clearance: BodyBodyClearanceSoa::default(),
            segments: Vec::new(),
        }
    }
}
