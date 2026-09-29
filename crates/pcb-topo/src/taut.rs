// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The shortest path through a channel of the triangulation that keeps
//! every vertex of the channel at its own distance: a taut wire. Each
//! vertex is a disc (its obstacle, the clearance, half the wire's width,
//! the wires nested inside), on the side of the wire the channel says. The
//! path is tangent segments between the discs it touches and arcs around
//! them.
//!
//! The discs touched are first guessed by the funnel algorithm on portals
//! shrunk by the radii, then corrected with exact distances: a disc the
//! path comes too close to is added, a disc the path turns away from is
//! dropped, until nothing changes.

pub type Point = [f64; 2];

pub fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}

pub fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1]]
}

pub fn scale(a: Point, factor: f64) -> Point {
    [a[0] * factor, a[1] * factor]
}

pub fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

pub fn cross(a: Point, b: Point) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

pub fn length(a: Point) -> f64 {
    dot(a, a).sqrt()
}

pub fn distance(a: Point, b: Point) -> f64 {
    length(sub(a, b))
}

pub fn lerp(a: Point, b: Point, t: f64) -> Point {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

pub fn point_segment_distance(point: Point, a: Point, b: Point) -> f64 {
    let d = sub(b, a);
    let span = dot(d, d);
    if span < 1.0e-24 {
        return distance(point, a);
    }
    let t = (dot(sub(point, a), d) / span).clamp(0.0, 1.0);
    distance(point, lerp(a, b, t))
}

/// A disc the wire passes on one side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Disc {
    pub center: Point,
    pub radius: f64,
    /// +1: the disc lies to the left of the wire, -1: to the right.
    pub side: f64,
    /// The channel vertex the disc stands for (`usize::MAX` for the ends).
    pub vertex: usize,
}

impl Disc {
    pub fn point(at: Point) -> Self {
        Self { center: at, radius: 0.0, side: 1.0, vertex: usize::MAX }
    }
}

/// The directed tangent from `a` to `b` with each disc on its side: the
/// touch points on `a` and on `b`.
pub fn tangent(a: &Disc, b: &Disc) -> Option<(Point, Point)> {
    let d = sub(b.center, a.center);
    let span = length(d);
    // With `n` the left normal of the tangent's direction, each centre lies
    // at its signed distance side * radius: n . (b - a) = k.
    let k = b.side * b.radius - a.side * a.radius;
    if span < 1.0e-12 || k.abs() > span {
        return None;
    }
    let theta = d[1].atan2(d[0]) + (k / span).clamp(-1.0, 1.0).acos();
    let n = [theta.cos(), theta.sin()];
    let p = sub(a.center, scale(n, a.side * a.radius));
    let q = sub(b.center, scale(n, b.side * b.radius));
    Some((p, q))
}

/// A path through discs: `discs[0]` and the last are its end points (zero
/// radius); `touch[i]` is where the path meets disc `i` and leaves it.
#[derive(Clone, Debug)]
pub struct Path {
    pub discs: Vec<Disc>,
    pub touch: Vec<(Point, Point)>,
}

/// Signed turn from direction `a` to `b`, in (-pi, pi].
fn turn(a: Point, b: Point) -> f64 {
    cross(a, b).atan2(dot(a, b))
}

/// Angle swept around `disc` from `from` to `to`, positive
/// counter-clockwise, in the direction the side prescribes.
pub fn sweep(disc: &Disc, from: Point, to: Point) -> f64 {
    let a0 = (from[1] - disc.center[1]).atan2(from[0] - disc.center[0]);
    let a1 = (to[1] - disc.center[1]).atan2(to[0] - disc.center[0]);
    let tau = std::f64::consts::TAU;
    if disc.side > 0.0 { (a1 - a0).rem_euclid(tau) } else { -(a0 - a1).rem_euclid(tau) }
}

