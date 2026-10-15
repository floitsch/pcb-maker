// Copyright (C) 2026 Toit contributors.

//! Footprints taken from a board (`import-kicad-netlist --footprints-from`):
//! a designer's board has every footprint its schematic names, in the
//! version the board was made with, where the libraries may lack one (a
//! personal library that is not in the repository) or carry a later version
//! (a courtyard that grew, a hotswap socket redrawn).
//!
//! A board's footprint is brought back to library form the way KiCad's
//! "Export Footprints to Library" does: a back footprint flipped to the
//! front, the pose removed (pad and text angles, which a board file keeps
//! absolute, made relative again; zones, which it keeps in board
//! coordinates, moved into the footprint's frame), the links to the symbol
//! and the pads' nets dropped.

use super::*;

/// One footprint of the board, with what tells it apart.
#[derive(Clone, Debug)]
pub(crate) struct BoardFootprint {
    pub reference: String,
    /// The symbol it comes from (`/sheet/symbol`), when linked.
    pub path: Option<String>,
    /// Its footprint id as the board names it (`Library:Name`).
    pub id: String,
    pub uuid: Option<String>,
    pub expr: Expr,
}

impl BoardFootprint {
    /// The footprint's name without its library.
    pub fn name(&self) -> &str {
        name_of(&self.id)
    }
}

/// A footprint id's name without the library nickname.
pub(crate) fn name_of(id: &str) -> &str {
    id.rsplit_once(':').map_or(id, |(_, name)| name)
}

/// The footprints of a board file.
#[derive(Clone, Debug)]
pub(crate) struct BoardFootprints {
    pub file: PathBuf,
    pub footprints: Vec<BoardFootprint>,
}

/// How a schematic symbol's footprint was found on the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Found {
    /// The symbol's own footprint (by its path, else its reference).
    Own,
    /// Another footprint of that name (the symbol's own is not on the board).
    ByName,
}

impl BoardFootprints {
    pub fn read(file: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(file).map_err(|error| {
            crate::verdict::input_error(format!(
                "--footprints-from: failed to read {}: {error}",
                file.display()
            ))
        })?;
        let board = parse(&text).map_err(|error| {
            crate::verdict::input_error(format!("--footprints-from {}: {error}", file.display()))
        })?;
        if board.head() != Some("kicad_pcb") {
            return Err(crate::verdict::input_error(format!(
                "--footprints-from {}: not a KiCad board",
                file.display()
            )));
        }
        let footprints = board
            .children()
            .iter()
            .filter(|item| matches!(item.head(), Some("footprint" | "module")))
            .map(|footprint| BoardFootprint {
                reference: footprint_reference(footprint).unwrap_or_default(),
                path: form_atom(footprint, "path", 1).map(str::to_owned),
                id: footprint
                    .children()
                    .get(1)
                    .and_then(Expr::atom)
                    .unwrap_or("")
                    .to_string(),
                uuid: footprint_uuid(footprint),
                expr: footprint.clone(),
            })
            .collect();
        Ok(Self {
            file: file.to_path_buf(),
            footprints,
        })
    }

    /// The symbol's own footprint on the board: by its path, else by its
    /// reference when one footprint has it and that footprint is not linked
    /// to another of the schematic's symbols (`claimed`, their paths: a
    /// board renumbered after the schematic). Any footprint name.
    pub fn own(
        &self,
        path: Option<&str>,
        reference: &str,
        claimed: &BTreeSet<String>,
    ) -> Option<&BoardFootprint> {
        if let Some(path) = path
            && let Some(found) = self
                .footprints
                .iter()
                .find(|footprint| footprint.path.as_deref() == Some(path))
        {
            return Some(found);
        }
        let mut same = self.footprints.iter().filter(|footprint| {
            footprint.reference == reference
                && !footprint
                    .path
                    .as_ref()
                    .is_some_and(|path| claimed.contains(path))
        });
        match (same.next(), same.next()) {
            (Some(found), None) => Some(found),
            _ => None,
        }
    }

