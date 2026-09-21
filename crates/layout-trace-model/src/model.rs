use serde::{Deserialize, Deserializer, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    pub fn width(self) -> f64 {
        self.max.x - self.min.x
    }

    pub fn height(self) -> f64 {
        self.max.y - self.min.y
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Problem {
    pub schema_version: u32,
    pub board: Board,
    pub rules: Rules,
    pub components: Vec<Component>,
    /// Legacy two-terminal route declarations. They compile to one route-graph
    /// branch and remain supported as a compact compatibility syntax.
    #[serde(default)]
    pub nets: Vec<Net>,
    /// Electrical connectivity goals with any number of terminals. The model
    /// compiler chooses an initial terminal-anchored tree; later topology
    /// passes may introduce movable junction nodes.
    #[serde(default)]
    pub electrical_nets: Vec<ElectricalNet>,
    /// Pose-relative placement requirements. Unlike component-local board
    /// regions, these follow both components as the engine moves them.
    #[serde(default)]
    pub placement_constraints: Vec<PlacementConstraint>,
    #[serde(default)]
    pub solver: SolverConfig,
}

impl Problem {
    pub fn check_schema(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "unsupported schema_version {}; expected {}",
                self.schema_version, SCHEMA_VERSION
            ));
        }
        if self.board.bounds.width() <= 0.0 || self.board.bounds.height() <= 0.0 {
            return Err("board bounds must have positive width and height".into());
        }
        if self.board.layers.is_empty() {
            return Err("board must declare at least one copper layer".into());
        }
        let mut layer_ids = std::collections::HashSet::new();
        if self
            .board
            .layers
            .iter()
            .any(|layer| layer.id.is_empty() || !layer_ids.insert(layer.id.as_str()))
        {
            return Err("copper layer IDs must be non-empty and unique".into());
        }
        if self.rules.clearance < 0.0 {
            return Err("clearance must not be negative".into());
        }
        if self.rules.via_drill <= 0.0
            || self.rules.via_diameter <= 0.0
            || self.rules.via_drill > self.rules.via_diameter
        {
            return Err(
                "via dimensions must be positive and drill must not exceed diameter".into(),
            );
        }
        if !self.solver.clearance_influence.is_finite() || self.solver.clearance_influence < 0.0 {
            return Err("solver clearance_influence must be finite and non-negative".into());
        }
        let mut component_ids = std::collections::HashSet::new();
        for component in &self.components {
            if component.id.is_empty() {
                return Err("component IDs must not be empty".into());
            }
            if !component_ids.insert(component.id.as_str()) {
                return Err(format!("duplicate component ID {}", component.id));
            }
            if component.size.x <= 0.0 || component.size.y <= 0.0 {
                return Err("component dimensions must be positive".into());
            }
            if let Some(geometry) = &component.placement_geometry {
                let mut body_layers = std::collections::HashSet::new();
                if geometry
                    .body_layers
                    .iter()
                    .any(|layer| !layer_ids.contains(layer.as_str()) || !body_layers.insert(layer))
                {
                    return Err("placement body layers must name distinct board layers".into());
                }
                let size = match geometry.body {
                    CopperShape::Circle { diameter } => Vec2::new(diameter, diameter),
                    CopperShape::Rect {
                        size,
                        rotation_degrees,
                    } => {
                        if rotation_degrees != 0.0 {
                            return Err(
                                "placement body rotation belongs to the component pose".into()
                            );
                        }
                        size
                    }
                };
                if !size.x.is_finite()
                    || !size.y.is_finite()
                    || !component.size.x.is_finite()
                    || !component.size.y.is_finite()
                    || size.x <= 0.0
                    || size.y <= 0.0
                    || (size.x - component.size.x).abs() > 1e-6
                    || (size.y - component.size.y).abs() > 1e-6
                    || !geometry.clearance.is_finite()
                    || geometry.clearance < 0.0
                    || !geometry.board_overhang.is_finite()
                    || geometry.board_overhang < 0.0
                    || !geometry.pad_edge_clearance.is_finite()
                    || geometry.pad_edge_clearance < 0.0
                {
                    return Err(format!(
                        "component {} has invalid placement geometry",
                        component.id
                    ));
                }
                if !geometry.body_parts.is_empty() {
                    if !matches!(geometry.body, CopperShape::Rect { .. }) {
                        return Err("compound placement bodies require rectangular bounds".into());
                    }
                    let mut low = Vec2::new(f64::INFINITY, f64::INFINITY);
                    let mut high = Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
                    for part in &geometry.body_parts {
                        part.check()?;
                        for v in &part.vertices {
                            low.x = low.x.min(v.x);
                            low.y = low.y.min(v.y);
                            high.x = high.x.max(v.x);
                            high.y = high.y.max(v.y);
                        }
                    }
                    if (low.x + size.x / 2.0).abs() > 1e-6
                        || (low.y + size.y / 2.0).abs() > 1e-6
                        || (high.x - size.x / 2.0).abs() > 1e-6
                        || (high.y - size.y / 2.0).abs() > 1e-6
                    {
                        return Err(
                            "compound placement body bounds must match the centered component size"
                                .into(),
                        );
                    }
                }
            }
            let mut pin_ids = std::collections::HashSet::new();
            for pin in &component.pins {
                if pin.id.is_empty() {
                    return Err(format!("component {} has an empty pin ID", component.id));
                }
                if !pin_ids.insert(pin.id.as_str()) {
                    return Err(format!(
                        "component {} has duplicate pin ID {}",
                        component.id, pin.id
                    ));
                }
                let mut pad_ids = std::collections::HashSet::new();
                for pad in &pin.pads {
                    if pad.id.is_empty() {
                        return Err(format!(
                            "component {} pin {} has an empty pad ID",
                            component.id, pin.id
                        ));
                    }
                    if !pad_ids.insert(pad.id.as_str()) {
                        return Err(format!(
                            "component {} pin {} has duplicate pad ID {}",
                            component.id, pin.id, pad.id
                        ));
                    }
                    if let Some(center) = pad.local_center
                        && (!center.x.is_finite() || !center.y.is_finite())
                    {
                        return Err(format!(
                            "component {} pin {} pad {} has a non-finite local center",
                            component.id, pin.id, pad.id
                        ));
                    }
                }
            }
        }
        if self.nets.iter().any(|net| net.width <= 0.0)
            || self.electrical_nets.iter().any(|net| net.width <= 0.0)
        {
            return Err("trace widths must be positive".into());
        }
        for component in &self.components {
            for pin in &component.pins {
                for pad in &pin.pads {
                    if !layer_ids.contains(pad.layer.as_str()) {
                        return Err(format!(
                            "pad {}.{} references unknown layer {}",
                            component.id, pin.id, pad.layer
                        ));
                    }
                    pad.shape.check()?;
                }
            }
            for keepout in &component.routing_keepouts {
                if !layer_ids.contains(keepout.layer.as_str()) {
                    return Err(format!(
                        "component {} keepout references unknown layer {}",
                        component.id, keepout.layer
                    ));
                }
                keepout.shape.check()?;
            }
        }
        let components_by_id = self
            .components
            .iter()
            .map(|component| (component.id.as_str(), component))
            .collect::<std::collections::HashMap<_, _>>();
        let validate_pin_ref = |reference: &PinRef, context: &str| -> Result<(), String> {
            let Some(component) = components_by_id.get(reference.component.as_str()) else {
                return Err(format!(
                    "{context} references unknown component {}",
                    reference.component
                ));
            };
            if !component
                .pins
                .iter()
                .any(|candidate| candidate.id == reference.pin)
            {
                return Err(format!(
                    "{context} references unknown pin {}.{}",
                    reference.component, reference.pin
                ));
            }
            Ok(())
        };
        for constraint in &self.placement_constraints {
            match constraint {
                PlacementConstraint::MaximumDistance {
                    first,
                    second,
                    maximum,
                } => {
                    if !maximum.is_finite() || *maximum < 0.0 {
                        return Err(
                            "maximum placement distance must be finite and non-negative".into()
                        );
                    }
                    for anchor in [first, second] {
                        let Some(component) = components_by_id.get(anchor.component.as_str())
                        else {
                            return Err(format!(
                                "placement anchor references unknown component {}",
                                anchor.component
                            ));
                        };
                        if let Some(pin) = &anchor.pin
                            && !component.pins.iter().any(|candidate| candidate.id == *pin)
                        {
                            return Err(format!(
                                "placement anchor references unknown pin {}.{}",
                                anchor.component, pin
                            ));
                        }
                    }
                }
                PlacementConstraint::FaceBoardEdge {
                    component,
                    edge,
                    local_facing_direction_degrees,
                    angular_tolerance_degrees,
                    maximum_distance,
                } => {
                    let Some(component_record) = components_by_id.get(component.as_str()) else {
                        return Err(format!(
                            "edge-facing constraint references unknown component {component}"
                        ));
                    };
                    if !local_facing_direction_degrees.is_finite()
                        || !angular_tolerance_degrees.is_finite()
                        || !(0.0..=180.0).contains(angular_tolerance_degrees)
                    {
                        return Err(
                            "edge-facing angles must be finite and tolerance must be between zero and 180 degrees"
                                .into(),
                        );
                    }
                    if maximum_distance
                        .is_some_and(|distance| !distance.is_finite() || distance < 0.0)
                    {
                        return Err(
                            "maximum board-edge distance must be finite and non-negative".into(),
                        );
                    }
                    if component_record.constraints.rotation == Rotation::Fixed {
                        let facing =
                            component_record.rotation_degrees + local_facing_direction_degrees;
                        let error = angular_distance_degrees(facing, edge.outward_angle_degrees());
                        if error > angular_tolerance_degrees + 1.0e-9 {
                            return Err(format!(
                                "fixed-rotation component {component} cannot face the {edge:?} board edge"
                            ));
                        }
                    }
                }
            }
        }
        for net in &self.nets {
            if !layer_ids.contains(net.layer.as_str()) {
                return Err(format!(
                    "net {} references unknown layer {}",
                    net.id, net.layer
                ));
            }
            if net
                .allowed_layers
                .iter()
                .any(|layer| !layer_ids.contains(layer.as_str()))
            {
                return Err(format!("net {} has an unknown allowed layer", net.id));
            }
            validate_pin_ref(&net.from, &format!("net {} from endpoint", net.id))?;
            validate_pin_ref(&net.to, &format!("net {} to endpoint", net.id))?;
        }
        for net in &self.electrical_nets {
            if net.terminals.len() < 2 {
                return Err(format!(
                    "electrical net {} must contain at least two terminals",
                    net.id
                ));
            }
            let mut terminals = std::collections::HashSet::new();
            if net.terminals.iter().any(|terminal| {
                !terminals.insert((terminal.component.as_str(), terminal.pin.as_str()))
            }) {
                return Err(format!(
                    "electrical net {} contains a duplicate terminal",
                    net.id
                ));
            }
            if !layer_ids.contains(net.layer.as_str()) {
                return Err(format!(
                    "electrical net {} references unknown layer {}",
                    net.id, net.layer
                ));
            }
            if net
                .allowed_layers
                .iter()
                .any(|layer| !layer_ids.contains(layer.as_str()))
            {
                return Err(format!(
                    "electrical net {} has an unknown allowed layer",
                    net.id
                ));
            }
            for (index, terminal) in net.terminals.iter().enumerate() {
                validate_pin_ref(
                    terminal,
                    &format!("electrical net {} terminal {}", net.id, index),
                )?;
            }
        }
        if self
            .nets
            .iter()
            .any(|net| !net.tension_weight.is_finite() || net.tension_weight < 0.0)
            || self
                .electrical_nets
                .iter()
                .any(|net| !net.tension_weight.is_finite() || net.tension_weight < 0.0)
        {
            return Err("trace tension weights must be finite and non-negative".into());
        }
        Ok(())
    }
}