pub enum Built {
    Path(Path),
    /// Two consecutive discs overlap: no tangent between discs `i` and `i+1`.
    Overlap(usize),
    /// The path turns away from disc `i`: it does not touch it.
    WrongTurn(usize),
}

/// The tangents and arcs through `discs` (end points included).
pub fn build(discs: &[Disc]) -> Built {
    let count = discs.len();
    let mut touch = vec![(discs[0].center, discs[0].center); count];
    touch[count - 1] = (discs[count - 1].center, discs[count - 1].center);
    let mut directions = Vec::with_capacity(count - 1);
    for index in 0..count - 1 {
        let Some((p, q)) = tangent(&discs[index], &discs[index + 1]) else {
            return Built::Overlap(index);
        };
        touch[index].1 = p;
        touch[index + 1].0 = q;
        directions.push(sub(q, p));
    }
    for index in 1..count - 1 {
        let (a, b) = (directions[index - 1], directions[index]);
        if turn(a, b) * discs[index].side < -1.0e-9 {
            return Built::WrongTurn(index);
        }
    }
    Built::Path(Path { discs: discs.to_vec(), touch })
}

/// The closest approach of `path` to `point`: the distance, and where (the
/// tangent after disc `i`, or the arc of disc `i`).
#[derive(Clone, Copy, Debug)]
pub enum Where {
    Tangent(usize),
    Arc(usize),
}

/// The path's pieces in order: the arc of each inner disc with a radius,
/// the tangent after each disc.
fn elements(path: &Path) -> Vec<Where> {
    let count = path.discs.len();
    let mut list = Vec::with_capacity(2 * count);
    for index in 0..count {
        if index > 0 && index + 1 < count && path.discs[index].radius > 0.0 {
            list.push(Where::Arc(index));
        }
        if index + 1 < count {
            list.push(Where::Tangent(index));
        }
    }
    list
}

/// Distance from `point` to one piece of the path.
fn element_distance(path: &Path, element: Where, point: Point) -> f64 {
    match element {
        Where::Tangent(index) => point_segment_distance(point, path.touch[index].1, path.touch[index + 1].0),
        Where::Arc(index) => {
            let disc = &path.discs[index];
            let (from, to) = path.touch[index];
            let arc = sweep(disc, from, to);
            let angle = (point[1] - disc.center[1]).atan2(point[0] - disc.center[0]);
            let a0 = (from[1] - disc.center[1]).atan2(from[0] - disc.center[0]);
            let offset = if disc.side > 0.0 {
                (angle - a0).rem_euclid(std::f64::consts::TAU)
            } else {
                -(a0 - angle).rem_euclid(std::f64::consts::TAU)
            };
            let within = if arc >= 0.0 { offset <= arc } else { offset >= arc };
            if within {
                (distance(point, disc.center) - disc.radius).abs()
            } else {
                distance(point, from).min(distance(point, to))
            }
        }
    }
}

/// The direction of the path at the element `element` (at the point of it
/// nearest to `near`), and the discs that element belongs to (without the
/// end points).
pub fn element(path: &Path, element: Where, near: Point) -> (Point, Vec<Disc>) {
    let inner = |index: usize| path.discs.get(index).copied().filter(|disc| disc.vertex != usize::MAX && disc.radius > 0.0);
    match element {
        Where::Tangent(index) => {
            let direction = sub(path.touch[index + 1].0, path.touch[index].1);
            (direction, [inner(index), inner(index + 1)].into_iter().flatten().collect())
        }
        Where::Arc(index) => {
            let disc = path.discs[index];
            let radial = sub(near, disc.center);
            // Left discs are passed counter-clockwise.
            let direction = if disc.side > 0.0 { [-radial[1], radial[0]] } else { [radial[1], -radial[0]] };
            (direction, inner(index).into_iter().collect())
        }
    }
}