    /// The board's version of the footprint `id` for a symbol: its own
    /// footprint (whatever its name), else (`by_name`) the first footprint
    /// of that name.
    pub fn for_symbol(
        &self,
        path: Option<&str>,
        reference: &str,
        id: &str,
        by_name: bool,
        claimed: &BTreeSet<String>,
    ) -> Option<(&BoardFootprint, Found)> {
        let name = name_of(id);
        if let Some(own) = self.own(path, reference, claimed) {
            return Some((own, Found::Own));
        }
        if by_name {
            return self
                .footprints
                .iter()
                .find(|footprint| footprint.name() == name)
                .map(|footprint| (footprint, Found::ByName));
        }
        None
    }

    /// The footprint `reference`; where the reference repeats (mounting
    /// holes all called `H`, `REF**`), the one at `at`.
    pub fn by_reference(
        &self,
        reference: &str,
        at: Option<[f64; 2]>,
    ) -> Result<&BoardFootprint, String> {
        let same: Vec<&BoardFootprint> = self
            .footprints
            .iter()
            .filter(|footprint| footprint.reference == reference)
            .collect();
        match (at, same.as_slice()) {
            (_, []) => Err(format!(
                "{} has no footprint {reference}",
                self.file.display()
            )),
            (_, [one]) => Ok(one),
            (None, _) => Err(format!(
                "{} has {} footprints {reference}: say which with x and y where the board has it (origin absolute)",
                self.file.display(),
                same.len()
            )),
            (Some(at), _) => same
                .into_iter()
                .find(|footprint| {
                    form_at(&footprint.expr).is_ok_and(|own| {
                        (own[0] - at[0]).abs() < 1e-3 && (own[1] - at[1]).abs() < 1e-3
                    })
                })
                .ok_or_else(|| {
                    format!(
                        "{} has no footprint {reference} at {} {}",
                        self.file.display(),
                        at[0],
                        at[1]
                    )
                }),
        }
    }
}

/// An angle in (-180, 180], without float noise.
fn normal_angle(angle: f64) -> f64 {
    let mut angle = angle.rem_euclid(360.0);
    if angle > 180.0 {
        angle -= 360.0;
    }
    let rounded = (angle * 1e6).round() / 1e6;
    if rounded == 0.0 { 0.0 } else { rounded }
}

fn number(value: f64) -> Expr {
    let rounded = (value * 1e6).round() / 1e6;
    Expr::Atom(if rounded == 0.0 {
        "0".into()
    } else {
        rounded.to_string()
    })
}

/// Adds `turn` to the angle of an item's `(at x y [angle])` (pads and
/// texts, whose angles a board file keeps absolute).
fn turn_at(item: &mut Expr, turn: f64) {
    let Expr::List(parts) = item else { return };
    let Some(Expr::List(at)) = parts.iter_mut().find(|part| part.head() == Some("at")) else {
        return;
    };
    let angle = at
        .get(3)
        .and_then(Expr::atom)
        .and_then(|atom| atom.parse::<f64>().ok());
    let unlocked = at.iter().any(|value| value.atom() == Some("unlocked"));
    let new = normal_angle(angle.unwrap_or(0.0) + turn);
    at.truncate(3);
    if new != 0.0 || angle.is_some() {
        at.push(number(new));
    }
    if unlocked {
        at.push(Expr::Atom("unlocked".into()));
    }
}

/// Applies `map` to every point (`xy`) of a zone inside a footprint.
fn map_zone_points(zone: &mut Expr, map: &dyn Fn([f64; 2]) -> [f64; 2]) {
    let Expr::List(items) = zone else { return };
    if items.first().and_then(Expr::atom) == Some("xy") {
        if let (Some(x), Some(y)) = (
            items
                .get(1)
                .and_then(Expr::atom)
                .and_then(|atom| atom.parse::<f64>().ok()),
            items
                .get(2)
                .and_then(Expr::atom)
                .and_then(|atom| atom.parse::<f64>().ok()),
        ) {
            let [x, y] = map([x, y]);
            items[1] = number(x);
            items[2] = number(y);
        }
        return;
    }
    for item in items.iter_mut() {
        map_zone_points(item, map);
    }
}

