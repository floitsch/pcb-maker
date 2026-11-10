// Copyright (C) 2026 Toit contributors.

//! `row`: parts in a line or grid at a fixed pitch, placed as one part.

use super::*;

/// Parts in a line at a fixed pitch, turned alike (LED bars, key rows, test
/// points). The row is placed as one part, led by its first part: other
/// constraints name the first part only.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadRowConstraint {
    /// References or glob patterns, in row order (a glob's parts in natural
    /// order: D2 before D10).
    pub parts: Vec<String>,
    /// Distance between neighbouring parts' origins.
    pub pitch_mm: f64,
    /// `x` (default): along the board's x axis; `y`: along its y axis. The
    /// row may run either way; a rotation constraint on its first part
    /// fixes that.
    #[serde(default)]
    pub axis: Option<String>,
    /// Parts per line: with it, the parts fill a grid line by line (a
    /// keyboard's switches), the lines `row_pitch_mm` apart (default
    /// `pitch_mm`) across the axis.
    #[serde(default)]
    pub columns: Option<usize>,
    #[serde(default)]
    pub row_pitch_mm: Option<f64>,
    /// A differential pair's two series parts (`pair_rows`, not the
    /// user's): the row may turn by any quarter turn, not only a half.
    #[serde(skip)]
    pub pair: bool,
}

/// Natural order of references: D2 before D10.
pub(crate) fn natural(reference: &str) -> (String, u64, String) {
    let digits = reference
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(reference.len());
    let end = reference[digits..]
        .find(|c: char| !c.is_ascii_digit())
        .map_or(reference.len(), |end| digits + end);
    (
        reference[..digits].to_string(),
        reference[digits..end].parse().unwrap_or(0),
        reference[end..].to_string(),
    )
}

/// What `apply_rows` returns: the followers' leaders, their places, and the user's rows: each
/// leader with a member's own body size (before the members' bodies go to
/// the leader), for the outline's test.
type AppliedRows = (
    BTreeMap<usize, usize>,
    BTreeMap<usize, RowPlace>,
    Vec<(usize, [f64; 2])>,
);

/// Turns every row into one part: the first part carries the others' bodies,
/// pins and pads at their places in the row; the others become followers
/// that occupy nothing themselves. Returns each follower's leader.
pub(super) fn apply_rows(
    problem: &mut core::Problem,
    references: &[String],
    footprints: usize,
    pad_boxes: &[Vec<[f64; 4]>],
    rows: &[KiCadRowConstraint],
) -> Result<AppliedRows, String> {
    let mut followers = Followers::default();
    // The first parts of the rows.
    let mut user_rows = Vec::new();
    for entry in rows {
        let members = row_members(references, footprints, entry)?;
        let grid = RowGrid::of(entry, members.len())?;
        let leader = members[0];
        user_rows.push((leader, problem.components[members[1]].body_size));
        let angle = problem.poses[leader].angle + row_turn(problem, &members, &grid);
        let row = Row {
            entry,
            members,
            grid,
            angle,
        };
        let mut carried = row_carrier(&problem.components[leader]);
        let (member_pads, body) = carry_members(
            problem,
            references,
            pad_boxes,
            &row,
            &mut carried,
            &mut followers,
        )?;
        check_row_pads(problem, references, pad_boxes, &row, &member_pads)?;
        finish_row(problem, &row, carried, body);
    }
    let mut poses = problem.poses.clone();
    problem.sync_followers(&mut poses);
    problem.poses = poses;
    Ok((followers.leaders, followers.places, user_rows))
}

/// The rows applied so far: each follower's leader and its place.
#[derive(Default)]
struct Followers {
    leaders: BTreeMap<usize, usize>,
    places: BTreeMap<usize, RowPlace>,
}

/// A row's grid: the axis its lines run along, the axis across it, the
/// parts per line and the pitches along and across.
struct RowGrid {
    axis: [f64; 2],
    across: [f64; 2],
    columns: usize,
    pitch: f64,
    line_pitch: f64,
}

