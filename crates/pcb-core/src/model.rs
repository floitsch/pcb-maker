use crate::{Bounds, Vec2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Board {
    pub bounds: Bounds,
    pub layers: Vec<Layer>,
    pub rules: Rules,
    pub components: Vec<Component>,
    pub connections: Vec<Connection>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Layer {
    pub id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Rules {
    pub clearance: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Component {
    pub id: String,
    pub position: Vec2,
    #[serde(default)]
    pub rotation_radians: f32,
    pub kind: ComponentKind,
    pub mobility: Mobility,
    pub terminals: Vec<Terminal>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComponentKind {
    Anchor,
    TwoTerminalBody { length: f32, radius: f32 },
    Rect { size: Vec2, routing_keepout: bool },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Mobility {
    pub inverse_mass: f32,
    pub rotation_mobility: f32,
    pub translate_x: bool,
    pub translate_y: bool,
    pub rotate: bool,
}

impl Mobility {
    pub const FIXED: Self = Self {
        inverse_mass: 0.0,
        rotation_mobility: 0.0,
        translate_x: false,
        translate_y: false,
        rotate: false,
    };

    pub const FREE: Self = Self {
        inverse_mass: 1.0,
        rotation_mobility: 1.0,
        translate_x: true,
        translate_y: true,
        rotate: true,
    };
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Terminal {
    pub id: String,
    pub local_position: Vec2,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TerminalRef {
    pub component: String,
    pub terminal: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Connection {
    pub id: String,
    pub terminals: Vec<TerminalRef>,
    pub layer: usize,
    pub width: f32,
    pub seed_route: Vec<Vec2>,
}

/// Representation-independent state emitted by a continuous engine. Solver
/// handles, particles, constraints, and body buffers do not cross this seam.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContinuousSolution {
    pub components: Vec<ContinuousComponentPose>,
    pub connections: Vec<ContinuousConnection>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContinuousComponentPose {
    pub id: String,
    pub position: Vec2,
    pub rotation_radians: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContinuousConnection {
    pub id: String,
    pub layer: usize,
    pub width: f32,
    pub points: Vec<Vec2>,
}

impl Board {
    pub fn validate(&self) -> Result<(), String> {
        if self.layers.is_empty() {
            return Err("a board needs at least one copper layer".into());
        }
        if self.rules.clearance < 0.0 {
            return Err("clearance cannot be negative".into());
        }
        for component in &self.components {
            if component.id.is_empty() {
                return Err("component IDs cannot be empty".into());
            }
            if component.mobility.inverse_mass < 0.0 || component.mobility.rotation_mobility < 0.0 {
                return Err(format!("{} has negative mobility", component.id));
            }
            if !component.position.x.is_finite()
                || !component.position.y.is_finite()
                || !component.rotation_radians.is_finite()
            {
                return Err(format!("{} has a non-finite pose", component.id));
            }
            match component.kind {
                ComponentKind::TwoTerminalBody { length, radius }
                    if !length.is_finite()
                        || !radius.is_finite()
                        || length <= 0.0
                        || radius <= 0.0 =>
                {
                    return Err(format!(
                        "{} has invalid two-terminal dimensions",
                        component.id
                    ));
                }
                ComponentKind::Rect { size, .. }
                    if !size.x.is_finite()
                        || !size.y.is_finite()
                        || size.x <= 0.0
                        || size.y <= 0.0 =>
                {
                    return Err(format!(
                        "{} has invalid rectangular dimensions",
                        component.id
                    ));
                }
                _ => {}
            }
        }
        for connection in &self.connections {
            if connection.terminals.len() < 2 {
                return Err(format!("{} needs at least two terminals", connection.id));
            }
            if connection.layer >= self.layers.len() {
                return Err(format!("{} uses an unknown layer", connection.id));
            }
            if connection.width <= 0.0 {
                return Err(format!("{} has non-positive width", connection.id));
            }
        }
        Ok(())
    }
}