/// Turns every pad and text of a footprint by `turn` degrees and maps its
/// zones' points: what moving the footprint's pose does to a board file.
fn repose(items: &mut [Expr], turn: f64, zones: &dyn Fn([f64; 2]) -> [f64; 2]) {
    for item in items.iter_mut() {
        match item.head() {
            Some("pad" | "fp_text" | "property") => turn_at(item, turn),
            Some("zone") => map_zone_points(item, zones),
            _ => {}
        }
    }
}

/// A board's footprint in library form, named `name` (`Library:Name` or
/// `Name`): on the front, at the origin, unrotated, without links to a
/// symbol or nets.
pub(crate) fn library_form(footprint: &Expr, name: &str) -> Result<Expr, String> {
    let mut footprint = footprint.clone();
    if let Expr::List(items) = &mut footprint {
        items[0] = Expr::Atom("footprint".into());
    }
    if form_atom(&footprint, "layer", 1) == Some("B.Cu") {
        crate::flip::flip_footprint(&mut footprint)?;
    }
    let [x, y, angle] = form_at(&footprint).unwrap_or([0.0; 3]);
    let Expr::List(items) = &mut footprint else {
        return Err("footprint is not a list".into());
    };
    // Board point to the footprint's frame: board = at + R(-angle) local.
    let to_local = move |point: [f64; 2]| rotate_vector([point[0] - x, point[1] - y], angle);
    repose(&mut items[2..], -angle, &to_local);
    items[1] = Expr::Atom(quote(name));
    items.retain(|item| {
        !matches!(
            item.head(),
            Some(
                "at" | "uuid"
                    | "tstamp"
                    | "tedit"
                    | "path"
                    | "sheetname"
                    | "sheetfile"
                    | "locked"
                    | "placed"
            )
        ) && !matches!(item.atom(), Some("locked" | "placed"))
    });
    for item in items.iter_mut() {
        if item.head() == Some("pad")
            && let Expr::List(parts) = item
        {
            parts.retain(|part| !matches!(part.head(), Some("net" | "pinfunction" | "pintype")));
        }
        if item.head() == Some("attr")
            && let Expr::List(parts) = item
        {
            parts.retain(|part| part.atom() != Some("board_only"));
        }
        // KiCad 7 boards lock texts: `(fp_text reference locked "C2" ...)`;
        // a library's text has no such flag.
        if item.head() == Some("fp_text")
            && let Expr::List(parts) = item
            && matches!(parts.get(2), Some(Expr::Atom(atom)) if atom == "locked")
        {
            parts.remove(2);
        }
    }
    Ok(footprint)
}

/// A library-form footprint put at a pose on the front: `(at x y angle)`
/// set, pad and text angles made absolute, zones in board coordinates.
pub(crate) fn posed(footprint: &Expr, at: [f64; 3]) -> Result<Expr, String> {
    let mut footprint = footprint.clone();
    let Expr::List(items) = &mut footprint else {
        return Err("footprint is not a list".into());
    };
    let to_board = move |point: [f64; 2]| {
        let offset = rotate_vector(point, -at[2]);
        [at[0] + offset[0], at[1] + offset[1]]
    };
    repose(&mut items[2..], at[2], &to_board);
    items.retain(|item| item.head() != Some("at"));
    let layer = items
        .iter()
        .position(|item| item.head() == Some("layer"))
        .map_or(2, |index| index + 1);
    let angle = normal_angle(at[2]);
    let pose = if angle == 0.0 {
        format!("(at {} {})", encode(&number(at[0])), encode(&number(at[1])))
    } else {
        format!(
            "(at {} {} {})",
            encode(&number(at[0])),
            encode(&number(at[1])),
            encode(&number(angle))
        )
    };
    items.insert(layer, parse(&pose)?);
    Ok(footprint)
}

/// The size of a footprint's body (its courtyard, else its pads), in its
/// own frame, and its pad count.
pub(crate) fn body_size(footprint: &Expr) -> Option<([f64; 2], usize)> {
    // The body is read from a footprint on a board: at the origin.
    let footprint = if footprint.child("at").is_some() {
        footprint.clone()
    } else {
        posed(footprint, [0.0; 3]).ok()?
    };
    let footprint = &footprint;
    let (_, size, _) = crate::board_placer::local_body(footprint).ok()?;
    let pads = footprint
        .children()
        .iter()
        .filter(|item| item.head() == Some("pad"))
        .count();
    Some((size, pads))
}