impl RowGrid {
    /// The grid of `row`, of `members` parts; refuses a pitch, an axis or
    /// a number of columns that makes none.
    #[allow(
        clippy::neg_cmp_op_on_partial_ord,
        reason = "!(x > 0.0) rejects NaN as well as zero and negative values"
    )]
    fn of(row: &KiCadRowConstraint, members: usize) -> Result<RowGrid, String> {
        if !(row.pitch_mm > 0.0) {
            return Err(format!("row {:?}: pitch_mm must be positive", row.parts));
        }
        let axis = match row.axis.as_deref() {
            None | Some("x") => [1.0, 0.0],
            Some("y") => [0.0, 1.0],
            Some(other) => return Err(format!("unknown row axis {other:?} (x or y)")),
        };
        // Lines of a grid follow each other across the axis.
        let across = [axis[1], axis[0]];
        let columns = row.columns.unwrap_or(members);
        let line_pitch = row.row_pitch_mm.unwrap_or(row.pitch_mm);
        if columns == 0 || !(line_pitch > 0.0) {
            return Err(format!(
                "row {:?}: columns and row_pitch_mm must be positive",
                row.parts
            ));
        }
        Ok(RowGrid {
            axis,
            across,
            columns,
            pitch: row.pitch_mm,
            line_pitch,
        })
    }

    /// Where the `place`-th member's origin lies from the first's, in the
    /// row's own frame.
    fn place_of(&self, place: usize) -> [f64; 2] {
        let along = (place % self.columns) as f64 * self.pitch;
        let line = (place / self.columns) as f64 * self.line_pitch;
        [
            self.axis[0] * along + self.across[0] * line,
            self.axis[1] * along + self.across[1] * line,
        ]
    }
}

/// One row as `apply_rows` builds it: its entry, its members in row order
/// (the first leads), its grid and the angle of its frame.
struct Row<'a> {
    entry: &'a KiCadRowConstraint,
    members: Vec<usize>,
    grid: RowGrid,
    angle: f64,
}

impl Row<'_> {
    fn leader(&self) -> usize {
        self.members[0]
    }

    /// The `place`-th member's place in the leader's own frame.
    fn offset(&self, place: usize) -> [f64; 2] {
        core::problem::rotate(self.grid.place_of(place), self.angle)
    }
}

/// A row's members in row order: each pattern's parts in natural order,
/// each part once. Refuses a pattern that matches nothing and a row of
/// fewer than two parts.
fn row_members(
    references: &[String],
    footprints: usize,
    row: &KiCadRowConstraint,
) -> Result<Vec<usize>, String> {
    let mut members: Vec<usize> = Vec::new();
    for pattern in &row.parts {
        let mut parts: Vec<usize> = (0..footprints)
            .filter(|index| glob_matches(pattern, &references[*index]))
            .collect();
        if parts.is_empty() {
            return Err(format!("row part {pattern:?} matches no part"));
        }
        parts.sort_by_key(|index| natural(&references[*index]));
        for part in parts {
            if !members.contains(&part) {
                members.push(part);
            }
        }
    }
    if members.len() < 2 {
        return Err(format!("row {:?} has fewer than two parts", row.parts));
    }
    Ok(members)
}

/// The turn of the row's frame from its leader's angle: none, or the
/// quarter or half turn the members lie at exactly.
fn row_turn(problem: &core::Problem, members: &[usize], grid: &RowGrid) -> f64 {
    // The row's frame: the leader's angle, or that angle turned by a
    // half turn when the members lie exactly
    // that way. A board placed before (layout mode lowers its own
    // placement again) holds a row the placer turned: built from the
    // leader's angle alone, it ran the other way, reported a region
    // the row kept as missed by the row's length and moved the members.
    // A stack fresh from a netlist matches every frame alike and keeps
    // the leader's.
    let leader_pose = problem.poses[members[0]];
    let mismatch = |turn: f64| -> f64 {
        members
            .iter()
            .enumerate()
            .map(|(place, member)| {
                let offset = core::problem::rotate(grid.place_of(place), turn);
                let at = problem.poses[*member].position;
                (leader_pose.position[0] + offset[0] - at[0]).powi(2)
                    + (leader_pose.position[1] + offset[1] - at[1]).powi(2)
            })
            .sum()
    };
    // Only a row that lies exactly so (members within 0.01 mm of their
    // places) turns the frame: parts anywhere else keep the leader's.
    // A row turned a quarter (its first part's rotation: trial 6's
    // jacks down a eurorack board) lies so after the placement too: its
    // layout lowered the placed board again with the row along the
    // board's x axis, 114.7 mm on a 110 mm board, and failed (exit 4).
    let exact = 1.0e-4 * members.len() as f64;
    if mismatch(0.0) > exact {
        [180.0, 90.0, 270.0]
            .into_iter()
            .find(|turn| mismatch(*turn) <= exact)
            .unwrap_or(0.0)
    } else {
        0.0
    }
}