pub fn closest(path: &Path, point: Point) -> (f64, Where) {
    let mut best = (f64::INFINITY, Where::Tangent(0));
    for element in elements(path) {
        let d = element_distance(path, element, point);
        if d < best.0 {
            best = (d, element);
        }
    }
    best
}

fn segments_cross(a: Point, b: Point, c: Point, d: Point) -> bool {
    let (d1, d2) = (cross(sub(b, a), sub(c, a)), cross(sub(b, a), sub(d, a)));
    let (d3, d4) = (cross(sub(d, c), sub(a, c)), cross(sub(d, c), sub(b, c)));
    d1 * d2 <= 0.0 && d3 * d4 <= 0.0
}

/// The closest approach of `path` to `point` where the path crosses the
/// segment `a`–`b` (the pieces crossing it and their neighbours), or of
/// the whole path if it does not cross it.
pub fn closest_near(path: &Path, point: Point, a: Point, b: Point) -> f64 {
    let list = elements(path);
    let crosses = |element: Where| match element {
        Where::Tangent(index) => segments_cross(path.touch[index].1, path.touch[index + 1].0, a, b),
        Where::Arc(index) => {
            let disc = &path.discs[index];
            let (from, to) = path.touch[index];
            let arc = sweep(disc, from, to);
            let a0 = (from[1] - disc.center[1]).atan2(from[0] - disc.center[0]);
            let mut previous = from;
            (1..=8).any(|step| {
                let angle = a0 + arc * step as f64 / 8.0;
                let next = [disc.center[0] + disc.radius * angle.cos(), disc.center[1] + disc.radius * angle.sin()];
                let hit = segments_cross(previous, next, a, b);
                previous = next;
                hit
            })
        }
    };
    let mut best = f64::INFINITY;
    for (position, &element) in list.iter().enumerate() {
        if !crosses(element) {
            continue;
        }
        for near in list.iter().take((position + 3).min(list.len())).skip(position.saturating_sub(2)) {
            best = best.min(element_distance(path, *near, point));
        }
    }
    if best.is_finite() { best } else { closest(path, point).0 }
}

/// Total length of the path.
pub fn path_length(path: &Path) -> f64 {
    let count = path.discs.len();
    let mut total = 0.0;
    for index in 0..count {
        if index > 0 && index + 1 < count {
            let (from, to) = path.touch[index];
            total += sweep(&path.discs[index], from, to).abs() * path.discs[index].radius;
        }
        if index + 1 < count {
            total += distance(path.touch[index].1, path.touch[index + 1].0);
        }
    }
    total
}

/// The path as a polyline; arcs become polylines circumscribing them,
/// straying at most `bulge` outside.
pub fn polyline(path: &Path, bulge: f64) -> Vec<Point> {
    let count = path.discs.len();
    let mut points = vec![path.discs[0].center];
    let push = |points: &mut Vec<Point>, point: Point| {
        if points.last().is_none_or(|last| distance(*last, point) > 1.0e-9) {
            points.push(point);
        }
    };
    for index in 1..count {
        let disc = &path.discs[index];
        let (from, to) = path.touch[index];
        push(&mut points, from);
        if index + 1 < count && disc.radius > 0.0 {
            let arc = sweep(disc, from, to);
            // Half the step angle: the polygon's corners lie at
            // radius / cos(half), `bulge` outside the arc.
            let half = (disc.radius / (disc.radius + bulge)).clamp(-1.0, 1.0).acos().max(1.0e-3);
            let pieces = ((arc.abs() / (2.0 * half)).ceil() as usize).max(1);
            if arc.abs() > 1.0e-9 {
                let step = arc / pieces as f64;
                let outer = disc.radius / (step.abs() / 2.0).cos();
                let a0 = (from[1] - disc.center[1]).atan2(from[0] - disc.center[0]);
                for piece in 0..pieces {
                    let angle = a0 + step * (piece as f64 + 0.5);
                    push(&mut points, [disc.center[0] + outer * angle.cos(), disc.center[1] + outer * angle.sin()]);
                }
            }
            push(&mut points, to);
        }
    }
    points
}

