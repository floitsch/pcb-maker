// Copyright (C) 2026 Toit contributors.

//! The `add` entries of an existing `constraints.json`: parts the schematic
//! does not have.

use super::*;
use crate::placement_constraints::KiCadAddConstraint;

/// An entry that cannot be carried out: an input error naming the part.
fn invalid(part: &str, why: String) -> String {
    crate::verdict::input_error(format!("constraints.json add {part}: {why}"))
}

/// A part's footprint in library form, the board's nets of its pads, its
/// pose there (position, angle, back side) and its id.
struct AddedForm {
    form: Expr,
    board_nets: BTreeMap<String, String>,
    board_pose: Option<([f64; 3], bool)>,
    id: String,
    kept: Option<String>,
}

/// The parts of `add` in `<out>/constraints.json`, put on the board
/// `board_only`; returns a line per part. A library footprint not found
/// goes to `missing`; an entry that cannot be carried out is an input
/// error.
pub(super) fn add_parts(
    board: &mut Expr,
    output: &Path,
    libraries: &BTreeMap<String, PathBuf>,
    designer: Option<&crate::board_footprints::BoardFootprints>,
    missing: &mut Vec<String>,
    kept_ids: &mut BTreeSet<String>,
) -> Result<Vec<String>, String> {
    let Some(additions) = read_additions(output)? else {
        return Ok(Vec::new());
    };
    let nets_on_board: BTreeSet<String> = board
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .flat_map(|footprint| {
            footprint
                .children()
                .iter()
                .filter(|item| item.head() == Some("pad"))
                .filter_map(|pad| form_atom(pad, "net", 1))
        })
        .map(str::to_owned)
        .collect();
    let mut lines = Vec::new();
    for addition in additions {
        let Some((footprint, line)) = add_part(
            &addition,
            board,
            &nets_on_board,
            libraries,
            designer,
            missing,
            kept_ids,
        )?
        else {
            continue;
        };
        lines.push(line);
        if let Expr::List(items) = board {
            items.push(footprint);
        }
    }
    Ok(lines)
}

/// The `add` entries of `<out>/constraints.json`; `None` where there is no
/// such file or no `add` in it.
fn read_additions(output: &Path) -> Result<Option<Vec<KiCadAddConstraint>>, String> {
    let path = output.join("constraints.json");
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(None);
    };
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|error| {
        crate::verdict::input_error(format!("invalid constraints {}: {error}", path.display()))
    })?;
    let Some(entries) = value.get("add") else {
        return Ok(None);
    };
    serde_json::from_value(entries.clone())
        .map(Some)
        .map_err(|error| {
            crate::verdict::input_error(format!("invalid add in {}: {error}", path.display()))
        })
}

/// One `add` entry as a footprint for the board and its report line; `None`
/// where its library footprint is missing (then in `missing`).
fn add_part(
    addition: &KiCadAddConstraint,
    board: &Expr,
    nets_on_board: &BTreeSet<String>,
    libraries: &BTreeMap<String, PathBuf>,
    designer: Option<&crate::board_footprints::BoardFootprints>,
    missing: &mut Vec<String>,
    kept_ids: &mut BTreeSet<String>,
) -> Result<Option<(Expr, String)>, String> {
    let part = addition.part.as_str();
    let (side_back, absolute) = check_addition(addition, board)?;
    let Some(AddedForm {
        form,
        board_nets,
        board_pose,
        id,
        kept,
    }) = added_form(addition, absolute, libraries, designer, missing, kept_ids)?
    else {
        return Ok(None);
    };
    let nets = addition.nets.clone().unwrap_or(board_nets);
    let (at, back, locked) = added_pose(addition, side_back, absolute, board_pose);
    let footprint = dress_added(form, part, &id, &nets, kept.is_some())?;
    let uuid = kept.unwrap_or_else(|| uuid_of(&format!("add footprint {part}")));
    let mut footprint =
        crate::board_footprints::posed(&footprint, if back { [at[0], at[1], -at[2]] } else { at })?;
    if back {
        crate::flip::flip_footprint(&mut footprint)?;
    }
    if let Expr::List(items) = &mut footprint {
        let layer = items
            .iter()
            .position(|item| item.head() == Some("layer"))
            .map_or(2, |index| index + 1);
        items.insert(layer, parse(&format!("(uuid \"{uuid}\")"))?);
        if locked {
            items.insert(layer, parse("(locked yes)")?);
        }
    }
    let line = added_line(
        addition,
        &id,
        &nets,
        nets_on_board,
        at,
        back,
        locked,
        absolute,
    );
    Ok(Some((footprint, line)))
}