/// The leader's component as the row's part before its members join it:
/// the boxes, pins, pads and slots each member (the leader too) adds at
/// its place cleared.
fn row_carrier(leader: &core::Component) -> core::Component {
    let mut carried = leader.clone();
    // The members' far-side boxes join the row's: none lets a part in.
    carried.far_open = Default::default();
    carried.hollow.clear();
    carried.tight_hollow.clear();
    carried.pins.clear();
    carried.far_side.clear();
    carried.far_copper.clear();
    carried.far_courtyard.clear();
    carried.pads.clear();
    // Members without courtyards keep the bodies a courtyard would
    // give them, and every member's own slots go with the row.
    carried.courtless.clear();
    carried.slots.clear();
    carried
}

/// A member's pads at its place in the row: (member, place, boxes).
type MemberPads = Vec<(usize, usize, Vec<[f64; 4]>)>;

/// Gives `carried` every member's pins, pads, boxes and slots at its place
/// in the row, and makes the members after the first followers of the
/// leader. Returns each member's pads at its place and the box of the
/// members' bodies. Refuses a part in two rows and a row across sides.
fn carry_members(
    problem: &mut core::Problem,
    references: &[String],
    pad_boxes: &[Vec<[f64; 4]>],
    row: &Row,
    carried: &mut core::Component,
    followers: &mut Followers,
) -> Result<(MemberPads, [f64; 4]), String> {
    let leader = row.leader();
    let side = problem.components[leader].side;
    let mut body = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    // Where a member blocks with shaped boxes (`hollow`), the row
    // blocks with every member's: its own boxes, or its whole body.
    let shaped = row
        .members
        .iter()
        .any(|member| !problem.components[*member].hollow.is_empty());
    // Each member's pads at its place, to refuse a row whose pitch
    // puts two members' pads on each other (trial 5: a row of two 0402
    // capacitors turned along the axis at 1 mm, 1.82 mm long, came back
    // shorted, with no word before the run).
    let mut member_pads: MemberPads = Vec::new();
    for (place, member) in row.members.iter().enumerate() {
        if followers.leaders.contains_key(member)
            || followers.leaders.values().any(|leader| leader == member)
        {
            return Err(format!("{} is in two rows", references[*member]));
        }
        let component = &problem.components[*member];
        if component.side != side {
            return Err(format!(
                "row {:?}: all parts must be on one side",
                row.entry.parts
            ));
        }
        // The member's place in the leader's own frame.
        let offset = row.offset(place);
        let shift = |boxes: &[[f64; 4]]| -> Vec<[f64; 4]> {
            boxes
                .iter()
                .map(|b| {
                    [
                        b[0] + offset[0],
                        b[1] + offset[1],
                        b[2] + offset[0],
                        b[3] + offset[1],
                    ]
                })
                .collect()
        };
        let center = [
            component.body_center[0] + offset[0],
            component.body_center[1] + offset[1],
        ];
        body = [
            body[0].min(center[0] - component.body_size[0] / 2.0),
            body[1].min(center[1] - component.body_size[1] / 2.0),
            body[2].max(center[0] + component.body_size[0] / 2.0),
            body[3].max(center[1] + component.body_size[1] / 2.0),
        ];
        carried
            .pins
            .extend(component.pins.iter().map(|pin| core::Pin {
                offset: [pin.offset[0] + offset[0], pin.offset[1] + offset[1]],
                net: pin.net,
            }));
        carried.far_side.extend(shift(&component.far_side));
        carried.far_copper.extend(shift(&component.far_copper));
        carried
            .far_courtyard
            .extend(shift(&component.far_courtyard));
        carried.pads.extend(shift(&pad_boxes[*member]));
        member_pads.push((*member, place, shift(&pad_boxes[*member])));
        carried.courtless.extend(shift(&component.courtless));
        carried.slots.extend(shift(&component.slots));
        if shaped {
            if component.hollow.is_empty() {
                let half = [component.body_size[0] / 2.0, component.body_size[1] / 2.0];
                carried.hollow.push([
                    center[0] - half[0],
                    center[1] - half[1],
                    center[0] + half[0],
                    center[1] + half[1],
                ]);
            } else {
                carried.hollow.extend(shift(&component.hollow));
            }
        }
        carried.halo = carried.halo.max(component.halo);
        carried.edge_inset = carried.edge_inset.min(component.edge_inset);
        if *member != leader {
            followers.leaders.insert(*member, leader);
            followers.places.insert(
                *member,
                RowPlace {
                    offset,
                    center,
                    half: [component.body_size[0] / 2.0, component.body_size[1] / 2.0],
                },
            );
            problem
                .constraints
                .followers
                .push(core::constraints::Follower {
                    part: *member,
                    leader,
                    offset,
                    angle: 0.0,
                });
        }
    }
    Ok((member_pads, body))
}