/// A component feature measured by a relational placement constraint. An
/// omitted pin denotes the component center; a pin follows the component's
/// translation and rotation.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementAnchor {
    pub component: String,
    #[serde(default)]
    pub pin: Option<String>,
}

/// Hard relationships between placement anchors. Further relationships can
/// reuse this anchor representation without component-specific engine cases.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlacementConstraint {
    MaximumDistance {
        first: PlacementAnchor,
        second: PlacementAnchor,
        maximum: f64,
    },
    /// Keep a component-local facing direction pointed through a selected
    /// board edge. Coordinates use +X=east and +Y=north. An optional maximum
    /// distance measures the gap from the oriented component body (not its
    /// center) to that edge.
    FaceBoardEdge {
        component: String,
        edge: BoardEdge,
        #[serde(default)]
        local_facing_direction_degrees: f64,
        #[serde(default)]
        angular_tolerance_degrees: f64,
        #[serde(default)]
        maximum_distance: Option<f64>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardEdge {
    North,
    East,
    South,
    West,
}

impl BoardEdge {
    pub fn outward_angle_degrees(self) -> f64 {
        match self {
            Self::East => 0.0,
            Self::North => 90.0,
            Self::West => 180.0,
            Self::South => 270.0,
        }
    }
}

pub fn angular_distance_degrees(first: f64, second: f64) -> f64 {
    ((first - second + 180.0).rem_euclid(360.0) - 180.0).abs()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Board {
    pub bounds: Rect,
    #[serde(default = "default_copper_layers")]
    pub layers: Vec<CopperLayer>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CopperLayer {
    pub id: String,
}

fn default_copper_layers() -> Vec<CopperLayer> {
    vec![CopperLayer { id: "top".into() }]
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub clearance: f64,
    #[serde(default = "default_via_diameter")]
    pub via_diameter: f64,
    #[serde(default = "default_via_drill")]
    pub via_drill: f64,
    /// Permit a layer transition whose via is centered on a terminal pad.
    /// This requires a compatible fabrication process and is deliberately
    /// disabled unless the input problem opts in.
    #[serde(default)]
    pub allow_via_in_pad: bool,
}

fn default_via_diameter() -> f64 {
    0.8
}

fn default_via_drill() -> f64 {
    0.4
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    pub id: String,
    pub position: Vec2,
    pub size: Vec2,
    /// Explicit mechanical exclusion geometry. Absence retains the legacy
    /// pad-extended rectangle and board-rule clearance used by placement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement_geometry: Option<PlacementGeometry>,
    #[serde(default)]
    pub rotation_degrees: f64,
    #[serde(default)]
    pub constraints: ComponentConstraints,
    /// Legacy examples treat the placement body as copper keepout. Physical
    /// footprints set this false and provide pads/keepouts explicitly.
    #[serde(default = "default_true")]
    pub body_is_routing_keepout: bool,
    #[serde(default)]
    pub routing_keepouts: Vec<RoutingKeepout>,
    pub pins: Vec<Pin>,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementGeometry {
    /// Centered on the component origin. `size` must match its local bounds.
    /// When body_parts is nonempty this is only their rectangular envelope;
    /// the physical body is the union of the supplied convex parts.
    pub body: CopperShape,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body_parts: Vec<ConvexPlacementBody>,
    /// Mechanical sides represented by their board copper-layer IDs. Empty
    /// preserves the legacy all-layer exclusion policy, including pad pairs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body_layers: Vec<String>,
    /// Mechanical margin outside the supplied body/courtyard, not copper clearance.
    pub clearance: f64,
    /// Body-only allowance at the board outline. Pads and explicit regions
    /// remain contained. This must come from an explicit benchmark/product policy.
    #[serde(default)]
    pub board_overhang: f64,
    /// Copper-pad clearance to the board outline, independent of body overhang.
    #[serde(default)]
    pub pad_edge_clearance: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConvexPlacementBody {
    /// Ordered convex boundary in component-local coordinates. Parts may
    /// overlap; the union is the mechanical exclusion region, not its hull.
    pub vertices: Vec<Vec2>,
}

impl ConvexPlacementBody {
    fn check(&self) -> Result<(), String> {
        let vertices = &self.vertices;
        if vertices.len() < 3
            || vertices
                .iter()
                .any(|v| !v.x.is_finite() || !v.y.is_finite())
        {
            return Err("placement body parts need at least three finite vertices".into());
        }
        let area = vertices
            .iter()
            .zip(vertices.iter().cycle().skip(1))
            .take(vertices.len())
            .map(|(a, b)| a.x * b.y - b.x * a.y)
            .sum::<f64>();
        if !area.is_finite() || area.abs() <= 1e-12 {
            return Err("placement body parts must have positive area".into());
        }
        for (a, b) in vertices
            .iter()
            .zip(vertices.iter().cycle().skip(1))
            .take(vertices.len())
        {
            if (b.x - a.x).hypot(b.y - a.y) <= 1e-9
                || vertices.iter().any(|v| {
                    ((b.x - a.x) * (v.y - a.y) - (b.y - a.y) * (v.x - a.x)) * area.signum() < -1e-9
                })
            {
                return Err(
                    "placement body parts must be simple convex polygons with distinct edges"
                        .into(),
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentConstraints {
    #[serde(default)]
    pub movement: Movement,
    #[serde(default)]
    pub rotation: Rotation,
    #[serde(default)]
    pub region: Option<Rect>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Movement {
    Fixed,
    Horizontal,
    Vertical,
    #[default]
    Free,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rotation {
    Fixed,
    #[default]
    Free,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    pub id: String,
    pub offset: Vec2,
    #[serde(default)]
    pub pads: Vec<Pad>,
}

impl Pin {
    /// Component-local center of this physical copper shape. This is a rigid
    /// structural transform in the owning component frame, never a routing
    /// edge, copper bridge, layer transition, or independent degree of
    /// freedom. Legacy pads are centered on the logical pin anchor.
    pub fn pad_local_center(&self, pad: &Pad) -> Vec2 {
        pad.local_center.unwrap_or(self.offset)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Pad {
    /// Stable identity within the owning logical pin. Older inputs which omit
    /// this field receive a deterministic content-derived ID during
    /// deserialization. Identical same-pin legacy pad records are rejected as
    /// ambiguous; they need explicit IDs.
    pub id: String,
    /// Optional rigid component-local physical center. Absence preserves the
    /// legacy convention that pad copper is centered on `Pin::offset`. The
    /// relation to that logical anchor is structural only; it contributes no
    /// hidden copper, layer, capacity, homotopy, or pad degree of freedom.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_center: Option<Vec2>,
    pub layer: String,
    pub shape: CopperShape,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SerializedPad {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    local_center: Option<Vec2>,
    layer: String,
    shape: CopperShape,
}

impl<'de> Deserialize<'de> for Pad {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let serialized = SerializedPad::deserialize(deserializer)?;
        let id = serialized.id.unwrap_or_else(|| {
            generated_pad_id(
                &serialized.layer,
                &serialized.shape,
                serialized.local_center,
            )
        });
        Ok(Self {
            id,
            local_center: serialized.local_center,
            layer: serialized.layer,
            shape: serialized.shape,
        })
    }
}

/// Typed identity of physical copper owned by a rigid component frame. This
/// is an anchor reference only: it has no trace width, capacity, or layer-edge
/// semantics of its own.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PadRef {
    pub component: String,
    pub pin: String,
    pub pad: String,
}

/// Topology-facing port identity. A BGA escape solver can eventually expose a
/// certified breakout port without pretending that the structural pad anchor
/// is copper connecting it to that port.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "kind", content = "reference", rename_all = "snake_case")]
pub enum PadOrBreakoutPortRef {
    PhysicalPad(PadRef),
    BreakoutPort { interface: String, port: String },
}

fn legacy_pad_id(layer: &str, shape: &CopperShape) -> String {
    let shape_key = match shape {
        CopperShape::Circle { diameter } => format!("circle:{:016x}", diameter.to_bits()),
        CopperShape::Rect {
            size,
            rotation_degrees,
        } => format!(
            "rect:{:016x}:{:016x}:{:016x}",
            size.x.to_bits(),
            size.y.to_bits(),
            rotation_degrees.to_bits()
        ),
    };
    format!("legacy-pad-v1:{}:{layer}:{shape_key}", layer.len())
}

fn generated_pad_id(layer: &str, shape: &CopperShape, local_center: Option<Vec2>) -> String {
    let legacy = legacy_pad_id(layer, shape);
    match local_center {
        None => legacy,
        Some(center) => format!(
            "legacy-pad-v2:{}:{:016x}:{:016x}",
            legacy.len(),
            center.x.to_bits(),
            center.y.to_bits()
        ),
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CopperShape {
    Circle {
        diameter: f64,
    },
    Rect {
        size: Vec2,
        #[serde(default)]
        rotation_degrees: f64,
    },
}

impl CopperShape {
    fn check(&self) -> Result<(), String> {
        match self {
            Self::Circle { diameter } if diameter.is_finite() && *diameter > 0.0 => Ok(()),
            Self::Rect { size, .. }
                if size.x.is_finite() && size.y.is_finite() && size.x > 0.0 && size.y > 0.0 =>
            {
                Ok(())
            }
            _ => Err("copper shapes must have finite positive dimensions".into()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingKeepout {
    pub layer: String,
    #[serde(default)]
    pub offset: Vec2,
    pub shape: CopperShape,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PinRef {
    pub component: String,
    pub pin: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Net {
    pub id: String,
    /// Optional electrical identity shared by several legacy route branches.
    /// When absent, `id` is both the electrical-net and branch identity.
    #[serde(default)]
    pub electrical_net: Option<String>,
    pub width: f64,
    /// Relative coefficient of curve-shortening energy. Clearance forces are
    /// unaffected. Zero means that the route reserves connectivity without
    /// preferring a shorter representative.
    #[serde(default = "default_tension_weight")]
    pub tension_weight: f64,
    #[serde(default = "default_top_layer")]
    pub layer: String,
    /// Empty means only `layer`; otherwise the outer layer-assignment pass may
    /// select any listed layer and inserts endpoint vias when necessary.
    #[serde(default)]
    pub allowed_layers: Vec<String>,
    pub from: PinRef,
    pub to: PinRef,
    #[serde(default)]
    pub seed_route: Vec<Vec2>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ElectricalNet {
    pub id: String,
    pub width: f64,
    #[serde(default = "default_tension_weight")]
    pub tension_weight: f64,
    #[serde(default = "default_top_layer")]
    pub layer: String,
    #[serde(default)]
    pub allowed_layers: Vec<String>,
    pub terminals: Vec<PinRef>,
}

fn default_tension_weight() -> f64 {
    1.0
}

fn default_top_layer() -> String {
    "top".into()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SolverConfig {
    #[serde(default = "default_stages")]
    pub width_stages: Vec<f64>,
    #[serde(default = "default_iterations")]
    pub iterations_per_stage: usize,
    #[serde(default = "default_particles")]
    pub particles_per_trace: usize,
    #[serde(default = "default_step")]
    pub step_size: f64,
    #[serde(default = "default_smoothing")]
    pub smoothing: f64,
    #[serde(default = "default_repulsion")]
    pub repulsion: f64,
    /// Extra distance over which clearance forces become active. Exact DRC
    /// still uses only the declared physical clearance.
    #[serde(default)]
    pub clearance_influence: f64,
    #[serde(default = "default_component_mobility")]
    pub component_mobility: f64,
    #[serde(default = "default_rotation_mobility")]
    pub rotation_mobility: f64,
}

impl Default for SolverConfig {
    fn default() -> Self {
        Self {
            width_stages: default_stages(),
            iterations_per_stage: default_iterations(),
            particles_per_trace: default_particles(),
            step_size: default_step(),
            smoothing: default_smoothing(),
            repulsion: default_repulsion(),
            clearance_influence: 0.0,
            component_mobility: default_component_mobility(),
            rotation_mobility: default_rotation_mobility(),
        }
    }
}

fn default_stages() -> Vec<f64> {
    vec![0.0, 0.1, 0.25, 0.5, 0.75, 1.0]
}

fn default_iterations() -> usize {
    300
}

fn default_particles() -> usize {
    10
}

fn default_step() -> f64 {
    0.04
}

fn default_smoothing() -> f64 {
    0.35
}

fn default_repulsion() -> f64 {
    1.4
}

fn default_component_mobility() -> f64 {
    0.25
}

fn default_rotation_mobility() -> f64 {
    0.002
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference_problem() -> Problem {
        serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "board": {
                "bounds": {
                    "min": { "x": 0.0, "y": 0.0 },
                    "max": { "x": 20.0, "y": 20.0 }
                },
                "layers": [{ "id": "top" }]
            },
            "rules": { "clearance": 0.2 },
            "components": [
                {
                    "id": "A",
                    "position": { "x": 2.0, "y": 10.0 },
                    "size": { "x": 2.0, "y": 2.0 },
                    "pins": [{ "id": "1", "offset": { "x": 0.0, "y": 0.0 } }]
                },
                {
                    "id": "B",
                    "position": { "x": 18.0, "y": 10.0 },
                    "size": { "x": 2.0, "y": 2.0 },
                    "pins": [{ "id": "1", "offset": { "x": 0.0, "y": 0.0 } }]
                }
            ],
            "nets": [{
                "id": "LEGACY",
                "width": 0.25,
                "from": { "component": "A", "pin": "1" },
                "to": { "component": "B", "pin": "1" }
            }],
            "electrical_nets": [{
                "id": "TREE",
                "width": 0.25,
                "terminals": [
                    { "component": "A", "pin": "1" },
                    { "component": "B", "pin": "1" }
                ]
            }]
        }))
        .unwrap()
    }

    #[test]
    fn schema_accepts_known_legacy_and_electrical_terminal_pins() {
        reference_problem().check_schema().unwrap();
    }

    #[test]
    fn schema_rejects_duplicate_component_and_pin_ids() {
        let mut duplicate_component = reference_problem();
        duplicate_component.components[1].id = "A".into();
        assert_eq!(
            duplicate_component.check_schema().unwrap_err(),
            "duplicate component ID A"
        );

        let mut duplicate_pin = reference_problem();
        let repeated_pin = duplicate_pin.components[0].pins[0].clone();
        duplicate_pin.components[0].pins.push(repeated_pin);
        assert_eq!(
            duplicate_pin.check_schema().unwrap_err(),
            "component A has duplicate pin ID 1"
        );
    }

    #[test]
    fn schema_rejects_unknown_legacy_endpoint_component_and_pin() {
        let mut unknown_component = reference_problem();
        unknown_component.nets[0].from.component = "MISSING".into();
        assert_eq!(
            unknown_component.check_schema().unwrap_err(),
            "net LEGACY from endpoint references unknown component MISSING"
        );

        let mut unknown_pin = reference_problem();
        unknown_pin.nets[0].to.pin = "missing".into();
        assert_eq!(
            unknown_pin.check_schema().unwrap_err(),
            "net LEGACY to endpoint references unknown pin B.missing"
        );
    }

    #[test]
    fn schema_rejects_unknown_electrical_terminal_component_and_pin() {
        let mut unknown_component = reference_problem();
        unknown_component.electrical_nets[0].terminals[0].component = "MISSING".into();
        assert_eq!(
            unknown_component.check_schema().unwrap_err(),
            "electrical net TREE terminal 0 references unknown component MISSING"
        );

        let mut unknown_pin = reference_problem();
        unknown_pin.electrical_nets[0].terminals[1].pin = "missing".into();
        assert_eq!(
            unknown_pin.check_schema().unwrap_err(),
            "electrical net TREE terminal 1 references unknown pin B.missing"
        );
    }

    #[test]
    fn implicit_pad_ids_are_stable_under_pad_permutation() {
        let pin_json = |reverse: bool| {
            let mut pads = vec![
                serde_json::json!({
                    "layer": "top",
                    "shape": {"kind": "circle", "diameter": 1.0}
                }),
                serde_json::json!({
                    "layer": "bottom",
                    "shape": {"kind": "circle", "diameter": 1.0}
                }),
            ];
            if reverse {
                pads.reverse();
            }
            serde_json::json!({
                "id": "P",
                "offset": {"x": 0.0, "y": 0.0},
                "pads": pads
            })
        };
        let forward: Pin = serde_json::from_value(pin_json(false)).unwrap();
        let reverse: Pin = serde_json::from_value(pin_json(true)).unwrap();
        let by_layer = |pin: &Pin| {
            pin.pads
                .iter()
                .map(|pad| (pad.layer.clone(), pad.id.clone()))
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        assert_eq!(by_layer(&forward), by_layer(&reverse));
        assert_ne!(forward.pads[0].id, forward.pads[1].id);
    }

    #[test]
    fn explicit_local_centers_distinguish_idless_physical_pads_without_changing_legacy_json() {
        let legacy: Pad = serde_json::from_value(serde_json::json!({
            "layer": "top",
            "shape": {"kind": "circle", "diameter": 1.0}
        }))
        .unwrap();
        assert!(legacy.id.starts_with("legacy-pad-v1:"));
        assert!(
            serde_json::to_value(&legacy)
                .unwrap()
                .get("local_center")
                .is_none()
        );

        let centered = |x| -> Pad {
            serde_json::from_value(serde_json::json!({
                "layer": "top",
                "local_center": {"x": x, "y": 0.25},
                "shape": {"kind": "circle", "diameter": 1.0}
            }))
            .unwrap()
        };
        let left = centered(-0.75);
        let right = centered(0.75);
        assert_ne!(left.id, right.id);
        assert!(left.id.starts_with("legacy-pad-v2:"));

        let pin = Pin {
            id: "P".into(),
            offset: Vec2::new(9.0, 8.0),
            pads: vec![legacy.clone(), left.clone()],
        };
        assert_eq!(pin.pad_local_center(&legacy), pin.offset);
        assert_eq!(pin.pad_local_center(&left), Vec2::new(-0.75, 0.25));
    }

    #[test]
    fn explicit_ids_support_multiple_physical_pads_on_one_pin() {
        let mut problem = reference_problem();
        problem.components[0].pins[0].pads = vec![
            Pad {
                id: "left".into(),
                local_center: Some(Vec2::new(-0.5, 0.0)),
                layer: "top".into(),
                shape: CopperShape::Circle { diameter: 0.8 },
            },
            Pad {
                id: "right".into(),
                local_center: Some(Vec2::new(0.5, 0.0)),
                layer: "top".into(),
                shape: CopperShape::Circle { diameter: 0.8 },
            },
        ];
        problem.check_schema().unwrap();

        problem.components[0].pins[0].pads[1].id = "left".into();
        assert_eq!(
            problem.check_schema().unwrap_err(),
            "component A pin 1 has duplicate pad ID left"
        );
    }

    #[test]
    fn legacy_examples_gain_nonempty_pad_ids() {
        for source in [
            include_str!(
                "../../../benchmarks/imported/layout-trace/forced-crossing-two-layer.json"
            ),
            include_str!("../../../benchmarks/imported/layout-trace/dual-esp32-benchmark.json"),
        ] {
            let problem: Problem = serde_json::from_str(source).unwrap();
            problem.check_schema().unwrap();
            assert!(problem.components.iter().all(|component| {
                component
                    .pins
                    .iter()
                    .all(|pin| pin.pads.iter().all(|pad| !pad.id.is_empty()))
            }));
        }
    }

    #[test]
    fn physical_pad_and_breakout_refs_are_typed_and_round_trip() {
        let references = [
            PadOrBreakoutPortRef::PhysicalPad(PadRef {
                component: "U1".into(),
                pin: "GPIO0".into(),
                pad: "top-copper".into(),
            }),
            PadOrBreakoutPortRef::BreakoutPort {
                interface: "U1.escape".into(),
                port: "north-3".into(),
            },
        ];
        for reference in references {
            let serialized = serde_json::to_string(&reference).unwrap();
            let decoded: PadOrBreakoutPortRef = serde_json::from_str(&serialized).unwrap();
            assert_eq!(decoded, reference);
        }
    }
}