/// One portal of a channel: its two vertices (left and right of the
/// wire, looking along it) with the distance the wire keeps from each.
#[derive(Clone, Copy, Debug)]
pub struct Portal {
    pub left: Point,
    pub right: Point,
    pub left_vertex: usize,
    pub right_vertex: usize,
    pub left_radius: f64,
    pub right_radius: f64,
}

/// The funnel algorithm on portals shrunk by the radii: the discs the path
/// most likely touches, in order. A squeezed portal (radii longer than the
/// portal) is narrowed to its middle.
fn funnel(start: Point, end: Point, portals: &[Portal]) -> Vec<Disc> {
    // (left point, right point, left disc, right disc)
    let mut points: Vec<(Point, Point, Option<Disc>, Option<Disc>)> = Vec::with_capacity(portals.len() + 2);
    points.push((start, start, None, None));
    for portal in portals {
        let span = distance(portal.left, portal.right).max(1.0e-12);
        let (mut a, mut b) = (portal.left_radius / span, 1.0 - portal.right_radius / span);
        if a > b {
            let middle = (a + b) / 2.0;
            a = middle.clamp(0.0, 1.0);
            b = a;
        }
        points.push((
            lerp(portal.left, portal.right, a),
            lerp(portal.left, portal.right, b),
            Some(Disc { center: portal.left, radius: portal.left_radius, side: 1.0, vertex: portal.left_vertex }),
            Some(Disc { center: portal.right, radius: portal.right_radius, side: -1.0, vertex: portal.right_vertex }),
        ));
    }
    points.push((end, end, None, None));
    let mut discs = Vec::new();
    let mut apex = start;
    let (mut left, mut right) = (start, start);
    let (mut left_index, mut right_index) = (0usize, 0usize);
    let mut index = 1;
    let mut guard = 0;
    while index < points.len() {
        guard += 1;
        if guard > 1_000_000 {
            break;
        }
        let (portal_left, portal_right, _, _) = points[index];
        // The right side: tighten, or cross over the left.
        if cross(sub(right, apex), sub(portal_right, apex)) >= 0.0 {
            if distance(apex, right) < 1.0e-12 || cross(sub(left, apex), sub(portal_right, apex)) < 0.0 {
                right = portal_right;
                right_index = index;
            } else {
                if let Some(disc) = points[left_index].2 {
                    discs.push(disc);
                }
                apex = left;
                (left, right) = (apex, apex);
                right_index = left_index;
                index = left_index + 1;
                continue;
            }
        }
        // The left side.
        if cross(sub(left, apex), sub(portal_left, apex)) <= 0.0 {
            if distance(apex, left) < 1.0e-12 || cross(sub(right, apex), sub(portal_left, apex)) > 0.0 {
                left = portal_left;
                left_index = index;
            } else {
                if let Some(disc) = points[right_index].3 {
                    discs.push(disc);
                }
                apex = right;
                (left, right) = (apex, apex);
                left_index = right_index;
                index = right_index + 1;
                continue;
            }
        }
        index += 1;
    }
    discs
}

/// Why no taut path was found.
#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    /// Two discs on opposite sides overlap (or an end lies in a disc): the
    /// channel is too narrow there, `need` against `have` millimetres.
    Squeezed { vertices: (usize, usize), at: Point, need: f64, have: f64 },
    /// The refinement did not settle.
    Unsettled,
}

/// The taut path from `start` to `end` through `portals`.
pub fn taut(start: Point, end: Point, portals: &[Portal]) -> Result<Path, Failure> {
    taut_with(start, end, portals, &[])
}