/// Whether two boxes overlap by more than a hundredth of a millimetre,
/// the first grown by `grow`.
fn boxes_meet(a: &[f64; 4], b: &[f64; 4], grow: f64) -> bool {
    a[0] - grow < b[2] - 0.01
        && b[0] < a[2] + grow - 0.01
        && a[1] - grow < b[3] - 0.01
        && b[1] < a[3] + grow - 0.01
}

/// A pad's net: the pin of `member` whose centre lies in its box.
fn pad_net(problem: &core::Problem, member: usize, pad: &[f64; 4]) -> Option<usize> {
    problem.components[member]
        .pins
        .iter()
        .find(|pin| {
            pin.offset[0] >= pad[0] - 1.0e-6
                && pin.offset[0] <= pad[2] + 1.0e-6
                && pin.offset[1] >= pad[1] - 1.0e-6
                && pin.offset[1] <= pad[3] + 1.0e-6
        })
        .map(|pin| pin.net)
}

/// The gap two members' pads keep (`first`'s pad `a`, `second`'s pad
/// `b`): none between pads of one net, else `clearance`.
fn pad_gap(
    problem: &core::Problem,
    pad_boxes: &[Vec<[f64; 4]>],
    clearance: f64,
    (first, a): (usize, usize),
    (second, b): (usize, usize),
) -> f64 {
    match (
        pad_net(problem, first, &pad_boxes[first][a]),
        pad_net(problem, second, &pad_boxes[second][b]),
    ) {
        (Some(x), Some(y)) if x == y => 0.0,
        _ => clearance,
    }
}

