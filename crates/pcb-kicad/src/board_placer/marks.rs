// Copyright (C) 2026 Toit contributors.

//! The board's own texts and silkscreen graphics, which the input placed:
//! header pin names, test pad names, a board's title, a logo. They stay
//! where they are (the polish no longer moves them, `labels`) and are
//! placement obstacles on their side, with a keepout's semantics: a part's
//! body (courtyard and pads) stays off them, and a part on the other side
//! keeps its holes and pads there off them (the breadboard fence: the
//! placer put SW1, R22 and R4 under the header and test pad labels, and
//! the polish moved the labels away, one 7.5 mm, the test pads' names
//! shuffled).

use super::*;
use pcb_placer as core;

/// A box: [min x, min y, max x, max y] in board coordinates.
pub(crate) type MarkBox = [f64; 4];

/// One mark of the input board: a text, a text box, or a cluster of
/// silkscreen graphics (shapes within 0.5 mm of each other, a logo drawn
/// as polygons).
#[derive(Clone, Debug)]
pub(crate) struct BoardMark {
    /// What it is, for reports: `text "GND"`, `silkscreen graphics (12
    /// shapes)`.
    pub what: String,
    /// The layer it is on (the first member's, for a cluster).
    pub layer: String,
    /// On the back (B.SilkS, B.Cu).
    pub back: bool,
    /// On a copper layer (a copper text): copper of no net.
    pub copper: bool,
    /// The board position it is reported at: a text's anchor, a cluster's
    /// centre.
    pub at: [f64; 2],
    pub bounds: MarkBox,
    /// What it covers: one box for a text, the members' boxes (merged to
    /// at most `MAXIMUM_BOXES`) for graphics, so that a rectangle drawn
    /// around a group of parts blocks with its sides, not its inside.
    pub boxes: Vec<MarkBox>,
}

impl BoardMark {
    /// How reports name it: what it is, its layer and anchor.
    pub(crate) fn key(&self) -> String {
        format!(
            "{} ({} at {:.2}, {:.2})",
            self.what, self.layer, self.at[0], self.at[1]
        )
    }
}

/// The marks that belong to a movable part rather than to the board: a
/// mark whose boxes overlap the part's body (courtyard and pads) on the
/// mark's side, or its holes and pads there from the other side, at the
/// part's input position (a connector's pin names inside its courtyard,
/// a chip's name on its outline). Such a mark labels the part, which the
/// placement moves: it is no obstacle (`labels.attached` reports it). A
/// part stacked with others at one origin (a task's unplaced parts, the
/// fence's movable parts) says nothing about where it belongs and claims
/// no mark; of several parts, the one overlapping most. Returns (the
/// mark's place in `marks`, the part).
pub(super) fn attach_marks(
    problem: &core::Problem,
    footprints: usize,
    marks: &[(usize, BoardMark)],
) -> Vec<(usize, usize)> {
    let movable: Vec<usize> = (0..footprints)
        .filter(|part| !problem.components[*part].fixed)
        .collect();
    let origin = |part: usize| problem.poses[part].position;
    let placed: Vec<usize> = movable
        .iter()
        .copied()
        .filter(|part| {
            !movable.iter().any(|other| {
                other != part
                    && (origin(*other)[0] - origin(*part)[0]).abs() < 0.01
                    && (origin(*other)[1] - origin(*part)[1]).abs() < 0.01
            })
        })
        .collect();
    let overlap = |a: (core::problem::Point, core::problem::Point), b: MarkBox| {
        let width = (a.0[0] + a.1[0]).min(b[2]) - (a.0[0] - a.1[0]).max(b[0]);
        let height = (a.0[1] + a.1[1]).min(b[3]) - (a.0[1] - a.1[1]).max(b[1]);
        if width > 0.0 && height > 0.0 {
            width * height
        } else {
            0.0
        }
    };
    let mut attached = Vec::new();
    for (place, (_, mark)) in marks.iter().enumerate() {
        let side = if mark.back {
            core::Side::Back
        } else {
            core::Side::Front
        };
        let mut best: Option<(f64, usize)> = None;
        for &part in &placed {
            let component = &problem.components[part];
            let pose = problem.poses[part];
            let boxes = if component.side.collides(side) {
                vec![(component.center(pose), component.half_extent(pose.angle))]
            } else {
                let mut boxes = component.far_boxes(pose);
                boxes.extend(component.boxes_of(&component.far_copper, pose));
                boxes
            };
            let area: f64 = boxes
                .iter()
                .flat_map(|b| mark.boxes.iter().map(move |m| (*b, *m)))
                .map(|(b, m)| overlap(b, m))
                .sum();
            if area > 0.0 && best.is_none_or(|(most, _)| area > most) {
                best = Some((area, part));
            }
        }
        if let Some((_, part)) = best {
            attached.push((place, part));
        }
    }
    attached
}

