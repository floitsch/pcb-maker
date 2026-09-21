use crate::{Bounds, Vec2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticleRole {
    Terminal,
    Trace,
    Component,
    Via,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ParticleView {
    pub id: u32,
    pub label: String,
    pub position: Vec2,
    pub role: ParticleRole,
    pub field_weight: f32,
    pub inverse_mass: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BodyView {
    pub id: u32,
    pub label: String,
    pub position: Vec2,
    pub angle_radians: f32,
    pub half_size: Vec2,
    pub inverse_mass: f32,
    pub inverse_inertia: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ConstraintView {
    pub label: String,
    pub family: String,
    pub first: u32,
    pub second: u32,
    pub residual: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AttachmentView {
    pub label: String,
    pub particle: u32,
    pub body: u32,
    pub target: Vec2,
    pub residual: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct VectorView {
    pub particle: u32,
    pub origin: Vec2,
    pub vector: Vec2,
    pub source: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FieldView {
    pub width: usize,
    pub height: usize,
    pub values: Vec<f32>,
    pub max_value: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FrameMetrics {
    pub max_constraint_residual: f32,
    pub max_field_pressure: f32,
    pub max_displacement: f32,
    pub max_body_displacement: f32,
    pub max_body_rotation_radians: f32,
    pub constraint_projections: u64,
    pub constraint_scalar_rows: u64,
    #[serde(default)]
    pub trace_tension_edges: u64,
    pub field_cells: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Frame {
    pub step: u64,
    pub bounds: Bounds,
    pub particles: Vec<ParticleView>,
    pub bodies: Vec<BodyView>,
    pub constraints: Vec<ConstraintView>,
    pub attachments: Vec<AttachmentView>,
    pub vectors: Vec<VectorView>,
    pub field: Option<FieldView>,
    pub metrics: FrameMetrics,
}