/// Refuses a row whose pitch puts two members' pads on each other, or
/// pads of two nets closer than the copper clearance, saying the pitch
/// that clears them.
fn check_row_pads(
    problem: &core::Problem,
    references: &[String],
    pad_boxes: &[Vec<[f64; 4]>],
    row: &Row,
    member_pads: &MemberPads,
) -> Result<(), String> {
    // Pads of two nets keep the copper clearance (`min_spacing`): a
    // row turns every part alike, so two lines of a bank face each
    // other with opposite nets (trial 8: the 4in1-mini's lines 4.2 mm
    // apart left 16 clearance findings, GND and +BATT 0.1 mm apart).
    let clearance = problem.min_spacing.max(0.0);
    let grid = &row.grid;
    for (index, (first, first_place, pads)) in member_pads.iter().enumerate() {
        for (second, second_place, others) in &member_pads[index + 1..] {
            let conflicts: Vec<(usize, usize, f64)> = pads
                .iter()
                .enumerate()
                .flat_map(|(i, a)| {
                    others.iter().enumerate().filter_map(move |(j, b)| {
                        let gap = pad_gap(problem, pad_boxes, clearance, (*first, i), (*second, j));
                        boxes_meet(a, b, gap).then_some((i, j, gap))
                    })
                })
                .collect();
            if conflicts.is_empty() {
                continue;
            }
            // Two parts of one line are `pitch_mm` apart along the
            // axis, of two lines `row_pitch_mm` across it (trial 8:
            // the 4in1-mini's bank of 1206 capacitors, its lines too
            // close, was told about pitch_mm and the parts' size in
            // their own frame, and every pitch_mm the agent tried was
            // refused).
            let one_line = first_place / grid.columns == second_place / grid.columns;
            let (setting, now, direction) = if one_line {
                ("pitch_mm", grid.pitch, grid.axis)
            } else {
                ("row_pitch_mm", grid.line_pitch, grid.across)
            };
            // The direction in the parts' frame.
            let local = core::problem::rotate(direction, row.angle);
            let (needed, kept_apart) =
                needed_pitch(problem, pad_boxes, clearance, *first, *second, local);
            let on_each_other = conflicts.iter().any(|(_, _, gap)| *gap == 0.0)
                || conflicts
                    .iter()
                    .any(|(i, j, _)| boxes_meet(&pads[*i], &others[*j], 0.0));
            let board_axis = if direction[0].abs() > 0.5 { "x" } else { "y" };
            let unset = if !one_line && row.entry.row_pitch_mm.is_none() {
                " (row_pitch_mm is not given: it takes pitch_mm)"
            } else {
                ""
            };
            return Err(format!(
                "row {:?}: the pads of {} and {} {} ({}): turned as the row's first part is ({} degrees), they need {:.2} mm along the board's {board_axis}{}, so {setting} must be at least that (now {}{unset}); or turn the parts (the first part's rotation turns the row)",
                row.entry.parts,
                references[*first],
                references[*second],
                if on_each_other {
                    "lie on each other".to_string()
                } else {
                    format!("of two nets come closer than the copper clearance ({clearance} mm)")
                },
                if one_line {
                    "neighbours in a line".to_string()
                } else {
                    format!(
                        "lines {} and {} of the grid",
                        first_place / grid.columns + 1,
                        second_place / grid.columns + 1
                    )
                },
                row.angle,
                needed,
                if kept_apart {
                    format!(" (the pads' span and {clearance} mm between pads of two nets)")
                } else {
                    " (the pads' span)".to_string()
                },
                now,
            ));
        }
    }
    Ok(())
}

/// The pitch along `local` (a direction in the parts' frame) that clears
/// every pad of `first` from every pad of `second`, with the clearance
/// between pads of two nets; and whether that clearance decides it.
fn needed_pitch(
    problem: &core::Problem,
    pad_boxes: &[Vec<[f64; 4]>],
    clearance: f64,
    first: usize,
    second: usize,
    local: [f64; 2],
) -> (f64, bool) {
    let along = |b: &[f64; 4], far: bool| -> f64 {
        let values = [[b[0], b[1]], [b[0], b[3]], [b[2], b[1]], [b[2], b[3]]]
            .map(|corner| corner[0] * local[0] + corner[1] * local[1]);
        if far {
            values.into_iter().fold(f64::NEG_INFINITY, f64::max)
        } else {
            values.into_iter().fold(f64::INFINITY, f64::min)
        }
    };
    let mut needed: f64 = 0.0;
    let mut kept_apart = false;
    for (i, a) in pad_boxes[first].iter().enumerate() {
        for (j, b) in pad_boxes[second].iter().enumerate() {
            let gap = pad_gap(problem, pad_boxes, clearance, (first, i), (second, j));
            let pitch = along(a, true) - along(b, false) + gap;
            if pitch > needed {
                needed = pitch;
                kept_apart = gap > 0.0;
            }
        }
    }
    (needed, kept_apart)
}

