// Copyright (C) 2026 Toit contributors.

//! Footprint forms and the netlist's strings: bringing a library footprint to
//! today's form, its properties and ids.

use super::*;
use pcb_kicad_file::kicad_nets::fp_text_value_index;

/// A deterministic UUID from text (version 4 layout).
pub(super) fn uuid_of(text: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut second = 0x8422_2325_cbf2_9ce4u64;
    for byte in text.bytes() {
        hash = (hash ^ byte as u64).wrapping_mul(0x100_0000_01b3);
        second = (second ^ byte as u64)
            .wrapping_mul(0x100_0000_01b3)
            .rotate_left(5);
    }
    format!(
        "{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}",
        hash >> 32,
        (hash >> 16) & 0xffff,
        hash & 0xfff,
        second & 0xfff,
        (second >> 12) & 0xffff_ffff_ffff
    )
}

/// Turns a KiCad 6 library footprint's `(fp_text reference ...)` and
/// `(fp_text value ...)` into the `Reference` and `Value` properties KiCad 8
/// and later write, keeping their place, layer and font. Left as they were,
/// KiCad reads the library's text (`Bat+0`) as the reference over the
/// property the import sets: every copy of the footprint got one reference,
/// and the schematic parity check found 73 duplicate and 73 missing
/// footprints on urchin.
pub(super) fn legacy_texts_to_properties(
    items: &mut [Expr],
    reference: &str,
) -> Result<(), String> {
    for item in items.iter_mut() {
        let Expr::List(parts) = item else { continue };
        let name = match (
            parts.first().and_then(Expr::atom),
            parts.get(1).and_then(Expr::atom),
        ) {
            (Some("fp_text"), Some("reference")) => "Reference",
            (Some("fp_text"), Some("value")) => "Value",
            _ => continue,
        };
        let value_index = fp_text_value_index(parts);
        let mut converted = vec![Expr::Atom("property".into()), Expr::Atom(requote(name))];
        converted.extend(parts.get(value_index).cloned());
        if value_index == 3 {
            converted.push(parse("(locked yes)")?);
        }
        let mut hidden = false;
        let mut unlocked = false;
        for part in parts.iter().skip(value_index + 1) {
            match part {
                Expr::Atom(atom) if atom == "hide" => hidden = true,
                Expr::List(at) if part.head() == Some("at") => {
                    // `(at x y [angle] [unlocked])`: the angle is required.
                    let mut numbers: Vec<Expr> = at
                        .iter()
                        .skip(1)
                        .filter(|value| value.atom() != Some("unlocked"))
                        .cloned()
                        .collect();
                    unlocked |= at.iter().any(|value| value.atom() == Some("unlocked"));
                    while numbers.len() < 3 {
                        numbers.push(Expr::Atom("0".into()));
                    }
                    converted.push(Expr::List(
                        [vec![Expr::Atom("at".into())], numbers].concat(),
                    ));
                }
                Expr::List(_) if part.head() == Some("tstamp") => {}
                other => converted.push(other.clone()),
            }
        }
        if unlocked {
            converted.push(parse("(unlocked yes)")?);
        }
        if hidden {
            converted.push(parse("(hide yes)")?);
        }
        converted.push(parse(&format!(
            "(uuid \"{}\")",
            uuid_of(&format!("property {reference} {name}"))
        ))?);
        *item = Expr::List(converted);
    }
    Ok(())
}