/// The entry's reference, side, position and origin as the board allows
/// them: returns whether it asks for the back side (if it says) and
/// whether its position is absolute.
fn check_addition(
    addition: &KiCadAddConstraint,
    board: &Expr,
) -> Result<(Option<bool>, bool), String> {
    let part = addition.part.as_str();
    if part.is_empty() {
        return Err(invalid(
            part,
            "give the new part's reference in part".into(),
        ));
    }
    let taken = board
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .any(|footprint| footprint_reference(footprint).as_deref() == Some(part));
    if taken {
        return Err(invalid(
            part,
            "the board has a part of that reference already".into(),
        ));
    }
    let side_back = match addition.side.as_deref() {
        None => None,
        Some("front") => Some(false),
        Some("back") => Some(true),
        Some(other) => {
            return Err(invalid(
                part,
                format!("side {other:?} is neither front nor back"),
            ));
        }
    };
    if addition.x.is_some() != addition.y.is_some() {
        return Err(invalid(part, "give both x and y, or neither".into()));
    }
    let absolute = match addition.origin.as_deref() {
        None | Some("board") => false,
        Some("absolute") => true,
        Some(other) => {
            return Err(invalid(
                part,
                format!("unknown origin {other:?} (board or absolute)"),
            ));
        }
    };
    Ok((side_back, absolute))
}

/// The footprint of an entry, from the libraries or the `--footprints-from`
/// board; `None` where a library footprint is not found or unreadable
/// (then in `missing`).
fn added_form(
    addition: &KiCadAddConstraint,
    absolute: bool,
    libraries: &BTreeMap<String, PathBuf>,
    designer: Option<&crate::board_footprints::BoardFootprints>,
    missing: &mut Vec<String>,
    kept_ids: &mut BTreeSet<String>,
) -> Result<Option<AddedForm>, String> {
    let part = addition.part.as_str();
    match (&addition.footprint, addition.from_board) {
        (Some(_), true) | (None, false) => {
            Err(invalid(part, "give footprint (a library footprint) or from_board (the --footprints-from board's), one of them".into()))
        }
        (None, true) => {
            let designer =
                designer.ok_or_else(|| invalid(part, "from_board takes the footprint from the board given with --footprints-from".into()))?;
            let at = match (addition.x, addition.y) {
                (Some(x), Some(y)) if absolute => Some([x, y]),
                _ => None,
            };
            let theirs = designer
                .by_reference(addition.board_reference.as_deref().unwrap_or(part), at)
                .map_err(|why| invalid(part, why))?;
            let nets: BTreeMap<String, String> = theirs
                .expr
                .children()
                .iter()
                .filter(|item| item.head() == Some("pad"))
                .filter_map(|pad| {
                    let number = pad.children().get(1).and_then(Expr::atom)?;
                    let net = pad.child("net")?.children().last()?.atom()?;
                    (!net.is_empty() && !number.is_empty())
                        .then(|| (number.to_string(), net.to_string()))
                })
                .collect();
            let at = form_at(&theirs.expr).unwrap_or([0.0; 3]);
            let back = form_atom(&theirs.expr, "layer", 1) == Some("B.Cu");
            let kept = kept_uuid(theirs, kept_ids);
            Ok(Some(AddedForm {
                form: crate::board_footprints::library_form(
                    &theirs.expr,
                    crate::board_footprints::name_of(&theirs.id),
                )?,
                board_nets: nets,
                board_pose: Some((at, back)),
                id: theirs.id.clone(),
                kept,
            }))
        }
        (Some(id), false) => {
            let Some((file, found)) = find_footprint(libraries, id) else {
                missing.push(format!("{part} (add: {id} not found in the libraries)"));
                return Ok(None);
            };
            match fs::read_to_string(&file)
                .map_err(|error| error.to_string())
                .and_then(|text| parse(&text))
                .and_then(|footprint| match footprint_form_error(&footprint) {
                    Some(why) => Err(format!("the file holds {why}")),
                    None => Ok(footprint),
                }) {
                Ok(footprint) => Ok(Some(AddedForm {
                    form: footprint,
                    board_nets: BTreeMap::new(),
                    board_pose: None,
                    id: found,
                    kept: None,
                })),
                Err(error) => {
                    missing.push(format!("{part} (add: {found} unreadable: {error})"));
                    Ok(None)
                }
            }
        }
    }
}