/// The taut path through `portals` that also keeps clear of `extra` discs
/// (on their sides).
pub fn taut_with(start: Point, end: Point, portals: &[Portal], extra: &[Disc]) -> Result<Path, Failure> {
    let mut inner: Vec<Disc> = Vec::new();
    for disc in funnel(start, end, portals) {
        push_disc(&mut inner, disc, start, end);
    }
    // Every vertex of the channel, with its side and radius.
    let mut vertices: Vec<Disc> = Vec::new();
    for portal in portals {
        for disc in [
            Disc { center: portal.left, radius: portal.left_radius, side: 1.0, vertex: portal.left_vertex },
            Disc { center: portal.right, radius: portal.right_radius, side: -1.0, vertex: portal.right_vertex },
        ] {
            if disc.radius > 0.0 && !vertices.iter().any(|known| known.vertex == disc.vertex) {
                vertices.push(disc);
            }
        }
    }
    for disc in extra {
        match vertices.iter_mut().find(|known| known.vertex == disc.vertex) {
            Some(known) => known.radius = known.radius.max(disc.radius),
            None => vertices.push(*disc),
        }
    }
    let mut seen: Vec<Vec<usize>> = Vec::new();
    for _ in 0..400 {
        let mut discs = Vec::with_capacity(inner.len() + 2);
        discs.push(Disc::point(start));
        discs.extend(inner.iter().copied());
        discs.push(Disc::point(end));
        let path = match build(&discs) {
            Built::Path(path) => path,
            Built::WrongTurn(index) => {
                inner.remove(index - 1);
                continue;
            }
            Built::Overlap(index) => {
                let (a, b) = (discs[index], discs[index + 1]);
                // A disc inside its neighbour on the same side is covered by
                // it; an end point inside a disc, or discs on opposite
                // sides that overlap, leave no room.
                let ends = a.vertex == usize::MAX || b.vertex == usize::MAX;
                if a.side == b.side && !ends {
                    let drop = if a.radius < b.radius { index } else { index + 1 };
                    inner.remove(drop - 1);
                    continue;
                }
                return Err(Failure::Squeezed {
                    vertices: (a.vertex, b.vertex),
                    at: lerp(a.center, b.center, 0.5),
                    need: (b.side * b.radius - a.side * a.radius).abs(),
                    have: distance(a.center, b.center),
                });
            }
        };
        // The vertex the path comes closest to, relative to its radius.
        let mut worst: Option<(f64, Disc, Where)> = None;
        for vertex in &vertices {
            if inner.iter().any(|disc| disc.vertex == vertex.vertex) {
                continue;
            }
            let (d, place) = closest(&path, vertex.center);
            let deficit = vertex.radius - d;
            if deficit > 1.0e-7 && worst.as_ref().is_none_or(|w| deficit > w.0) {
                worst = Some((deficit, *vertex, place));
            }
        }
        let Some((_, vertex, place)) = worst else {
            return Ok(path);
        };
        let at = match place {
            Where::Tangent(index) => index,
            Where::Arc(index) => {
                // Before or after the disc, by where the vertex lies along
                // the path's direction there.
                let incoming = sub(path.touch[index].0, path.touch[index - 1].1);
                let outgoing = sub(path.touch[index + 1].0, path.touch[index].1);
                let unit = |v: Point| scale(v, 1.0 / length(v).max(1.0e-12));
                let direction = add(unit(incoming), unit(outgoing));
                if dot(sub(vertex.center, path.discs[index].center), direction) < 0.0 { index - 1 } else { index }
            }
        };
        inner.insert(at.min(inner.len()), vertex);
        let signature: Vec<usize> = inner.iter().map(|disc| disc.vertex).collect();
        if seen.contains(&signature) {
            if std::env::var_os("PCB_TOPO_TAUT").is_some() {
                eprintln!("    taut unsettled from {start:?} to {end:?}; inserted {} ({:?} r {:.3} side {}) at {at}", vertex.vertex, vertex.center, vertex.radius, vertex.side);
                for (index, known) in seen.iter().enumerate().rev().take(6) {
                    eprintln!("      {index}: {known:?}");
                }
                for disc in &vertices {
                    eprintln!("      vertex {} at {:?} r {:.3} side {}", disc.vertex, disc.center, disc.radius, disc.side);
                }
            }
            return Err(Failure::Unsettled);
        }
        seen.push(signature);
    }
    Err(Failure::Unsettled)
}