/// Turns KiCad 5 arcs (`start` the centre, `end` where the arc starts,
/// `angle` in degrees), which KiCad 6 library files still carry, into the
/// three points KiCad 7 and later write (start, mid, end on the arc). KiCad
/// reads the old form only in a file of that age: a board of today with one
/// does not load (osprey's Tag-Connect footprint: kicad-cli exit 3, the
/// import failed in its pad check).
pub(super) fn legacy_arcs_to_three_points(items: &mut [Expr]) -> Result<(), String> {
    for item in items.iter_mut() {
        if item.head() != Some("fp_arc") || item.child("mid").is_some() {
            continue;
        }
        let Some(angle) = form_atom(item, "angle", 1).and_then(|angle| angle.parse::<f64>().ok())
        else {
            continue;
        };
        let (Ok(cx), Ok(cy), Ok(sx), Ok(sy)) = (
            form_f64(item, "start", 1),
            form_f64(item, "start", 2),
            form_f64(item, "end", 1),
            form_f64(item, "end", 2),
        ) else {
            continue;
        };
        let radius = (sx - cx).hypot(sy - cy);
        let first = (sy - cy).atan2(sx - cx);
        // KiCad's angle turns from the start towards +y (down) for a
        // positive value: the same sense as atan2 in board coordinates.
        let point = |name: &str, turn: f64| {
            let at = first + turn.to_radians();
            parse(&format!(
                "({name} {} {})",
                round_mm(cx + radius * at.cos()),
                round_mm(cy + radius * at.sin())
            ))
        };
        let replacement = [
            parse(&format!("(start {sx} {sy})"))?,
            point("mid", angle / 2.0)?,
            point("end", angle)?,
        ];
        let Expr::List(parts) = item else { continue };
        parts.retain(|part| !matches!(part.head(), Some("start" | "end" | "angle")));
        parts.splice(1..1, replacement);
    }
    Ok(())
}

pub(super) fn round_mm(value: f64) -> f64 {
    let rounded = (value * 1e6).round() / 1e6;
    if rounded == 0.0 { 0.0 } else { rounded }
}

/// Sets a footprint property's value, adding it (hidden, on the fab layer,
/// as KiCad does) when the footprint has none of that name.
pub(super) fn set_property(
    items: &mut Vec<Expr>,
    name: &str,
    value: &str,
    reference: &str,
) -> Result<(), String> {
    for item in items
        .iter_mut()
        .filter(|item| item.head() == Some("property"))
    {
        if let Expr::List(parts) = item
            && parts.get(1).and_then(Expr::atom) == Some(name)
            && parts.len() > 2
        {
            parts[2] = Expr::Atom(requote(value));
            return Ok(());
        }
    }
    let last = items
        .iter()
        .rposition(|item| item.head() == Some("property"))
        .map_or(items.len(), |index| index + 1);
    items.insert(
        last,
        parse(&format!(
            "(property {} {} (at 0 0 0) (unlocked yes) (layer \"F.Fab\") (hide yes) (uuid \"{}\") (effects (font (size 1.27 1.27) (thickness 0.15))))",
            requote(name),
            requote(value),
            uuid_of(&format!("property {reference} {name}"))
        ))?,
    );
    Ok(())
}

/// Gives every UUID inside a footprint instance its own value (library
/// files carry one set for all instances; KiCad makes new ones when it
/// adds a footprint). Update group member links along with the IDs they
/// point at. KiCad 6 and 7 use `tstamp` for items and `id` for groups.
pub(super) fn renew_uuids(item: &mut Expr, owner: &str) {
    let renew = |id: &mut Expr| {
        if let Some(old) = id.atom() {
            *id = Expr::Atom(requote(&uuid_of(&format!(
                "{owner} {}",
                old.to_ascii_lowercase()
            ))));
        }
    };
    let mut pending = vec![item];
    while let Some(item) = pending.pop() {
        let Expr::List(items) = item else {
            continue;
        };
        match items.first().and_then(Expr::atom) {
            Some("uuid" | "tstamp") => {
                if let Some(id) = items.get_mut(1) {
                    renew(id);
                }
                continue;
            }
            Some("group") => {
                for field in items.iter_mut().skip(2) {
                    let Expr::List(values) = field else { continue };
                    match values.first().and_then(Expr::atom) {
                        Some("members") => values.iter_mut().skip(1).for_each(renew),
                        Some("id") => {
                            if let Some(id) = values.get_mut(1) {
                                renew(id);
                            }
                        }
                        _ => (),
                    }
                }
            }
            _ => (),
        }
        pending.extend(items.iter_mut());
    }
}