/// Where: x and y as given (at the origin when they are relative to
/// the outline, which the layout draws: its `place` puts it there),
/// else where the board has it (locked), else anywhere. Returns position
/// and angle, whether it is on the back, and whether it is locked.
fn added_pose(
    addition: &KiCadAddConstraint,
    side_back: Option<bool>,
    absolute: bool,
    board_pose: Option<([f64; 3], bool)>,
) -> ([f64; 3], bool, bool) {
    match (addition.x, addition.y, board_pose) {
        (Some(x), Some(y), pose) => {
            let angle = addition.angle.or(pose.map(|(at, _)| at[2])).unwrap_or(0.0);
            let at = if absolute {
                [x, y, angle]
            } else {
                [0.0, 0.0, angle]
            };
            (
                at,
                side_back.or(pose.map(|(_, back)| back)).unwrap_or(false),
                false,
            )
        }
        (_, _, Some((at, back))) => (
            [at[0], at[1], addition.angle.unwrap_or(at[2])],
            side_back.unwrap_or(back),
            true,
        ),
        _ => (
            [0.0, 0.0, addition.angle.unwrap_or(0.0)],
            side_back.unwrap_or(false),
            false,
        ),
    }
}

/// The footprint form as the part `part`: its ids, reference, `board_only`
/// flag and pads on `nets`; an entry for a pad the footprint lacks is an
/// input error.
fn dress_added(
    mut footprint: Expr,
    part: &str,
    id: &str,
    nets: &BTreeMap<String, String>,
    kept: bool,
) -> Result<Expr, String> {
    let Expr::List(items) = &mut footprint else {
        return Err(invalid(part, "the footprint is not a list".into()));
    };
    items.retain(|item| {
        !matches!(
            item.head(),
            Some("version" | "generator" | "generator_version" | "at" | "uuid" | "tstamp")
        )
    });
    items[0] = Expr::Atom("footprint".into());
    items[1] = Expr::Atom(requote(id));
    if !kept {
        for item in items.iter_mut() {
            renew_uuids(item, &format!("add {part}"));
        }
    }
    legacy_texts_to_properties(items, part)?;
    legacy_arcs_to_three_points(items)?;
    set_property(items, "Reference", part, part)?;
    // Not in the schematic: KiCad's parity check leaves it out.
    let attr = match items.iter().position(|item| item.head() == Some("attr")) {
        Some(index) => index,
        None => {
            let at = items
                .iter()
                .position(|item| {
                    item.head() == Some("pad")
                        || item.head().is_some_and(|head| head.starts_with("fp_"))
                })
                .unwrap_or(items.len());
            items.insert(at, Expr::List(vec![Expr::Atom("attr".into())]));
            at
        }
    };
    if let Expr::List(flags) = &mut items[attr]
        && !flags.iter().any(|flag| flag.atom() == Some("board_only"))
    {
        flags.push(Expr::Atom("board_only".into()));
    }
    let mut unused: BTreeSet<&String> = nets.keys().collect();
    for item in items.iter_mut().filter(|item| item.head() == Some("pad")) {
        let number = item
            .children()
            .get(1)
            .and_then(Expr::atom)
            .unwrap_or("")
            .to_string();
        let Expr::List(parts) = item else { continue };
        parts.retain(|part| !matches!(part.head(), Some("net" | "pinfunction" | "pintype")));
        if let Some(net) = nets.get(&number) {
            unused.remove(&number);
            parts.push(parse(&format!("(net {})", quote(net)))?);
        }
    }
    if let Some(number) = unused.first() {
        return Err(invalid(part, format!("{id} has no pad {number}")));
    }
    Ok(footprint)
}

/// The report's line of an added part: its pads' nets, where it goes, and
/// the nets the schematic does not have.
#[allow(clippy::too_many_arguments)]
fn added_line(
    addition: &KiCadAddConstraint,
    id: &str,
    nets: &BTreeMap<String, String>,
    nets_on_board: &BTreeSet<String>,
    at: [f64; 3],
    back: bool,
    locked: bool,
    absolute: bool,
) -> String {
    let part = addition.part.as_str();
    let mut on: Vec<String> = Vec::new();
    let mut net_names: BTreeSet<&String> = nets.values().collect::<BTreeSet<_>>();
    for (number, net) in nets {
        on.push(format!(
            "{} {net}",
            if number.is_empty() { "\"\"" } else { number }
        ));
    }
    net_names.retain(|net| !nets_on_board.contains(net.as_str()));
    let mut line = format!("{part}: {id}");
    if !on.is_empty() {
        line.push_str(&format!(", pads on nets ({})", on.join(", ")));
    }
    line.push_str(&match (addition.x, locked) {
        (Some(_), _) => format!(
            ", at {} {} ({})",
            addition.x.unwrap_or(0.0),
            addition.y.unwrap_or(0.0),
            if absolute {
                "absolute"
            } else {
                "from the outline's corner, by the layout"
            }
        ),
        (None, true) => format!(", where the board has it ({} {}, locked)", at[0], at[1]),
        (None, false) => ", placed by the layout".into(),
    });
    if back {
        line.push_str(", on the back");
    }
    if !net_names.is_empty() {
        line.push_str(&format!(
            "; nets the schematic does not have: {}",
            net_names
                .into_iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    line
}