/// Appends `disc` unless it sits on an end point or repeats the last one
/// (then the larger radius stays).
fn push_disc(discs: &mut Vec<Disc>, disc: Disc, start: Point, end: Point) {
    if distance(disc.center, start) < 1.0e-9 || distance(disc.center, end) < 1.0e-9 || disc.radius <= 0.0 {
        return;
    }
    if let Some(last) = discs.last_mut()
        && last.vertex == disc.vertex
    {
        last.radius = last.radius.max(disc.radius);
        return;
    }
    discs.push(disc);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Point, b: Point) -> bool {
        distance(a, b) < 1.0e-9
    }

    #[test]
    fn tangents_touch_both_discs_on_their_sides() {
        let sides = [1.0, -1.0];
        for &sa in &sides {
            for &sb in &sides {
                let a = Disc { center: [0.0, 0.0], radius: 1.0, side: sa, vertex: 0 };
                let b = Disc { center: [10.0, 2.0], radius: 2.0, side: sb, vertex: 1 };
                let (p, q) = tangent(&a, &b).unwrap();
                assert!((distance(p, a.center) - 1.0).abs() < 1.0e-9);
                assert!((distance(q, b.center) - 2.0).abs() < 1.0e-9);
                let direction = sub(q, p);
                // Perpendicular to both radii.
                assert!(dot(direction, sub(p, a.center)).abs() < 1.0e-9);
                assert!(dot(direction, sub(q, b.center)).abs() < 1.0e-9);
                // Each centre on its side.
                assert!(cross(direction, sub(a.center, p)) * sa > 0.0);
                assert!(cross(direction, sub(b.center, q)) * sb > 0.0);
            }
        }
    }

    #[test]
    fn a_point_to_a_disc_inside_has_no_tangent() {
        let a = Disc::point([0.5, 0.0]);
        let b = Disc { center: [0.0, 0.0], radius: 1.0, side: 1.0, vertex: 0 };
        assert!(tangent(&a, &b).is_none());
    }

    /// A channel along the x axis from 0 to `n`, one portal per unit, with
    /// the given left (y > 0) and right (y < 0) vertices.
    fn channel(lefts: &[(f64, f64, f64)], rights: &[(f64, f64, f64)]) -> Vec<Portal> {
        // Portals pair each left vertex with each right vertex in x order,
        // as a triangulation strip would.
        let mut portals = Vec::new();
        let (mut i, mut j) = (0, 0);
        loop {
            let l = lefts[i];
            let r = rights[j];
            portals.push(Portal {
                left: [l.0, l.1],
                right: [r.0, r.1],
                left_vertex: i,
                right_vertex: 100 + j,
                left_radius: l.2,
                right_radius: r.2,
            });
            if i + 1 == lefts.len() && j + 1 == rights.len() {
                break;
            }
            if j + 1 == rights.len() || (i + 1 < lefts.len() && lefts[i + 1].0 <= rights[j + 1].0) {
                i += 1;
            } else {
                j += 1;
            }
        }
        portals
    }

    #[test]
    fn a_wide_channel_gives_a_straight_path() {
        let portals = channel(&[(1.0, 5.0, 0.5), (3.0, 5.0, 0.5)], &[(2.0, -5.0, 0.5), (4.0, -5.0, 0.5)]);
        let path = taut([0.0, 0.0], [5.0, 0.0], &portals).unwrap();
        assert_eq!(path.discs.len(), 2);
        assert!((path_length(&path) - 5.0).abs() < 1.0e-9);
    }

    #[test]
    fn a_vertex_in_the_way_is_wrapped_at_its_radius() {
        // A left vertex below the straight line: the path passes under it.
        let portals = channel(&[(2.0, -1.0, 0.5), (4.0, 5.0, 0.5)], &[(1.0, -5.0, 0.5), (3.0, -5.0, 0.5)]);
        let path = taut([0.0, 0.0], [5.0, 0.0], &portals).unwrap();
        assert_eq!(path.discs.len(), 3);
        assert_eq!(path.discs[1].vertex, 0);
        let (d, _) = closest(&path, [2.0, -1.0]);
        assert!((d - 0.5).abs() < 1.0e-9, "{d}");
        assert!(polyline(&path, 0.001).iter().all(|p| distance(*p, [2.0, -1.0]) >= 0.5 - 1.0e-9));
    }

    #[test]
    fn an_oblique_crossing_keeps_the_perpendicular_distance() {
        // The shrunk portal allows a point 0.5 along the portal from the
        // left vertex, but the path crosses at a shallow angle: measured
        // perpendicularly it would come closer; the refinement wraps it.
        let portals = vec![
            Portal { left: [2.0, 0.4], right: [2.0, -5.0], left_vertex: 0, right_vertex: 1, left_radius: 0.5, right_radius: 0.1 },
            Portal { left: [2.0, 0.4], right: [6.0, -5.0], left_vertex: 0, right_vertex: 2, left_radius: 0.5, right_radius: 0.1 },
        ];
        let path = taut([0.0, -0.5], [10.0, -0.3], &portals).unwrap();
        let (d, _) = closest(&path, [2.0, 0.4]);
        assert!(d >= 0.5 - 1.0e-7, "{d}");
    }

    #[test]
    fn a_zigzag_wraps_alternate_sides() {
        let portals = channel(
            &[(2.0, -0.5, 0.3), (6.0, 5.0, 0.3)],
            &[(1.0, -5.0, 0.3), (4.0, 0.5, 0.3), (7.0, -5.0, 0.3)],
        );
        let path = taut([0.0, 0.0], [8.0, 0.0], &portals).unwrap();
        let wrapped: Vec<usize> = path.discs[1..path.discs.len() - 1].iter().map(|d| d.vertex).collect();
        assert_eq!(wrapped, vec![0, 101]);
        for (center, radius) in [([2.0, -0.5], 0.3), ([4.0, 0.5], 0.3)] {
            let (d, _) = closest(&path, center);
            assert!((d - radius).abs() < 1.0e-7, "{d}");
        }
        let points = polyline(&path, 0.0005);
        assert!(near(points[0], [0.0, 0.0]) && near(*points.last().unwrap(), [8.0, 0.0]));
    }

    #[test]
    fn a_channel_too_narrow_is_squeezed() {
        let portals = vec![Portal { left: [2.0, 0.5], right: [2.0, -0.5], left_vertex: 0, right_vertex: 1, left_radius: 0.6, right_radius: 0.6 }];
        assert!(matches!(taut([0.0, 0.0], [4.0, 0.0], &portals), Err(Failure::Squeezed { .. })));
    }

    #[test]
    fn the_polyline_of_an_arc_stays_outside_by_at_most_the_bulge() {
        let portals = channel(&[(5.0, -3.0, 2.0), (10.0, 5.0, 0.1)], &[(1.0, -9.0, 0.1), (9.0, -9.0, 0.1)]);
        let path = taut([0.0, 0.0], [10.0, 0.0], &portals).unwrap();
        let points = polyline(&path, 0.0005);
        for pair in points.windows(2) {
            let d = point_segment_distance([5.0, -3.0], pair[0], pair[1]);
            assert!(d >= 2.0 - 1.0e-9, "{d}");
        }
        for point in &points {
            let d = distance(*point, [5.0, -3.0]);
            assert!(d >= 2.0 - 1.0e-9 && (d > 2.1 || d <= 2.0005 + 1.0e-9), "{d}");
        }
    }
}