/// Gives pads from legacy libraries and boards an ID before KiCad assigns
/// one at load time. Pad numbers can repeat or be empty, so use their order
/// within each footprint instance. Keep existing IDs for DRC exclusions.
pub(super) fn fill_pad_uuids(board: &mut Expr) {
    let mut used = BTreeSet::new();
    let mut pending = vec![&*board];
    while let Some(item) = pending.pop() {
        if matches!(item.head(), Some("uuid" | "tstamp"))
            && let Some(id) = item.children().get(1).and_then(Expr::atom)
        {
            used.insert(id.trim_matches('"').to_ascii_lowercase());
        }
        pending.extend(item.children());
    }

    let Expr::List(items) = board else { return };
    let mut instances = BTreeMap::<String, usize>::new();
    for footprint in items
        .iter_mut()
        .filter(|item| item.head() == Some("footprint"))
    {
        let owner = form_atom(footprint, "uuid", 1)
            .unwrap_or_default()
            .to_owned();
        let instance = instances.entry(owner.clone()).or_default();
        let Expr::List(children) = footprint else {
            continue;
        };
        for (index, pad) in children
            .iter_mut()
            .filter(|item| item.head() == Some("pad"))
            .enumerate()
        {
            if pad.child("uuid").is_some() || pad.child("tstamp").is_some() {
                continue;
            }
            let Expr::List(parts) = pad else { continue };
            let id = (0..)
                .map(|retry| uuid_of(&format!("pad {owner} {instance} {index} {retry}")))
                .find(|id| used.insert(id.clone()))
                .unwrap();
            parts.push(Expr::List(vec![
                Expr::Atom("uuid".into()),
                Expr::Atom(requote(&id)),
            ]));
        }
        *instance += 1;
    }
}

/// The ids a footprint taken from the `--footprints-from` board keeps: the
/// board footprint's own uuid, the first time that footprint is used (a
/// footprint used again, as a template for another part, gets new ones).
/// Its pads and drawings keep theirs too, as KiCad's "Update PCB from
/// Schematic" keeps a footprint's: the project's DRC exclusions name
/// items by these ids (`courtyards_overlap|x|y|<footprint>|<footprint>`,
/// `copper_edge_clearance|x|y|<edge>|<pad>`), and with new ids the
/// designer's excluded findings come back as errors (MokyaLora's
/// R34/R35, urchin's key sockets over the edge).
pub(super) fn kept_uuid(
    theirs: &crate::board_footprints::BoardFootprint,
    kept: &mut BTreeSet<String>,
) -> Option<String> {
    let uuid = theirs.uuid.as_deref()?.trim_matches('"').to_string();
    (!uuid.is_empty() && kept.insert(uuid.clone())).then_some(uuid)
}

/// A string from the netlist, still escaped as KiCad writes strings.
pub(super) fn text_of(node: &Expr, head: &str) -> Option<String> {
    form_atom(node, head, 1).map(str::to_owned)
}

/// A string `text_of` read, as an atom again.
pub(super) fn requote(escaped: &str) -> String {
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copying_a_grouped_footprint_remaps_items_and_nested_members_together() {
        let mut part = parse(
            r#"(footprint "model"
                (pad "1" smd rect (size 1 1) (tstamp ABCD))
                (group "inner" (id "inner1") (members "abcd"))
                (group "outer" (uuid "outer1") (members "inner1")))"#,
        )
        .unwrap();
        let original = part.clone();
        renew_uuids(&mut part, "J1");
        let items = &part.children()[2..];
        let pad = form_atom(&items[0], "tstamp", 1).unwrap();
        let inner = form_atom(&items[1], "id", 1).unwrap();
        assert_ne!(pad, "ABCD");
        assert_ne!(inner, "inner1");
        assert_ne!(form_atom(&items[2], "uuid", 1), Some("outer1"));
        assert_eq!(form_atom(&items[1], "members", 1), Some(pad));
        assert_eq!(form_atom(&items[2], "members", 1), Some(inner));
        assert!(crate::board_footprints::same_library_model(
            &part, &original
        ));
    }
}