/// Gives the leader the row's body (`body`, the members' bodies' box) and
/// the angles that keep the row along its axis; the other members occupy
/// nothing and stay fixed, carried by the leader.
fn finish_row(
    problem: &mut core::Problem,
    row: &Row,
    mut carried: core::Component,
    body: [f64; 4],
) {
    carried.body_center = [(body[0] + body[2]) / 2.0, (body[1] + body[3]) / 2.0];
    carried.body_size = [body[2] - body[0], body[3] - body[1]];
    carried.round = false;
    carried.tight = None;
    carried.courtyards.clear();
    carried.holes_inside = false;
    // Turning the row by a half turn keeps it along its axis; a pair's
    // two parts stay side by side whichever way the row turns.
    let angle = row.angle;
    carried.angle_options.retain(|option| {
        if row.entry.pair {
            return true;
        }
        let turn = (option - angle).rem_euclid(360.0);
        turn.abs() < 1.0e-6 || (turn - 180.0).abs() < 1.0e-6
    });
    if carried.angle_options.is_empty() {
        carried.angle_options = vec![angle];
    }
    problem.components[row.leader()] = carried;
    for member in &row.members[1..] {
        let component = &mut problem.components[*member];
        component.side = core::Side::Neither;
        component.fixed = true;
        component.pins.clear();
        component.far_side.clear();
        component.far_copper.clear();
        component.far_courtyard.clear();
        component.pads.clear();
        component.courtless.clear();
        component.slots.clear();
        component.halo = 0.0;
        component.body_size = [0.01, 0.01];
    }
}

/// Where a row member sits in its leader's frame: its origin's offset, its
/// body's centre and half size.
pub(super) struct RowPlace {
    pub(super) offset: [f64; 2],
    pub(super) center: [f64; 2],
    pub(super) half: [f64; 2],
}

/// The refusals for rows whose first part a place entry puts at `x`, `y`
/// (`pinned`) and that then run off the board (`user_rows`: each row's
/// first part).
pub(super) fn rows_off_the_board(
    problem: &core::Problem,
    references: &[String],
    user_rows: &[(usize, [f64; 2])],
    pinned: &[usize],
) -> Vec<String> {
    let mut cannot = Vec::new();
    // A row whose first part a place entry puts at x, y runs from there
    // along its axis in that part's frame: where it then leaves the board
    // its other parts would lie off it, kept there (trial 6: J4 placed at
    // the top of a eurorack board, turned 90 degrees, its seven jacks ran
    // up from it, 96 mm beyond the outline, with no word from the place
    // check and 71 connections open in the layout).
    if problem.outline.len() >= 3 {
        let bounds = problem.bounds();
        for (leader, _) in user_rows {
            if !pinned.contains(leader) {
                continue;
            }
            let component = &problem.components[*leader];
            let pose = problem.poses[*leader];
            let center = component.center(pose);
            let half = component.half_extent(pose.angle);
            let beyond = [
                ("left", bounds[0] - (center[0] - half[0])),
                ("top", bounds[1] - (center[1] - half[1])),
                ("right", center[0] + half[0] - bounds[2]),
                ("bottom", center[1] + half[1] - bounds[3]),
            ];
            let out: Vec<String> = beyond
                .iter()
                .filter(|(_, by)| *by > 0.5)
                .map(|(side, by)| format!("{by:.1} mm beyond the {side} edge"))
                .collect();
            // Half the row or more off the board: a row merely hanging
            // over an edge (eurorack jacks reach 2 mm past the board, as
            // the designer's do) is the place entry's to decide.
            let off = center[0] < bounds[0]
                || center[0] > bounds[2]
                || center[1] < bounds[1]
                || center[1] > bounds[3];
            if off && !out.is_empty() {
                cannot.push(format!(
                    "the row of {} runs from where its place entry puts it ({:.2}, {:.2}, {} degrees) along its axis in that part's frame and reaches {}: its other parts would lie off the board; place {} at the row's other end, or give it the angle a half turn from this one",
                    references[*leader],
                    pose.position[0],
                    pose.position[1],
                    pose.angle,
                    out.join(" and "),
                    references[*leader]
                ));
            }
        }
    }
    cannot
}