/// Most boxes one cluster of graphics blocks with (a dense logo is
/// coarsened to a grid of runs).
const MAXIMUM_BOXES: usize = 48;

/// Graphics within this distance of each other form one cluster.
const CLUSTER_GAP: f64 = 0.5;

fn overlaps(a: MarkBox, b: MarkBox, gap: f64) -> bool {
    a[0] < b[2] + gap && b[0] < a[2] + gap && a[1] < b[3] + gap && b[1] < a[3] + gap
}

fn union(a: MarkBox, b: MarkBox) -> MarkBox {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

fn shapes_box(shapes: &[pcb_router::Shape]) -> Option<MarkBox> {
    shapes
        .iter()
        .map(pcb_router::Shape::aabb)
        .reduce(pcb_router::geometry::Aabb::union)
        .map(|bounds| {
            [
                bounds.minimum[0],
                bounds.minimum[1],
                bounds.maximum[0],
                bounds.maximum[1],
            ]
        })
}

/// Whether a board text is hidden (KiCad 8's `(hide yes)`, older `hide`
/// atoms in the item or its effects).
fn hidden(item: &Expr) -> bool {
    let hides = |node: &Expr| {
        node.children().iter().any(|child| {
            child.atom() == Some("hide")
                || (child.head() == Some("hide")
                    && !matches!(
                        child.children().get(1).and_then(Expr::atom),
                        Some("no" | "false")
                    ))
        })
    };
    hides(item) || item.child("effects").is_some_and(hides)
}

/// A KiCad 6 or 7 text written `(gr_text locked "R2" ...)` without the
/// `locked` atom, so that its second child is its text.
fn without_locked(item: &Expr) -> Expr {
    match item {
        Expr::List(children)
            if children.get(1).and_then(Expr::atom) == Some("locked")
                && children.get(2).and_then(Expr::atom).is_some() =>
        {
            let mut children = children.clone();
            children.remove(1);
            Expr::List(children)
        }
        _ => item.clone(),
    }
}

/// The text of a board text (`None` for another item).
pub(crate) fn mark_text(item: &Expr) -> Option<String> {
    matches!(item.head(), Some("gr_text"))
        .then(|| {
            without_locked(item)
                .children()
                .get(1)
                .and_then(Expr::atom)
                .map(str::to_string)
        })
        .flatten()
}

/// The layer names an item is drawn on (`layer`, or KiCad 9's `layers`).
fn item_layers(item: &Expr) -> Vec<String> {
    match form_atom(item, "layer", 1) {
        Some(name) => vec![name.to_string()],
        None => item
            .child("layers")
            .map(Expr::children)
            .unwrap_or_default()
            .iter()
            .skip(1)
            .filter_map(Expr::atom)
            .map(str::to_string)
            .collect(),
    }
}

/// The side of a silkscreen or outer copper layer: false for the front,
/// true for the back; whether it is copper.
fn mark_side(layer: &str) -> Option<(bool, bool)> {
    match layer {
        "F.SilkS" => Some((false, false)),
        "B.SilkS" => Some((true, false)),
        "F.Cu" => Some((false, true)),
        "B.Cu" => Some((true, true)),
        _ => None,
    }
}

/// The board's texts and text boxes on the silkscreen and outer copper
/// layers, and its silkscreen graphics in clusters, that lie on the board
/// (`outline`: the outline's box; `None`: all). A cluster that spans most
/// of the board (a frame around it, an image behind every part: Blue_Line's
/// 8793 polygons) is decoration of the whole board, not a mark: left out,
/// as the polish has always left it.
pub(crate) fn board_marks(pcb: &Expr, outline: Option<MarkBox>) -> Result<Vec<BoardMark>, String> {
    let on_board = |bounds: MarkBox| outline.is_none_or(|board| overlaps(bounds, board, 0.0));
    let mut marks = Vec::new();
    // Graphics to cluster: (layer, back, box, its shapes' boxes).
    let mut graphics: Vec<(String, bool, MarkBox, Vec<MarkBox>)> = Vec::new();
    for item in pcb.children() {
        let head = item.head();
        if !matches!(
            head,
            Some(
                "gr_text"
                    | "gr_text_box"
                    | "gr_line"
                    | "gr_rect"
                    | "gr_arc"
                    | "gr_circle"
                    | "gr_poly"
                    | "gr_curve"
            )
        ) {
            continue;
        }
        for layer in item_layers(item) {
            let Some((back, copper)) = mark_side(&layer) else {
                continue;
            };
            match head {
                Some("gr_text" | "gr_text_box") => {
                    if hidden(item) {
                        continue;
                    }
                    let item = without_locked(item);
                    let shapes = if head == Some("gr_text") {
                        if mark_text(&item).is_none_or(|text| text.is_empty()) {
                            continue;
                        }
                        board_router::copper_graphic_shapes(&item)?
                    } else {
                        board_router::text_box_shapes(&item)?
                    };
                    let Some(bounds) = shapes_box(&shapes) else {
                        continue;
                    };
                    if !on_board(bounds) {
                        continue;
                    }
                    let at = if head == Some("gr_text") {
                        let at = form_at(&item)?;
                        [at[0], at[1]]
                    } else {
                        [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0]
                    };
                    marks.push(BoardMark {
                        what: match mark_text(&item) {
                            Some(text) => format!("text {text:?}"),
                            None => "text box".into(),
                        },
                        layer,
                        back,
                        copper,
                        at,
                        bounds,
                        boxes: vec![bounds],
                    });
                }
                // Copper graphics are the lowering's already (copper
                // obstacles); here the silkscreen's.
                _ if !copper => {
                    // A Bezier curve: the box of its control points holds it.
                    let shapes = if head == Some("gr_curve") {
                        let points = outline::pts_points(item)?;
                        let Some(bounds) = points
                            .iter()
                            .map(|point| [point[0], point[1], point[0], point[1]])
                            .reduce(union)
                        else {
                            continue;
                        };
                        vec![bounds]
                    } else {
                        board_router::copper_graphic_shapes(item)?
                            .iter()
                            .map(|shape| {
                                let bounds = shape.aabb();
                                [
                                    bounds.minimum[0],
                                    bounds.minimum[1],
                                    bounds.maximum[0],
                                    bounds.maximum[1],
                                ]
                            })
                            .collect()
                    };
                    let Some(bounds) = shapes.iter().copied().reduce(union) else {
                        continue;
                    };
                    if !on_board(bounds) {
                        continue;
                    }
                    graphics.push((layer, back, bounds, shapes));
                }
                _ => {}
            }
        }
    }
    marks.extend(cluster_graphics(graphics, outline));
    Ok(marks)
}

/// Silkscreen graphics in clusters of shapes within `CLUSTER_GAP` of each
/// other (union-find over a grid of cells), one mark each.
fn cluster_graphics(
    graphics: Vec<(String, bool, MarkBox, Vec<MarkBox>)>,
    outline: Option<MarkBox>,
) -> Vec<BoardMark> {
    let count = graphics.len();
    let mut parent: Vec<usize> = (0..count).collect();
    fn root(parent: &mut [usize], mut index: usize) -> usize {
        while parent[index] != index {
            parent[index] = parent[parent[index]];
            index = parent[index];
        }
        index
    }
    // Every shape (a stroke, a filled polygon) in the cells its grown box
    // touches; shapes sharing a cell are tested. By the shapes, not the
    // graphics' boxes: a rectangle drawn around a group is near what lies
    // by its sides, not what lies inside it.
    let cell = 2.0;
    let mut cells: BTreeMap<(bool, i64, i64), Vec<(usize, MarkBox)>> = BTreeMap::new();
    for (index, (_, back, _, shapes)) in graphics.iter().enumerate() {
        for shape in shapes {
            let grown = [
                shape[0] - CLUSTER_GAP / 2.0,
                shape[1] - CLUSTER_GAP / 2.0,
                shape[2] + CLUSTER_GAP / 2.0,
                shape[3] + CLUSTER_GAP / 2.0,
            ];
            let range =
                |low: f64, high: f64| (low / cell).floor() as i64..=(high / cell).floor() as i64;
            for x in range(grown[0], grown[2]) {
                for y in range(grown[1], grown[3]) {
                    let members = cells.entry((*back, x, y)).or_default();
                    for &(other, other_shape) in members.iter() {
                        if other != index && overlaps(other_shape, *shape, CLUSTER_GAP) {
                            let (a, b) = (root(&mut parent, index), root(&mut parent, other));
                            if a != b {
                                parent[a.max(b)] = a.min(b);
                            }
                        }
                    }
                    members.push((index, *shape));
                }
            }
        }
    }
    let mut clusters: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..count {
        let root = root(&mut parent, index);
        clusters.entry(root).or_default().push(index);
    }
    let mut marks = Vec::new();
    for members in clusters.into_values() {
        let bounds = members
            .iter()
            .map(|member| graphics[*member].2)
            .reduce(union)
            .expect("a cluster has members");
        if outline.is_some_and(|board| {
            bounds[2] - bounds[0] > 0.8 * (board[2] - board[0])
                || bounds[3] - bounds[1] > 0.8 * (board[3] - board[1])
        }) {
            continue;
        }
        let boxes: Vec<MarkBox> = members
            .iter()
            .flat_map(|member| graphics[*member].3.iter().copied())
            .collect();
        let first = &graphics[members[0]];
        marks.push(BoardMark {
            what: if members.len() == 1 {
                "silkscreen graphic".into()
            } else {
                format!("silkscreen graphics ({} shapes)", members.len())
            },
            layer: first.0.clone(),
            back: first.1,
            copper: false,
            at: [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0],
            bounds,
            boxes: coarsened(boxes, bounds),
        });
    }
    marks
}

/// At most `MAXIMUM_BOXES` boxes covering `boxes`: as they are when they
/// are few, else the runs of grid cells they touch, row by row, on a grid
/// that doubles until the runs are few enough.
fn coarsened(boxes: Vec<MarkBox>, bounds: MarkBox) -> Vec<MarkBox> {
    if boxes.len() <= MAXIMUM_BOXES {
        return boxes;
    }
    let mut size = 0.25;
    loop {
        let columns = (((bounds[2] - bounds[0]) / size).ceil() as usize).max(1);
        let rows = (((bounds[3] - bounds[1]) / size).ceil() as usize).max(1);
        let mut taken = vec![false; columns * rows];
        for b in &boxes {
            let column =
                |x: f64| (((x - bounds[0]) / size).floor().max(0.0) as usize).min(columns - 1);
            let row = |y: f64| (((y - bounds[1]) / size).floor().max(0.0) as usize).min(rows - 1);
            for r in row(b[1])..=row(b[3]) {
                for c in column(b[0])..=column(b[2]) {
                    taken[r * columns + c] = true;
                }
            }
        }
        let mut runs = Vec::new();
        for r in 0..rows {
            let mut c = 0;
            while c < columns {
                if !taken[r * columns + c] {
                    c += 1;
                    continue;
                }
                let start = c;
                while c < columns && taken[r * columns + c] {
                    c += 1;
                }
                runs.push([
                    bounds[0] + start as f64 * size,
                    bounds[1] + r as f64 * size,
                    (bounds[0] + c as f64 * size).min(bounds[2]),
                    (bounds[1] + (r + 1) as f64 * size).min(bounds[3]),
                ]);
            }
        }
        // Runs of one row continue those of the row above with the same
        // columns: one box.
        let mut merged: Vec<MarkBox> = Vec::new();
        for run in runs {
            match merged.iter_mut().rev().find(|previous| {
                (previous[0] - run[0]).abs() < 1e-9
                    && (previous[2] - run[2]).abs() < 1e-9
                    && (previous[3] - run[1]).abs() < 1e-9
            }) {
                Some(previous) => previous[3] = run[3],
                None => merged.push(run),
            }
        }
        if merged.len() <= MAXIMUM_BOXES {
            return merged;
        }
        size *= 2.0;
    }
}

/// The outline's box, if the board has one.
pub(crate) fn outline_box(pcb: &Expr) -> Option<MarkBox> {
    outline::board_loops(pcb).ok().and_then(|loops| {
        loops
            .outline
            .iter()
            .map(|point| [point[0], point[1], point[0], point[1]])
            .reduce(union)
    })
}

/// A mark as a fixed placement obstacle on its side: its box, and where it
/// is graphics, the boxes it blocks with (`hollow`, also as its copper
/// against far-side holes and pads).
pub(super) fn mark_obstacle(mark: &BoardMark) -> (core::Component, core::Pose) {
    let bounds = mark.bounds;
    let center = [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0];
    let local: Vec<[f64; 4]> = if mark.boxes.len() > 1 {
        mark.boxes
            .iter()
            .map(|b| {
                [
                    b[0] - center[0],
                    b[1] - center[1],
                    b[2] - center[0],
                    b[3] - center[1],
                ]
            })
            .collect()
    } else {
        Vec::new()
    };
    let component = core::Component {
        name: if mark.copper {
            "copper text".into()
        } else {
            "silkscreen mark".into()
        },
        body_center: [0.0, 0.0],
        body_size: [bounds[2] - bounds[0], bounds[3] - bounds[1]],
        round: false,
        halo: 0.0,
        pins: Vec::new(),
        side: if mark.back {
            core::Side::Back
        } else {
            core::Side::Front
        },
        fixed: true,
        angle_options: vec![0.0],
        far_side: Vec::new(),
        far_copper: Vec::new(),
        far_courtyard: Vec::new(),
        hollow: local.clone(),
        tight: None,
        edge_inset: 0.0,
        courtyards: Vec::new(),
        // A hollow mark blocks with its boxes against every part, those
        // with holes too.
        holes_inside: true,
        pads: local,
        copper_only: false,
        cutout_outline: Vec::new(),
        tight_hollow: Vec::new(),
        hole_rings: Vec::new(),
        copper_graphics: Vec::new(),
    };
    let pose = core::Pose {
        position: center,
        angle: 0.0,
    };
    (component, pose)
}