/// Whether a library footprint's body differs from the board's by more
/// than a fifth on a side, or in its pad count.
pub(crate) fn drifted(library: ([f64; 2], usize), board: ([f64; 2], usize)) -> bool {
    let side = |a: f64, b: f64| (a - b).abs() > 0.2 * b.max(0.1);
    side(library.0[0], board.0[0]) || side(library.0[1], board.0[1]) || library.1 != board.1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn a_rotated_back_footprint_comes_back_to_library_form() {
        // A library footprint: pad 1 at (-1, 0), pad 2 at (1, 0.5) turned
        // 90 degrees, a courtyard 4 x 2, a keepout zone on the front.
        let library = parse(
            r#"(footprint "Lib:Part" (layer "F.Cu")
                (property "Reference" "REF**" (at 0 -2 0) (layer "F.SilkS"))
                (fp_rect (start -2 -1) (end 2 1) (layer "F.CrtYd") (width 0.05))
                (pad "1" smd rect (at -1 0) (size 1 0.5) (layers "F.Cu" "F.Mask"))
                (pad "2" smd rect (at 1 0.5 90) (size 1 0.5) (layers "F.Cu" "F.Mask"))
                (zone (layer "F.Cu") (keepout (tracks not_allowed)) (polygon (pts (xy 2 -1) (xy 3 -1) (xy 3 1)))))"#,
        )
        .unwrap();
        // On a board: at (50, 30), turned 30 degrees, then flipped to the
        // back, linked to a symbol, pads on nets.
        let mut board = posed(&library, [50.0, 30.0, 30.0]).unwrap();
        crate::flip::flip_footprint(&mut board).unwrap();
        let Expr::List(items) = &mut board else {
            unreachable!()
        };
        items.insert(3, parse(r#"(path "/abc")"#).unwrap());
        items.insert(3, parse(r#"(uuid "1234")"#).unwrap());
        for item in items.iter_mut().filter(|item| item.head() == Some("pad")) {
            if let Expr::List(parts) = item {
                parts.push(parse(r#"(net 3 "GND")"#).unwrap());
            }
        }
        assert_eq!(form_atom(&board, "layer", 1), Some("B.Cu"));
        let back = library_form(&board, "Lib:Part").unwrap();
        assert_eq!(form_atom(&back, "layer", 1), Some("F.Cu"));
        assert!(
            back.child("at").is_none()
                && back.child("path").is_none()
                && back.child("uuid").is_none()
        );
        let pads: Vec<&Expr> = back
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
            .collect();
        let at = |pad: &Expr| form_at(pad).unwrap();
        assert!(
            near(at(pads[0])[0], -1.0) && near(at(pads[0])[1], 0.0) && near(at(pads[0])[2], 0.0),
            "{:?}",
            at(pads[0])
        );
        assert!(
            near(at(pads[1])[0], 1.0) && near(at(pads[1])[1], 0.5) && near(at(pads[1])[2], 90.0),
            "{:?}",
            at(pads[1])
        );
        assert!(pads.iter().all(|pad| pad.child("net").is_none()));
        assert_eq!(form_atom(pads[0], "layers", 1), Some("F.Cu"));
        let zone = encode(back.child("zone").unwrap());
        assert!(
            zone.contains("(xy 2 -1)") && zone.contains("(xy 3 1)"),
            "{zone}"
        );
        // The body is the library's.
        assert_eq!(
            body_size(&back).map(|(size, pads)| ([size[0].round(), size[1].round()], pads)),
            Some(([4.0, 2.0], 2))
        );
    }

    #[test]
    fn drift_is_a_fifth_on_a_side_or_another_pad_count() {
        assert!(!drifted(([4.0, 2.0], 2), ([4.4, 2.2], 2)));
        assert!(drifted(([15.9, 12.2], 4), ([12.85, 11.6], 4)));
        assert!(drifted(([4.0, 2.0], 3), ([4.0, 2.0], 2)));
    }
}
