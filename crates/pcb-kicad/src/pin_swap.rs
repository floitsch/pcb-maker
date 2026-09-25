// Copyright (C) 2026 Toit contributors.

//! Pin swapping: nets on interchangeable pins (the GPIOs of a
//! microcontroller, the elements of a resistor network, the ports of a
//! crosspoint switch) are permuted so that the board gets easier to route,
//! and the schematic is rewritten to match the board.
//!
//! The interchangeable pins come from a `pin-swaps.json` next to the board
//! (see `benchmarks/fence/README.md`). A *group* holds *units*; the pins of a
//! unit move together (a resistor-network element is one unit of two pins),
//! and the nets on the units of a group may be permuted freely, subject to
//! per-net `restrictions`. The assignment is chosen by annealing on a
//! ratsnest estimate: minimum-spanning-tree length plus a penalty for every
//! crossing of two ratsnest edges of different nets. On two layers every
//! crossing costs routing room or a via, so crossings weigh several
//! millimetres.
//!
//! A pin of a chip cannot leave in any direction: it escapes outwards,
//! past its neighbours, and a net that has to reach the far side of the
//! chip goes around it. The estimate therefore starts each such pin at an
//! escape point outside the footprint's pad field and charges every
//! ratsnest line that runs through a pad field.

use super::*;
use std::collections::HashMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadPinSwapSpec {
    pub version: u32,
    /// Keyed by footprint reference; a key starting with `*` names a group
    /// of units on several footprints (`cross_component`), whose pins are
    /// written `REF:pad`.
    pub components: BTreeMap<String, KiCadPinSwapComponent>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadPinSwapComponent {
    pub groups: Vec<KiCadPinSwapGroup>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadPinSwapGroup {
    pub name: String,
    pub units: Vec<Vec<String>>,
    /// Net name (without the sheet path) to the pins it may use.
    #[serde(default)]
    pub restrictions: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub cross_component: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadPinSwapConfig {
    /// Millimetres of ratsnest length one crossing is worth.
    #[serde(default = "default_crossing_cost")]
    pub crossing_cost_mm: f64,
    /// Nets with more pads than this (power, ground) are left out of the
    /// estimate: they are poured or routed wide anyway.
    #[serde(default = "default_maximum_pads")]
    pub maximum_net_pads: usize,
    /// Millimetres charged when a ratsnest line runs through the pad field
    /// of a surface-mount part with at least four pads (a detour around it).
    #[serde(default = "default_body_cost")]
    pub body_crossing_cost_mm: f64,
    /// Annealing moves per unit.
    #[serde(default = "default_moves_per_unit")]
    pub moves_per_unit: usize,
    #[serde(default)]
    pub seed: u64,
}

fn default_crossing_cost() -> f64 {
    3.0
}

fn default_body_cost() -> f64 {
    10.0
}

fn default_maximum_pads() -> usize {
    12
}

fn default_moves_per_unit() -> usize {
    2000
}

impl Default for KiCadPinSwapConfig {
    fn default() -> Self {
        Self {
            crossing_cost_mm: default_crossing_cost(),
            body_crossing_cost_mm: default_body_cost(),
            maximum_net_pads: default_maximum_pads(),
            moves_per_unit: default_moves_per_unit(),
            seed: 0,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct KiCadPinSwapEstimate {
    pub length_mm: f64,
    pub crossings: usize,
    /// Ratsnest lines through the pad field of a part.
    #[serde(default)]
    pub body_crossings: usize,
    pub cost: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadPinChange {
    pub reference: String,
    pub pad: String,
    pub before: String,
    pub after: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadPinSwapResult {
    pub before: KiCadPinSwapEstimate,
    pub after: KiCadPinSwapEstimate,
    pub groups: usize,
    pub units: usize,
    /// Every pad whose net changed.
    pub changes: Vec<KiCadPinChange>,
    pub schematic_labels_changed: usize,
    pub seconds: f64,
}

/// One pin of a unit: a footprint reference and a pad name (all pads of
/// that name, since a pad name may repeat).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct PinKey {
    reference: String,
    pad: String,
}

#[derive(Clone, Debug)]
struct Unit {
    pins: Vec<PinKey>,
}

#[derive(Clone, Debug)]
struct Group {
    units: Vec<Unit>,
    /// `allowed[payload]` lists the units payload may sit on (all if empty).
    allowed: Vec<Vec<bool>>,
}

/// Nets as the estimate sees them: fixed pad positions plus pins that
/// belong to a unit and move with the assignment.
/// A pad of the estimate: where it is and where its track leaves the
/// part (the same point for pads that can be left in any direction).
#[derive(Clone, Copy, Debug)]
struct Terminal {
    pad: [f64; 2],
    escape: [f64; 2],
}

#[derive(Clone, Debug, Default)]
struct NetTerminals {
    fixed: Vec<Terminal>,
    /// (group, payload, index of the pin within the unit).
    moving: Vec<(usize, usize, usize)>,
}

/// A pad field (minimum, maximum) that ratsnest lines should go around.
type Body = ([f64; 2], [f64; 2]);

struct Problem {
    groups: Vec<Group>,
    /// Pads per pin (several when a pad name repeats).
    positions: HashMap<PinKey, Vec<Terminal>>,
    bodies: Vec<Body>,
    /// payloads[group][payload] = the nets on the unit's pins, in order.
    payloads: Vec<Vec<Vec<Option<String>>>>,
    nets: Vec<NetTerminals>,
    /// Nets (indices into `nets`) that each payload carries.
    payload_nets: Vec<Vec<Vec<usize>>>,
}

type Edge = ([f64; 2], [f64; 2]);

struct State<'a> {
    problem: &'a Problem,
    config: &'a KiCadPinSwapConfig,
    /// unit_of[group][payload]
    unit_of: Vec<Vec<usize>>,
    /// payload_on[group][unit]
    payload_on: Vec<Vec<usize>>,
    /// Per net: its ratsnest segments (escape stubs included).
    edges: Vec<Vec<Edge>>,
    lengths: Vec<f64>,
    /// Per net: ratsnest lines through a pad field.
    bodies: Vec<usize>,
}

fn segments_cross(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let orient = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    };
    let (d1, d2) = (orient(c, d, a), orient(c, d, b));
    let (d3, d4) = (orient(a, b, c), orient(a, b, d));
    // Edges sharing an end point (a pad of both nets cannot exist, but two
    // pins of one footprint can coincide in a degenerate unit) do not cross.
    const EPSILON: f64 = 1.0e-9;
    d1.abs() > EPSILON
        && d2.abs() > EPSILON
        && d3.abs() > EPSILON
        && d4.abs() > EPSILON
        && (d1 > 0.0) != (d2 > 0.0)
        && (d3 > 0.0) != (d4 > 0.0)
}

/// Whether segment a-b enters the box (Liang-Barsky clipping).
fn segment_hits_box(a: [f64; 2], b: [f64; 2], body: &Body) -> bool {
    let (mut low, mut high) = (0.0f64, 1.0f64);
    for axis in 0..2 {
        let delta = b[axis] - a[axis];
        if delta.abs() < 1.0e-12 {
            if a[axis] <= body.0[axis] || a[axis] >= body.1[axis] {
                return false;
            }
            continue;
        }
        let mut t0 = (body.0[axis] - a[axis]) / delta;
        let mut t1 = (body.1[axis] - a[axis]) / delta;
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        low = low.max(t0);
        high = high.min(t1);
        if low >= high {
            return false;
        }
    }
    true
}

/// Minimum spanning tree over the escape points, plus the escape stubs.
fn spanning_tree(terminals: &[Terminal]) -> (Vec<Edge>, f64) {
    let points: Vec<[f64; 2]> = terminals.iter().map(|terminal| terminal.escape).collect();
    let mut edges = Vec::new();
    let mut length = 0.0;
    if points.len() < 2 {
        return (edges, length);
    }
    for terminal in terminals {
        let stub = distance_squared(terminal.pad, terminal.escape).sqrt();
        if stub > 1.0e-9 {
            edges.push((terminal.pad, terminal.escape));
            length += stub;
        }
    }
    let mut inside = vec![false; points.len()];
    let mut best = vec![(f64::INFINITY, 0usize); points.len()];
    inside[0] = true;
    for index in 1..points.len() {
        best[index] = (distance_squared(points[0], points[index]).sqrt(), 0);
    }
    for _ in 1..points.len() {
        let next = (0..points.len())
            .filter(|index| !inside[*index])
            .min_by(|a, b| best[*a].0.total_cmp(&best[*b].0))
            .unwrap();
        inside[next] = true;
        edges.push((points[best[next].1], points[next]));
        length += best[next].0;
        for index in 0..points.len() {
            if !inside[index] {
                let distance = distance_squared(points[next], points[index]).sqrt();
                if distance < best[index].0 {
                    best[index] = (distance, next);
                }
            }
        }
    }
    (edges, length)
}

impl<'a> State<'a> {
    fn new(problem: &'a Problem, config: &'a KiCadPinSwapConfig) -> Self {
        let unit_of: Vec<Vec<usize>> = problem
            .groups
            .iter()
            .map(|group| (0..group.units.len()).collect())
            .collect();
        let payload_on = unit_of.clone();
        let mut state = Self {
            problem,
            config,
            unit_of,
            payload_on,
            edges: vec![Vec::new(); problem.nets.len()],
            lengths: vec![0.0; problem.nets.len()],
            bodies: vec![0; problem.nets.len()],
        };
        for net in 0..problem.nets.len() {
            state.refresh(net);
        }
        state
    }

    fn points(&self, net: usize) -> Vec<Terminal> {
        let terminals = &self.problem.nets[net];
        let mut points = terminals.fixed.clone();
        for (group, payload, pin) in &terminals.moving {
            let unit = self.unit_of[*group][*payload];
            let key = &self.problem.groups[*group].units[unit].pins[*pin];
            points.extend(self.problem.positions[key].iter().copied());
        }
        points
    }

    fn refresh(&mut self, net: usize) {
        let (edges, length) = spanning_tree(&self.points(net));
        self.bodies[net] = edges
            .iter()
            .map(|(a, b)| {
                self.problem
                    .bodies
                    .iter()
                    .filter(|body| segment_hits_box(*a, *b, body))
                    .count()
            })
            .sum();
        self.edges[net] = edges;
        self.lengths[net] = length;
    }

    fn crossings_of(&self, net: usize, skip: &[usize]) -> usize {
        let mut count = 0;
        for (other, edges) in self.edges.iter().enumerate() {
            if other == net || skip.contains(&other) {
                continue;
            }
            for (a, b) in &self.edges[net] {
                for (c, d) in edges {
                    if segments_cross(*a, *b, *c, *d) {
                        count += 1;
                    }
                }
            }
        }
        count
    }

    /// Cost of the nets in `touched`: their length and every crossing they
    /// take part in (crossings among them counted once).
    fn local_cost(&self, touched: &[usize]) -> f64 {
        let mut length = 0.0;
        let mut crossings = 0;
        let mut bodies = 0;
        for (index, net) in touched.iter().enumerate() {
            length += self.lengths[*net];
            bodies += self.bodies[*net];
            crossings += self.crossings_of(*net, &touched[..index]);
        }
        length
            + self.config.crossing_cost_mm * crossings as f64
            + self.config.body_crossing_cost_mm * bodies as f64
    }

    fn estimate(&self) -> KiCadPinSwapEstimate {
        let length_mm: f64 = self.lengths.iter().sum();
        let mut crossings = 0;
        for net in 0..self.edges.len() {
            for other in net + 1..self.edges.len() {
                for (a, b) in &self.edges[net] {
                    for (c, d) in &self.edges[other] {
                        if segments_cross(*a, *b, *c, *d) {
                            crossings += 1;
                        }
                    }
                }
            }
        }
        let body_crossings: usize = self.bodies.iter().sum();
        KiCadPinSwapEstimate {
            length_mm,
            crossings,
            body_crossings,
            cost: length_mm
                + self.config.crossing_cost_mm * crossings as f64
                + self.config.body_crossing_cost_mm * body_crossings as f64,
        }
    }

    fn allowed(&self, group: usize, payload: usize, unit: usize) -> bool {
        let allowed = &self.problem.groups[group].allowed[payload];
        allowed.is_empty() || allowed[unit]
    }

    /// Exchanges the payloads on two units of a group.
    fn exchange(&mut self, group: usize, first: usize, second: usize) {
        let (a, b) = (self.payload_on[group][first], self.payload_on[group][second]);
        self.payload_on[group][first] = b;
        self.payload_on[group][second] = a;
        self.unit_of[group][a] = second;
        self.unit_of[group][b] = first;
    }

    fn touched(&self, group: usize, first: usize, second: usize) -> Vec<usize> {
        let mut touched: Vec<usize> = [first, second]
            .iter()
            .flat_map(|unit| {
                self.problem.payload_nets[group][self.payload_on[group][*unit]]
                    .iter()
                    .copied()
            })
            .collect();
        touched.sort_unstable();
        touched.dedup();
        touched
    }

    /// Cost change of exchanging two units; the state is left exchanged
    /// when `keep` says so.
    fn try_exchange(
        &mut self,
        group: usize,
        first: usize,
        second: usize,
        keep: impl FnOnce(f64) -> bool,
    ) -> Option<f64> {
        let (a, b) = (self.payload_on[group][first], self.payload_on[group][second]);
        if !self.allowed(group, a, second) || !self.allowed(group, b, first) {
            return None;
        }
        let touched = self.touched(group, first, second);
        if touched.is_empty() {
            return None;
        }
        let before = self.local_cost(&touched);
        let saved: Vec<(Vec<Edge>, f64, usize)> = touched
            .iter()
            .map(|net| (self.edges[*net].clone(), self.lengths[*net], self.bodies[*net]))
            .collect();
        self.exchange(group, first, second);
        for net in &touched {
            self.refresh(*net);
        }
        let delta = self.local_cost(&touched) - before;
        if !keep(delta) {
            self.exchange(group, first, second);
            for (net, (edges, length, bodies)) in touched.iter().zip(saved) {
                self.edges[*net] = edges;
                self.lengths[*net] = length;
                self.bodies[*net] = bodies;
            }
        }
        Some(delta)
    }
}

/// Small deterministic generator (xorshift64*), so runs are repeatable.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn optimize<'a>(problem: &'a Problem, config: &'a KiCadPinSwapConfig) -> State<'a> {
    let mut state = State::new(problem, config);
    let movable: Vec<usize> = (0..problem.groups.len())
        .filter(|group| problem.groups[*group].units.len() >= 2)
        .collect();
    if movable.is_empty() {
        return state;
    }
    let units: usize = movable.iter().map(|group| problem.groups[*group].units.len()).sum();
    let mut random = Random(config.seed ^ 0x9e37_79b9_7f4a_7c15);
    let pick = |random: &mut Random| {
        // Groups are drawn in proportion to their size.
        let mut index = random.below(units);
        let mut group = movable[0];
        for candidate in &movable {
            let size = problem.groups[*candidate].units.len();
            if index < size {
                group = *candidate;
                break;
            }
            index -= size;
        }
        let size = problem.groups[group].units.len();
        let first = random.below(size);
        let second = (first + 1 + random.below(size - 1)) % size;
        (group, first, second)
    };
    // Starting temperature: the typical size of a worsening move.
    let mut samples = Vec::new();
    for _ in 0..200 {
        let (group, first, second) = pick(&mut random);
        if let Some(delta) = state.try_exchange(group, first, second, |_| false)
            && delta > 0.0
        {
            samples.push(delta);
        }
    }
    samples.sort_by(f64::total_cmp);
    let mut temperature = samples.get(samples.len() / 2).copied().unwrap_or(1.0);
    let moves = config.moves_per_unit * units;
    let cooling = (1.0e-3f64).powf(1.0 / moves.max(1) as f64);
    for _ in 0..moves {
        let (group, first, second) = pick(&mut random);
        let threshold = random.unit();
        state.try_exchange(group, first, second, |delta| {
            delta <= 0.0 || threshold < (-delta / temperature).exp()
        });
        temperature *= cooling;
    }
    // Finish with steepest descent over all exchanges.
    loop {
        let mut improved = false;
        for group in &movable {
            let size = problem.groups[*group].units.len();
            for first in 0..size {
                for second in first + 1..size {
                    if state
                        .try_exchange(*group, first, second, |delta| delta < -1.0e-9)
                        .is_some_and(|delta| delta < -1.0e-9)
                    {
                        improved = true;
                    }
                }
            }
        }
        if !improved {
            break;
        }
    }
    state
}

fn pin_key(group_key: &str, cross_component: bool, pin: &str) -> Result<PinKey, String> {
    if cross_component {
        let (reference, pad) = pin
            .split_once(':')
            .ok_or_else(|| format!("pin {pin:?} of {group_key} is not written REF:pad"))?;
        Ok(PinKey {
            reference: reference.to_string(),
            pad: pad.to_string(),
        })
    } else {
        Ok(PinKey {
            reference: group_key.to_string(),
            pad: pin.to_string(),
        })
    }
}

/// Pads of the board: position and net per (reference, pad name).
struct BoardPads {
    positions: HashMap<PinKey, Vec<Terminal>>,
    nets: HashMap<PinKey, Option<String>>,
    all: Vec<(PinKey, Terminal, Option<String>)>,
    bodies: Vec<Body>,
}

/// How far outside the pad field a pin's track is taken to be free.
const ESCAPE_MM: f64 = 1.0;

fn board_pads(pcb: &Expr) -> Result<BoardPads, String> {
    let mut pads = BoardPads {
        positions: HashMap::new(),
        nets: HashMap::new(),
        all: Vec::new(),
        bodies: Vec::new(),
    };
    for footprint in pcb.children().iter().filter(|item| item.head() == Some("footprint")) {
        let at = form_at(footprint)?;
        let reference = footprint_reference(footprint).unwrap_or_default();
        let mut own: Vec<(PinKey, [f64; 2], Option<String>)> = Vec::new();
        let mut through = false;
        for pad in footprint.children().iter().filter(|item| item.head() == Some("pad")) {
            through |= matches!(
                pad.children().get(2).and_then(Expr::atom),
                Some("thru_hole" | "np_thru_hole")
            );
            let name = pad.children().get(1).and_then(Expr::atom).unwrap_or("").to_string();
            let offset = rotate_vector(
                {
                    let pad_at = form_at(pad)?;
                    [pad_at[0], pad_at[1]]
                },
                -at[2],
            );
            let center = [at[0] + offset[0], at[1] + offset[1]];
            let net = node_net(pad).map(str::to_string);
            let key = PinKey {
                reference: reference.clone(),
                pad: name,
            };
            own.push((key, center, net));
        }
        // A surface-mount part with a pad field: its pins escape outwards,
        // along the axis on which they lie furthest out.
        let field = (!through && own.len() >= 4).then(|| {
            let mut minimum = [f64::INFINITY; 2];
            let mut maximum = [f64::NEG_INFINITY; 2];
            for (_, center, _) in &own {
                for axis in 0..2 {
                    minimum[axis] = minimum[axis].min(center[axis]);
                    maximum[axis] = maximum[axis].max(center[axis]);
                }
            }
            (minimum, maximum)
        });
        if let Some((minimum, maximum)) = field {
            pads.bodies.push((minimum, maximum));
        }
        for (key, center, net) in own {
            let escape = match field {
                Some((minimum, maximum)) => {
                    let middle = [(minimum[0] + maximum[0]) / 2.0, (minimum[1] + maximum[1]) / 2.0];
                    let half = [
                        ((maximum[0] - minimum[0]) / 2.0).max(1.0e-3),
                        ((maximum[1] - minimum[1]) / 2.0).max(1.0e-3),
                    ];
                    let relative = [
                        (center[0] - middle[0]) / half[0],
                        (center[1] - middle[1]) / half[1],
                    ];
                    let axis = if relative[0].abs() >= relative[1].abs() { 0 } else { 1 };
                    let mut escape = center;
                    escape[axis] = if relative[axis] >= 0.0 {
                        maximum[axis] + ESCAPE_MM
                    } else {
                        minimum[axis] - ESCAPE_MM
                    };
                    escape
                }
                None => center,
            };
            let terminal = Terminal { pad: center, escape };
            pads.positions.entry(key.clone()).or_default().push(terminal);
            let entry = pads.nets.entry(key.clone()).or_insert_with(|| net.clone());
            if entry.is_none() {
                *entry = net.clone();
            }
            pads.all.push((key, terminal, net));
        }
    }
    Ok(pads)
}

/// Nets that stand for "no connection": they never count and never need a
/// schematic label.
fn is_unconnected(net: &Option<String>) -> bool {
    net.as_deref()
        .is_none_or(|net| net.is_empty() || net.starts_with("unconnected-"))
}

fn build_problem(
    spec: &KiCadPinSwapSpec,
    pads: &BoardPads,
    config: &KiCadPinSwapConfig,
) -> Result<Problem, String> {
    if spec.version != 1 {
        return Err(format!("unsupported pin-swaps version {}", spec.version));
    }
    let mut groups = Vec::new();
    let mut payloads = Vec::new();
    let mut seen: BTreeSet<PinKey> = BTreeSet::new();
    for (key, component) in &spec.components {
        for group in &component.groups {
            let cross = group.cross_component || key.starts_with('*');
            let mut units = Vec::new();
            for unit in &group.units {
                let pins = unit
                    .iter()
                    .map(|pin| pin_key(key, cross, pin))
                    .collect::<Result<Vec<_>, _>>()?;
                for pin in &pins {
                    if !pads.positions.contains_key(pin) {
                        return Err(format!(
                            "pin-swap group {key}/{}: {}.{} is not a pad on the board",
                            group.name, pin.reference, pin.pad
                        ));
                    }
                    if !seen.insert(pin.clone()) {
                        return Err(format!(
                            "pin {}.{} appears in more than one swap unit",
                            pin.reference, pin.pad
                        ));
                    }
                }
                units.push(Unit { pins });
            }
            if let Some(width) = units.first().map(|unit| unit.pins.len())
                && units.iter().any(|unit| unit.pins.len() != width)
            {
                return Err(format!(
                    "pin-swap group {key}/{}: units have different pin counts",
                    group.name
                ));
            }
            let group_payloads: Vec<Vec<Option<String>>> = units
                .iter()
                .map(|unit| unit.pins.iter().map(|pin| pads.nets[pin].clone()).collect())
                .collect();
            let mut allowed = Vec::new();
            for payload in &group_payloads {
                let mut mask: Vec<bool> = Vec::new();
                for net in payload.iter().flatten() {
                    let Some(pins) = group.restrictions.get(normalize_net(net)) else {
                        continue;
                    };
                    let pins = pins
                        .iter()
                        .map(|pin| pin_key(key, cross, pin))
                        .collect::<Result<BTreeSet<_>, _>>()?;
                    let unit_mask: Vec<bool> = units
                        .iter()
                        .map(|unit| unit.pins.iter().any(|pin| pins.contains(pin)))
                        .collect();
                    mask = if mask.is_empty() {
                        unit_mask
                    } else {
                        mask.iter().zip(unit_mask).map(|(a, b)| *a && b).collect()
                    };
                }
                allowed.push(mask);
            }
            for (index, payload) in group_payloads.iter().enumerate() {
                if !allowed[index].is_empty() && !allowed[index][index] {
                    return Err(format!(
                        "pin-swap group {key}/{}: the board's own assignment violates the restriction of {:?}",
                        group.name, payload
                    ));
                }
            }
            groups.push(Group { units, allowed });
            payloads.push(group_payloads);
        }
    }
    // Nets of the estimate.
    let mut pad_count: HashMap<&str, usize> = HashMap::new();
    for (_, _, net) in &pads.all {
        if let Some(net) = net {
            *pad_count.entry(net.as_str()).or_default() += 1;
        }
    }
    let counted = |net: &Option<String>| {
        !is_unconnected(net)
            && pad_count
                .get(net.as_deref().unwrap_or(""))
                .is_some_and(|count| *count <= config.maximum_net_pads)
    };
    let mut index_of: BTreeMap<String, usize> = BTreeMap::new();
    let mut nets: Vec<NetTerminals> = Vec::new();
    let mut net_index = |name: &str, nets: &mut Vec<NetTerminals>| {
        *index_of.entry(name.to_string()).or_insert_with(|| {
            nets.push(NetTerminals::default());
            nets.len() - 1
        })
    };
    for (key, terminal, net) in &pads.all {
        if seen.contains(key) || !counted(net) {
            continue;
        }
        let index = net_index(net.as_deref().unwrap(), &mut nets);
        nets[index].fixed.push(*terminal);
    }
    let mut payload_nets = Vec::new();
    for (group, group_payloads) in payloads.iter().enumerate() {
        let mut per_payload = Vec::new();
        for (payload, pins) in group_payloads.iter().enumerate() {
            let mut carried = Vec::new();
            for (pin, net) in pins.iter().enumerate() {
                if !counted(net) {
                    continue;
                }
                let index = net_index(net.as_deref().unwrap(), &mut nets);
                nets[index].moving.push((group, payload, pin));
                carried.push(index);
            }
            carried.sort_unstable();
            carried.dedup();
            per_payload.push(carried);
        }
        payload_nets.push(per_payload);
    }
    Ok(Problem {
        groups,
        positions: pads.positions.clone(),
        bodies: pads.bodies.clone(),
        payloads,
        nets,
        payload_nets,
    })
}

/// The net KiCad gives an unconnected pin: named after the schematic pin
/// if known, else the pad's pin function.
fn unconnected_name(
    reference: &str,
    pad: &Expr,
    pad_name: &str,
    pin_names: &HashMap<PinKey, String>,
) -> String {
    let key = PinKey {
        reference: reference.to_string(),
        pad: pad_name.to_string(),
    };
    let function = pin_names
        .get(&key)
        .map(String::as_str)
        .or_else(|| form_atom(pad, "pinfunction", 1));
    match function {
        Some(function) if !function.is_empty() && function != "~" => {
            format!("unconnected-({reference}-{function}-Pad{pad_name})")
        }
        _ => format!("unconnected-({reference}-Pad{pad_name})"),
    }
}

fn quoted(value: &str) -> Expr {
    Expr::Atom(format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")))
}

/// Writes `changes` (pin -> new net, `None` for no connection) into the
/// board. Returns the name each changed pad ended up with.
fn apply_to_board(
    pcb: &mut Expr,
    changes: &HashMap<PinKey, Option<String>>,
    pin_names: &HashMap<PinKey, String>,
) -> Result<HashMap<PinKey, String>, String> {
    // Old-style boards number their nets.
    let mut numbers: BTreeMap<String, String> = BTreeMap::new();
    for item in pcb.children().iter().filter(|item| item.head() == Some("net")) {
        if let (Some(number), Some(name)) = (
            item.children().get(1).and_then(Expr::atom),
            item.children().get(2).and_then(Expr::atom),
        ) {
            numbers.insert(name.to_string(), number.to_string());
        }
    }
    let numbered = !numbers.is_empty();
    let mut next_number = numbers
        .values()
        .filter_map(|number| number.parse::<usize>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    let mut new_declarations: Vec<(String, String)> = Vec::new();
    let mut written = HashMap::new();
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    for footprint in items.iter_mut().filter(|item| item.head() == Some("footprint")) {
        let reference = footprint_reference(footprint).unwrap_or_default();
        let Expr::List(children) = footprint else {
            continue;
        };
        for pad in children.iter_mut().filter(|item| item.head() == Some("pad")) {
            let pad_name = pad.children().get(1).and_then(Expr::atom).unwrap_or("").to_string();
            let key = PinKey {
                reference: reference.clone(),
                pad: pad_name.clone(),
            };
            let Some(net) = changes.get(&key) else {
                continue;
            };
            let name = match net {
                Some(net) if !net.starts_with("unconnected-") => net.clone(),
                _ => unconnected_name(&reference, pad, &pad_name, pin_names),
            };
            let form = if numbered {
                let number = numbers.entry(name.clone()).or_insert_with(|| {
                    let number = next_number.to_string();
                    next_number += 1;
                    new_declarations.push((number.clone(), name.clone()));
                    number
                });
                Expr::List(vec![
                    Expr::Atom("net".into()),
                    Expr::Atom(number.clone()),
                    quoted(&name),
                ])
            } else {
                Expr::List(vec![Expr::Atom("net".into()), quoted(&name)])
            };
            let Expr::List(pad_items) = pad else {
                continue;
            };
            match pad_items.iter_mut().find(|item| item.head() == Some("net")) {
                Some(existing) => *existing = form,
                None => {
                    // Keep the usual order: the net follows the layers.
                    let at = pad_items
                        .iter()
                        .position(|item| item.head() == Some("layers"))
                        .map_or(pad_items.len(), |index| index + 1);
                    pad_items.insert(at, form);
                }
            }
            written.insert(key, name);
        }
    }
    if !new_declarations.is_empty() {
        let at = items
            .iter()
            .rposition(|item| item.head() == Some("net"))
            .map_or(items.len(), |index| index + 1);
        for (offset, (number, name)) in new_declarations.into_iter().enumerate() {
            items.insert(
                at + offset,
                Expr::List(vec![Expr::Atom("net".into()), Expr::Atom(number), quoted(&name)]),
            );
        }
    }
    Ok(written)
}

// ---------------------------------------------------------------------------
// Schematic.

const SCHEMATIC_TOLERANCE: f64 = 0.01;

fn same_point(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < SCHEMATIC_TOLERANCE && (a[1] - b[1]).abs() < SCHEMATIC_TOLERANCE
}

fn on_segment(point: [f64; 2], a: [f64; 2], b: [f64; 2]) -> bool {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length_squared = dx * dx + dy * dy;
    if length_squared == 0.0 {
        return same_point(point, a);
    }
    let t = ((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / length_squared;
    if !(-1.0e-9..=1.0 + 1.0e-9).contains(&t) {
        return false;
    }
    same_point(point, [a[0] + t * dx, a[1] + t * dy])
}

/// The eight orientations a schematic symbol can have, as matrices from
/// library coordinates (Y up) to sheet coordinates (Y down).
fn orientations() -> Vec<[f64; 4]> {
    let mut result = Vec::new();
    for mirror in [1.0, -1.0] {
        for quarter in 0..4 {
            let (sin, cos): (f64, f64) = match quarter {
                0 => (0.0, 1.0),
                1 => (1.0, 0.0),
                2 => (0.0, -1.0),
                _ => (-1.0, 0.0),
            };
            // Mirror in library X, flip Y into the sheet, then rotate.
            let base = [mirror, 0.0, 0.0, -1.0];
            let rotation = [cos, sin, -sin, cos];
            result.push([
                rotation[0] * base[0] + rotation[1] * base[2],
                rotation[0] * base[1] + rotation[1] * base[3],
                rotation[2] * base[0] + rotation[3] * base[2],
                rotation[2] * base[1] + rotation[3] * base[3],
            ]);
        }
    }
    result
}

/// The standard KiCad transform for a placed symbol's `at` angle and
/// `mirror` form.
fn kicad_orientation(angle: f64, mirror: Option<&str>) -> [f64; 4] {
    let (sin, cos) = angle.to_radians().sin_cos();
    let (sin, cos) = (sin.round(), cos.round());
    // Library Y up -> sheet Y down, rotated counter-clockwise on the sheet.
    let base = [1.0, 0.0, 0.0, -1.0];
    let rotation = [cos, sin, -sin, cos];
    let mut matrix = [
        rotation[0] * base[0] + rotation[1] * base[2],
        rotation[0] * base[1] + rotation[1] * base[3],
        rotation[2] * base[0] + rotation[3] * base[2],
        rotation[2] * base[1] + rotation[3] * base[3],
    ];
    match mirror {
        // Mirror about the sheet's X axis: Y flips.
        Some("x") => {
            matrix[2] = -matrix[2];
            matrix[3] = -matrix[3];
        }
        Some("y") => {
            matrix[0] = -matrix[0];
            matrix[1] = -matrix[1];
        }
        _ => {}
    }
    matrix
}

fn transform(matrix: [f64; 4], origin: [f64; 2], local: [f64; 2]) -> [f64; 2] {
    [
        origin[0] + matrix[0] * local[0] + matrix[1] * local[1],
        origin[1] + matrix[2] * local[0] + matrix[3] * local[1],
    ]
}

/// A library pin: unit (0 for all units), number, name, local connection
/// point.
type LibraryPin = (u32, String, String, [f64; 2]);

/// Library pins per symbol id.
fn library_pins(schematic: &Expr) -> BTreeMap<String, Vec<LibraryPin>> {
    let mut result: BTreeMap<String, Vec<LibraryPin>> = BTreeMap::new();
    let Some(library) = schematic.child("lib_symbols") else {
        return result;
    };
    for symbol in library.children().iter().filter(|item| item.head() == Some("symbol")) {
        let Some(id) = symbol.children().get(1).and_then(Expr::atom) else {
            continue;
        };
        let pins = result.entry(id.to_string()).or_default();
        let mut visit = |node: &Expr, unit: u32| {
            for pin in node.children().iter().filter(|item| item.head() == Some("pin")) {
                let (Ok(at), Some(number)) = (form_xy(pin, "at"), form_atom(pin, "number", 1)) else {
                    continue;
                };
                let name = form_atom(pin, "name", 1).unwrap_or("");
                pins.push((unit, number.to_string(), name.to_string(), at));
            }
        };
        visit(symbol, 0);
        for unit_symbol in symbol.children().iter().filter(|item| item.head() == Some("symbol")) {
            // Sub-symbols are named <name>_<unit>_<style>.
            let unit = unit_symbol
                .children()
                .get(1)
                .and_then(Expr::atom)
                .and_then(|name| name.rsplit('_').nth(1))
                .and_then(|unit| unit.parse().ok())
                .unwrap_or(0);
            visit(unit_symbol, unit);
        }
    }
    result
}

struct SchematicPin {
    reference: String,
    number: String,
    name: String,
    at: [f64; 2],
}

fn schematic_pins(schematic: &Expr) -> Result<Vec<SchematicPin>, String> {
    let library = library_pins(schematic);
    // Points where something attaches, to confirm the orientation.
    let mut anchors: Vec<[f64; 2]> = Vec::new();
    for item in schematic.children() {
        match item.head() {
            Some("wire") => {
                if let Some(points) = item.child("pts") {
                    for point in points.children().iter().filter(|p| p.head() == Some("xy")) {
                        if let (Some(x), Some(y)) = (
                            point.children().get(1).and_then(Expr::atom).and_then(|v| v.parse().ok()),
                            point.children().get(2).and_then(Expr::atom).and_then(|v| v.parse().ok()),
                        ) {
                            anchors.push([x, y]);
                        }
                    }
                }
            }
            Some("label" | "global_label" | "hierarchical_label" | "no_connect" | "junction") => {
                if let Ok(at) = form_xy(item, "at") {
                    anchors.push(at);
                }
            }
            _ => {}
        }
    }
    let mut result = Vec::new();
    for symbol in schematic.children().iter().filter(|item| item.head() == Some("symbol")) {
        let Some(id) = form_atom(symbol, "lib_id", 1) else {
            continue;
        };
        let reference = symbol
            .children()
            .iter()
            .find(|item| {
                item.head() == Some("property")
                    && item.children().get(1).and_then(Expr::atom) == Some("Reference")
            })
            .and_then(|item| item.children().get(2).and_then(Expr::atom))
            .unwrap_or("")
            .to_string();
        let at = form_at(symbol)?;
        let unit: u32 = form_atom(symbol, "unit", 1).and_then(|u| u.parse().ok()).unwrap_or(1);
        let pins: Vec<&LibraryPin> = library
            .get(id)
            .map(|pins| pins.iter().filter(|pin| pin.0 == 0 || pin.0 == unit).collect())
            .unwrap_or_default();
        let origin = [at[0], at[1]];
        let standard = kicad_orientation(at[2], form_atom(symbol, "mirror", 1));
        let hits = |matrix: [f64; 4]| {
            pins.iter()
                .filter(|pin| {
                    let point = transform(matrix, origin, pin.3);
                    anchors.iter().any(|anchor| same_point(*anchor, point))
                })
                .count()
        };
        // Trust the standard transform unless another orientation explains
        // clearly more of the attached wires and labels.
        let mut matrix = standard;
        let mut best = hits(standard);
        for candidate in orientations() {
            let count = hits(candidate);
            if count > best {
                best = count;
                matrix = candidate;
            }
        }
        for pin in pins {
            result.push(SchematicPin {
                reference: reference.clone(),
                number: pin.1.clone(),
                name: pin.2.clone(),
                at: transform(matrix, origin, pin.3),
            });
        }
    }
    Ok(result)
}

fn wire_points(wire: &Expr) -> Vec<[f64; 2]> {
    wire.child("pts")
        .map(|points| {
            points
                .children()
                .iter()
                .filter(|point| point.head() == Some("xy"))
                .filter_map(|point| {
                    Some([
                        point.children().get(1)?.atom()?.parse().ok()?,
                        point.children().get(2)?.atom()?.parse().ok()?,
                    ])
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Wires reachable from `start` (end points and points on a segment both
/// connect), skipping `excluded`; returns the wires and every point seen.
fn wire_network(
    start: [f64; 2],
    wires: &[(usize, Vec<[f64; 2]>)],
    excluded: &BTreeSet<usize>,
) -> (BTreeSet<usize>, Vec<[f64; 2]>) {
    let mut points = vec![start];
    let mut visited: BTreeSet<usize> = BTreeSet::new();
    let mut cursor = 0;
    while cursor < points.len() {
        let point = points[cursor];
        cursor += 1;
        for (index, wire) in wires {
            if visited.contains(index) || excluded.contains(index) {
                continue;
            }
            let touches = wire.iter().any(|end| same_point(*end, point))
                || wire.windows(2).any(|pair| on_segment(point, pair[0], pair[1]));
            if touches {
                visited.insert(*index);
                points.extend(wire.iter().copied());
            }
        }
    }
    (visited, points)
}

fn on_network(
    at: [f64; 2],
    points: &[[f64; 2]],
    visited: &BTreeSet<usize>,
    wires: &[(usize, Vec<[f64; 2]>)],
) -> bool {
    points.iter().any(|point| same_point(*point, at))
        || wires
            .iter()
            .filter(|(index, _)| visited.contains(index))
            .any(|(_, wire)| wire.windows(2).any(|pair| on_segment(at, pair[0], pair[1])))
}

fn label_expr(name: &str, at: [f64; 2]) -> Expr {
    Expr::List(vec![
        Expr::Atom("label".into()),
        quoted(name),
        Expr::List(vec![
            Expr::Atom("at".into()),
            Expr::Atom(format_coordinate(at[0])),
            Expr::Atom(format_coordinate(at[1])),
            Expr::Atom("0".into()),
        ]),
        Expr::List(vec![
            Expr::Atom("effects".into()),
            Expr::List(vec![
                Expr::Atom("font".into()),
                Expr::List(vec![
                    Expr::Atom("size".into()),
                    Expr::Atom("1.0".into()),
                    Expr::Atom("1.0".into()),
                ]),
            ]),
        ]),
    ])
}

fn no_connect_expr(at: [f64; 2]) -> Expr {
    Expr::List(vec![
        Expr::Atom("no_connect".into()),
        Expr::List(vec![
            Expr::Atom("at".into()),
            Expr::Atom(format_coordinate(at[0])),
            Expr::Atom(format_coordinate(at[1])),
        ]),
    ])
}

/// Brings the schematic in line with the swapped pins. A pin whose wires
/// only lead to labels gets its labels renamed. A pin wired to other pins
/// is cut loose: the wire run from the pin up to the first label, junction,
/// branch or pin is removed (the rest of the net keeps its name, by a new
/// label at the cut if it had none elsewhere), and the pin gets a label of
/// its new net. Returns the number of pins changed.
fn apply_to_schematic(
    schematic: &mut Expr,
    changes: &HashMap<PinKey, (Option<String>, Option<String>)>,
) -> Result<usize, String> {
    let pins = schematic_pins(schematic)?;
    let items = schematic.children();
    let wires: Vec<(usize, Vec<[f64; 2]>)> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.head() == Some("wire"))
        .map(|(index, item)| (index, wire_points(item)))
        .collect();
    let labels: Vec<(usize, [f64; 2], String)> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            matches!(item.head(), Some("label" | "global_label" | "hierarchical_label"))
        })
        .filter_map(|(index, item)| {
            Some((
                index,
                form_xy(item, "at").ok()?,
                item.children().get(1)?.atom()?.to_string(),
            ))
        })
        .collect();
    let points_of = |head: &str| -> Vec<(usize, [f64; 2])> {
        items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.head() == Some(head))
            .filter_map(|(index, item)| Some((index, form_xy(item, "at").ok()?)))
            .collect()
    };
    let no_connects = points_of("no_connect");
    let junctions = points_of("junction");
    let mut relabel: Vec<(usize, String)> = Vec::new();
    let mut remove: BTreeSet<usize> = BTreeSet::new();
    let mut add: Vec<Expr> = Vec::new();
    let mut changed = 0;
    let mut ordered: Vec<(&PinKey, &(Option<String>, Option<String>))> = changes.iter().collect();
    ordered.sort_by(|a, b| a.0.cmp(b.0));
    for (key, (before, after)) in ordered {
        let pin = pins
            .iter()
            .find(|pin| pin.reference == key.reference && pin.number == key.pad)
            .ok_or_else(|| {
                format!(
                    "pin {}.{} is not in the schematic (only single-sheet schematics are supported)",
                    key.reference, key.pad
                )
            })?;
        let (visited, points) = wire_network(pin.at, &wires, &remove);
        let wired_to_pin = pins
            .iter()
            .any(|other| !std::ptr::eq(other, pin) && on_network(other.at, &points, &visited, &wires));
        let before_connected = !is_unconnected(before);
        let after_connected = !is_unconnected(after);
        if !wired_to_pin {
            let pin_labels: Vec<usize> = labels
                .iter()
                .filter(|(_, at, _)| on_network(*at, &points, &visited, &wires))
                .map(|(index, ..)| *index)
                .collect();
            match (before_connected, after_connected) {
                (true, true) => {
                    if pin_labels.is_empty() {
                        return Err(format!(
                            "pin {}.{} has no label in the schematic",
                            key.reference, key.pad
                        ));
                    }
                    let name = normalize_net(after.as_deref().unwrap()).to_string();
                    for label in pin_labels {
                        relabel.push((label, name.clone()));
                    }
                }
                (true, false) => {
                    remove.extend(pin_labels);
                    remove.extend(visited.iter().copied());
                    add.push(no_connect_expr(pin.at));
                }
                (false, true) => {
                    remove.extend(
                        no_connects
                            .iter()
                            .filter(|(_, at)| same_point(*at, pin.at))
                            .map(|(index, _)| *index),
                    );
                    add.push(label_expr(normalize_net(after.as_deref().unwrap()), pin.at));
                }
                (false, false) => {}
            }
            changed += 1;
            continue;
        }
        // Cut the pin loose. Walk the unbranched wire run from the pin.
        let stops = |at: [f64; 2], removed: &BTreeSet<usize>| {
            labels.iter().any(|(_, point, _)| same_point(*point, at))
                || junctions.iter().any(|(_, point)| same_point(*point, at))
                || pins.iter().any(|other| same_point(other.at, at))
                || wires
                    .iter()
                    .filter(|(index, _)| !removed.contains(index))
                    .filter(|(_, wire)| {
                        wire.iter().any(|end| same_point(*end, at))
                            || wire.windows(2).any(|pair| on_segment(at, pair[0], pair[1]))
                    })
                    .count()
                    != 1
        };
        let mut cut = pin.at;
        let mut run: BTreeSet<usize> = remove.clone();
        loop {
            let incident: Vec<&(usize, Vec<[f64; 2]>)> = wires
                .iter()
                .filter(|(index, _)| !run.contains(index))
                .filter(|(_, wire)| {
                    wire.first().is_some_and(|end| same_point(*end, cut))
                        || wire.last().is_some_and(|end| same_point(*end, cut))
                })
                .collect();
            let [(index, wire)] = incident.as_slice() else {
                break;
            };
            run.insert(*index);
            cut = if same_point(wire[0], cut) {
                *wire.last().unwrap()
            } else {
                wire[0]
            };
            if stops(cut, &run) {
                break;
            }
        }
        let cut_wires: BTreeSet<usize> = run.difference(&remove).copied().collect();
        if cut_wires.is_empty() {
            return Err(format!(
                "pin {}.{} touches another pin directly in the schematic; it cannot be swapped",
                key.reference, key.pad
            ));
        }
        remove.extend(cut_wires);
        if before_connected {
            let (rest, rest_points) = wire_network(cut, &wires, &remove);
            let old = normalize_net(before.as_deref().unwrap());
            let named = labels
                .iter()
                .any(|(_, at, name)| name == old && on_network(*at, &rest_points, &rest, &wires));
            if !named {
                add.push(label_expr(old, cut));
            }
        }
        if after_connected {
            add.push(label_expr(normalize_net(after.as_deref().unwrap()), pin.at));
        } else {
            add.push(no_connect_expr(pin.at));
        }
        changed += 1;
    }
    let Expr::List(items) = schematic else {
        return Err("schematic root is not a list".into());
    };
    for (index, name) in relabel {
        if let Expr::List(label) = &mut items[index]
            && label.len() > 1
        {
            label[1] = quoted(&name);
        }
    }
    let mut index = 0;
    items.retain(|_| {
        let keep = !remove.contains(&index);
        index += 1;
        keep
    });
    let at = items
        .iter()
        .rposition(|item| matches!(item.head(), Some("label" | "wire" | "no_connect")))
        .map_or(items.len(), |index| index + 1);
    for (offset, item) in add.into_iter().enumerate() {
        items.insert(at + offset, item);
    }
    Ok(changed)
}

fn format_coordinate(value: f64) -> String {
    let text = format!("{:.4}", value);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-" {
        "0".into()
    } else {
        text.to_string()
    }
}

/// Chooses an assignment for the board in `pcb` and returns the pin
/// changes (pin -> (net before, net after)) with the estimates.
fn choose(
    pcb: &Expr,
    spec: &KiCadPinSwapSpec,
    config: &KiCadPinSwapConfig,
) -> Result<(HashMap<PinKey, (Option<String>, Option<String>)>, KiCadPinSwapEstimate, KiCadPinSwapEstimate, usize, usize), String> {
    let pads = board_pads(pcb)?;
    let problem = build_problem(spec, &pads, config)?;
    let before = State::new(&problem, config).estimate();
    let state = optimize(&problem, config);
    let after = state.estimate();
    let mut changes = HashMap::new();
    for (group, units) in problem.groups.iter().enumerate() {
        for (unit, members) in units.units.iter().enumerate() {
            let payload = state.payload_on[group][unit];
            if payload == unit {
                continue;
            }
            for (pin, key) in members.pins.iter().enumerate() {
                let before = problem.payloads[group][unit][pin].clone();
                let after = problem.payloads[group][payload][pin].clone();
                let same = before == after || (is_unconnected(&before) && is_unconnected(&after));
                if !same {
                    changes.insert(key.clone(), (before, after));
                }
            }
        }
    }
    let units = problem.groups.iter().map(|group| group.units.len()).sum();
    Ok((changes, before, after, problem.groups.len(), units))
}

/// Pin swapping on the board in `pcb` (in memory) and on the schematic
/// `schematic` (if any). Nets that KiCad names after a pin (`Net-(...)`)
/// cannot move, since their name would change with the schematic.
fn swap_pins(
    pcb: &mut Expr,
    schematic: Option<&mut Expr>,
    spec: &KiCadPinSwapSpec,
    config: &KiCadPinSwapConfig,
) -> Result<KiCadPinSwapResult, String> {
    let started = Instant::now();
    let (changes, before, after, groups, units) = choose(pcb, spec, config)?;
    if let Some((key, (_, after))) = changes
        .iter()
        .find(|(_, (_, after))| after.as_deref().is_some_and(|net| net.contains("Net-(")))
    {
        return Err(format!(
            "net {:?} would move to {}.{}, but its name comes from a pin; name the net in the schematic to swap it",
            after, key.reference, key.pad
        ));
    }
    let targets: HashMap<PinKey, Option<String>> = changes
        .iter()
        .map(|(key, (_, after))| (key.clone(), after.clone()))
        .collect();
    let pin_names: HashMap<PinKey, String> = match &schematic {
        Some(schematic) => schematic_pins(schematic)?
            .into_iter()
            .map(|pin| {
                (
                    PinKey {
                        reference: pin.reference,
                        pad: pin.number,
                    },
                    pin.name,
                )
            })
            .collect(),
        None => HashMap::new(),
    };
    let written = apply_to_board(pcb, &targets, &pin_names)?;
    let schematic_labels_changed = match schematic {
        Some(schematic) => apply_to_schematic(schematic, &changes)?,
        None => 0,
    };
    let mut report: Vec<KiCadPinChange> = changes
        .iter()
        .map(|(key, (before, _))| KiCadPinChange {
            reference: key.reference.clone(),
            pad: key.pad.clone(),
            before: before.clone().unwrap_or_default(),
            after: written.get(key).cloned().unwrap_or_default(),
        })
        .collect();
    report.sort_by(|a, b| {
        (a.reference.as_str(), natural_key(&a.pad)).cmp(&(b.reference.as_str(), natural_key(&b.pad)))
    });
    Ok(KiCadPinSwapResult {
        before,
        after,
        groups,
        units,
        changes: report,
        schematic_labels_changed,
        seconds: started.elapsed().as_secs_f64(),
    })
}

fn natural_key(pad: &str) -> (u64, String) {
    (pad.parse().unwrap_or(u64::MAX), pad.to_string())
}

pub fn read_pin_swap_spec(path: &Path) -> Result<KiCadPinSwapSpec, String> {
    serde_json::from_str(
        &fs::read_to_string(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

/// Copies the project in `source_directory` to `output_directory` with the
/// nets on interchangeable pins permuted, in the board and the schematic,
/// and writes `pin-swaps-result.json` there.
pub fn swap_kicad_pins(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    spec: &KiCadPinSwapSpec,
    config: &KiCadPinSwapConfig,
) -> Result<KiCadPinSwapResult, String> {
    if output_directory.exists() {
        fs::remove_dir_all(output_directory)
            .map_err(|error| format!("failed to clear {}: {error}", output_directory.display()))?;
    }
    if let Some(parent) = output_directory.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    copy_directory_tree(source_directory, output_directory)?;
    swap_kicad_pins_in_place(output_directory, board_id, spec, config)
}

/// As [swap_kicad_pins], rewriting the project in `directory` itself.
pub fn swap_kicad_pins_in_place(
    directory: &Path,
    board_id: &str,
    spec: &KiCadPinSwapSpec,
    config: &KiCadPinSwapConfig,
) -> Result<KiCadPinSwapResult, String> {
    let board_path = directory.join(format!("{board_id}.kicad_pcb"));
    let schematic_path = directory.join(format!("{board_id}.kicad_sch"));
    let mut pcb = parse(
        &fs::read_to_string(&board_path)
            .map_err(|error| format!("failed to read {}: {error}", board_path.display()))?,
    )?;
    let mut schematic = if schematic_path.exists() {
        Some(parse(&fs::read_to_string(&schematic_path).map_err(|error| {
            format!("failed to read {}: {error}", schematic_path.display())
        })?)?)
    } else {
        None
    };
    let result = swap_pins(&mut pcb, schematic.as_mut(), spec, config)?;
    fs::write(&board_path, format!("{}\n", encode(&pcb)))
        .map_err(|error| format!("failed to write {}: {error}", board_path.display()))?;
    if let Some(schematic) = &schematic {
        fs::write(&schematic_path, format!("{}\n", encode(schematic)))
            .map_err(|error| format!("failed to write {}: {error}", schematic_path.display()))?;
    }
    let report = directory.join("pin-swaps-result.json");
    fs::write(
        &report,
        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", report.display()))?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(pads: &[(&str, &str, f64, f64, &str)]) -> Expr {
        // (reference, pad, x, y, net) with every footprint at the origin.
        let mut references: Vec<&str> = pads.iter().map(|pad| pad.0).collect();
        references.dedup();
        let mut text = String::from("(kicad_pcb");
        for reference in references {
            text.push_str(&format!(
                " (footprint \"X\" (at 0 0) (property \"Reference\" \"{reference}\")"
            ));
            for (owner, pad, x, y, net) in pads {
                if *owner == reference {
                    text.push_str(&format!(
                        " (pad \"{pad}\" smd rect (at {x} {y}) (size 0.5 0.5) (layers \"F.Cu\") (net \"{net}\"))"
                    ));
                }
            }
            text.push(')');
        }
        text.push(')');
        parse(&text).unwrap()
    }

    fn net_of(pcb: &Expr, reference: &str, pad: &str) -> String {
        board_pads(pcb).unwrap().nets[&PinKey {
            reference: reference.into(),
            pad: pad.into(),
        }]
            .clone()
            .unwrap()
    }

    #[test]
    fn crossed_pins_are_uncrossed() {
        // U1 pins 1 (top) and 2 (bottom) face J1 pins A (bottom) and
        // B (top): as assigned, the two ratsnest lines cross.
        let mut pcb = board(&[
            ("U1", "1", 0.0, 0.0, "/A"),
            ("U1", "2", 0.0, 10.0, "/B"),
            ("J1", "a", 20.0, 10.0, "/A"),
            ("J1", "b", 20.0, 0.0, "/B"),
        ]);
        let spec: KiCadPinSwapSpec = serde_json::from_str(
            r#"{"version": 1, "components": {"U1": {"groups": [{"name": "gpio", "units": [["1"], ["2"]]}]}}}"#,
        )
        .unwrap();
        let result = swap_pins(&mut pcb, None, &spec, &KiCadPinSwapConfig::default()).unwrap();
        assert_eq!(result.before.crossings, 1);
        assert_eq!(result.after.crossings, 0);
        assert_eq!(net_of(&pcb, "U1", "1"), "/B");
        assert_eq!(net_of(&pcb, "U1", "2"), "/A");
        assert_eq!(result.changes.len(), 2);
    }

    #[test]
    fn pins_do_not_reach_across_their_chip() {
        // A four-pad chip: net A leaves on the left but goes to J1 on the
        // right, net B the other way round. As straight lines the two only
        // overlap; as escapes they run through the chip twice.
        let mut pcb = board(&[
            ("U1", "1", -1.0, 0.0, "/A"),
            ("U1", "2", 1.0, 0.0, "/B"),
            ("U1", "3", 0.0, -1.0, "/C"),
            ("U1", "4", 0.0, 1.0, "/D"),
            ("J1", "a", 10.0, 0.5, "/A"),
            ("J2", "b", -10.0, 0.5, "/B"),
            ("J3", "c", 0.0, -10.0, "/C"),
            ("J4", "d", 0.0, 10.0, "/D"),
        ]);
        let spec: KiCadPinSwapSpec = serde_json::from_str(
            r#"{"version": 1, "components": {"U1": {"groups": [{"name": "gpio", "units": [["1"], ["2"]]}]}}}"#,
        )
        .unwrap();
        let result = swap_pins(&mut pcb, None, &spec, &KiCadPinSwapConfig::default()).unwrap();
        assert_eq!(result.before.body_crossings, 2);
        assert_eq!(result.after.body_crossings, 0);
        assert_eq!(net_of(&pcb, "U1", "1"), "/B");
        assert_eq!(net_of(&pcb, "U1", "2"), "/A");
    }

    #[test]
    fn restrictions_are_honoured() {
        let mut pcb = board(&[
            ("U1", "1", 0.0, 0.0, "/A"),
            ("U1", "2", 0.0, 10.0, "/B"),
            ("J1", "a", 20.0, 10.0, "/A"),
            ("J1", "b", 20.0, 0.0, "/B"),
        ]);
        let spec: KiCadPinSwapSpec = serde_json::from_str(
            r#"{"version": 1, "components": {"U1": {"groups": [{"name": "gpio", "units": [["1"], ["2"]],
                "restrictions": {"A": ["1"]}}]}}}"#,
        )
        .unwrap();
        let result = swap_pins(&mut pcb, None, &spec, &KiCadPinSwapConfig::default()).unwrap();
        assert!(result.changes.is_empty());
        assert_eq!(net_of(&pcb, "U1", "1"), "/A");
    }

    #[test]
    fn units_move_together_across_components() {
        // Two single-element networks: element on RN1 carries (A, MA),
        // the one on RN2 carries (B, MB). A and B end at J1, MA and MB at
        // U1, both crossed; exchanging the elements uncrosses both.
        let mut pcb = board(&[
            ("RN1", "1", 10.0, 0.0, "/A"),
            ("RN1", "2", 12.0, 0.0, "/MA"),
            ("RN2", "1", 10.0, 10.0, "/B"),
            ("RN2", "2", 12.0, 10.0, "/MB"),
            ("J1", "a", 0.0, 10.0, "/A"),
            ("J1", "b", 0.0, 0.0, "/B"),
            ("U1", "1", 22.0, 10.0, "/MA"),
            ("U1", "2", 22.0, 0.0, "/MB"),
        ]);
        let spec: KiCadPinSwapSpec = serde_json::from_str(
            r#"{"version": 1, "components": {"*networks": {"groups": [{"name": "r", "cross_component": true,
                "units": [["RN1:1", "RN1:2"], ["RN2:1", "RN2:2"]]}]}}}"#,
        )
        .unwrap();
        let result = swap_pins(&mut pcb, None, &spec, &KiCadPinSwapConfig::default()).unwrap();
        assert_eq!(result.before.crossings, 2);
        assert_eq!(result.after.crossings, 0);
        assert_eq!(net_of(&pcb, "RN1", "1"), "/B");
        assert_eq!(net_of(&pcb, "RN1", "2"), "/MB");
    }

    #[test]
    fn schematic_labels_follow_the_pins() {
        let mut pcb = board(&[
            ("U1", "1", 0.0, 0.0, "/A"),
            ("U1", "2", 0.0, 10.0, "/B"),
            ("J1", "a", 20.0, 10.0, "/A"),
            ("J1", "b", 20.0, 0.0, "/B"),
        ]);
        // U1's pins at library (-5, 1) and (-5, -1); the symbol at
        // (100, 100) puts them at (95, 99) and (95, 101). Each has a stub
        // wire to a label.
        let mut schematic = parse(
            r#"(kicad_sch
              (lib_symbols (symbol "L:U" (symbol "U_0_1"
                (pin bidirectional line (at -5 1 0) (length 2.54) (number "1"))
                (pin bidirectional line (at -5 -1 0) (length 2.54) (number "2")))))
              (symbol (lib_id "L:U") (at 100 100 0) (unit 1) (property "Reference" "U1"))
              (wire (pts (xy 95 99) (xy 92 99)))
              (label "A" (at 92 99 180))
              (wire (pts (xy 95 101) (xy 92 101)))
              (label "B" (at 92 101 180)))"#,
        )
        .unwrap();
        let spec: KiCadPinSwapSpec = serde_json::from_str(
            r#"{"version": 1, "components": {"U1": {"groups": [{"name": "gpio", "units": [["1"], ["2"]]}]}}}"#,
        )
        .unwrap();
        let result =
            swap_pins(&mut pcb, Some(&mut schematic), &spec, &KiCadPinSwapConfig::default()).unwrap();
        assert_eq!(result.schematic_labels_changed, 2);
        let labels: Vec<(String, [f64; 2])> = schematic
            .children()
            .iter()
            .filter(|item| item.head() == Some("label"))
            .map(|item| {
                (
                    item.children()[1].atom().unwrap().to_string(),
                    form_xy(item, "at").unwrap(),
                )
            })
            .collect();
        assert_eq!(labels, vec![("B".to_string(), [92.0, 99.0]), ("A".to_string(), [92.0, 101.0])]);
    }

    #[test]
    fn pins_wired_to_other_pins_are_cut_loose() {
        let mut pcb = board(&[
            ("U1", "1", 0.0, 0.0, "/A"),
            ("U1", "2", 0.0, 10.0, "/B"),
            ("J1", "a", 20.0, 10.0, "/A"),
            ("J1", "b", 20.0, 0.0, "/B"),
        ]);
        // U1.1 is wired straight to J1.a (net A has no label at all);
        // U1.2 has a stub and a label.
        let mut schematic = parse(
            r#"(kicad_sch
              (lib_symbols
                (symbol "L:U" (symbol "U_0_1"
                  (pin bidirectional line (at -5 1 0) (length 2.54) (number "1"))
                  (pin bidirectional line (at -5 -1 0) (length 2.54) (number "2"))))
                (symbol "L:J" (symbol "J_0_1"
                  (pin passive line (at 0 0 0) (length 2.54) (number "a")))))
              (symbol (lib_id "L:U") (at 100 100 0) (unit 1) (property "Reference" "U1"))
              (symbol (lib_id "L:J") (at 80 99 0) (unit 1) (property "Reference" "J1"))
              (wire (pts (xy 95 99) (xy 88 99)))
              (wire (pts (xy 88 99) (xy 80 99)))
              (wire (pts (xy 95 101) (xy 92 101)))
              (label "B" (at 92 101 180)))"#,
        )
        .unwrap();
        let spec: KiCadPinSwapSpec = serde_json::from_str(
            r#"{"version": 1, "components": {"U1": {"groups": [{"name": "gpio", "units": [["1"], ["2"]]}]}}}"#,
        )
        .unwrap();
        let result =
            swap_pins(&mut pcb, Some(&mut schematic), &spec, &KiCadPinSwapConfig::default()).unwrap();
        assert_eq!(result.schematic_labels_changed, 2);
        let wires = schematic
            .children()
            .iter()
            .filter(|item| item.head() == Some("wire"))
            .count();
        // Both segments of the run to J1 are gone; the stub of U1.2 stays.
        assert_eq!(wires, 1);
        let mut labels: Vec<(String, [f64; 2])> = schematic
            .children()
            .iter()
            .filter(|item| item.head() == Some("label"))
            .map(|item| {
                (
                    item.children()[1].atom().unwrap().to_string(),
                    form_xy(item, "at").unwrap(),
                )
            })
            .collect();
        labels.sort_by(|a, b| a.1[0].total_cmp(&b.1[0]).then(a.1[1].total_cmp(&b.1[1])));
        assert_eq!(
            labels,
            vec![
                // J1.a keeps net A through a new label at the cut.
                ("A".to_string(), [80.0, 99.0]),
                ("A".to_string(), [92.0, 101.0]),
                ("B".to_string(), [95.0, 99.0]),
            ]
        );
    }
}
