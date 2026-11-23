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
use sha2::{Digest, Sha256};

mod groups;
mod placement_roundoff;
pub(crate) use placement_roundoff::same_two_pad_donor_model;

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
    pub sha256: String,
    pub footprints: Vec<BoardFootprint>,
    pub net_codes: BTreeMap<String, String>,
    /// The board's copper layers (its `layers` list's `.Cu` entries).
    pub copper_layers: usize,
}

/// How a schematic symbol's footprint was found on the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Found {
    /// The symbol's own footprint (by its path, else its reference).
    Own,
    /// Another footprint of that name (the symbol's own is not on the board).
    ByName,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DonorModels {
    Same,
    PerInstanceDocumentation,
    Incompatible,
}

impl BoardFootprints {
    pub fn read(file: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(file).map_err(|error| {
            crate::exit::input_error(format!(
                "--footprints-from: failed to read {}: {error}",
                file.display()
            ))
        })?;
        let board = parse(&text).map_err(|error| {
            crate::exit::input_error(format!("--footprints-from {}: {error}", file.display()))
        })?;
        if board.head() != Some("kicad_pcb") {
            return Err(crate::exit::input_error(format!(
                "--footprints-from {}: not a KiCad board",
                file.display()
            )));
        }
        if let Some(why) = board
            .children()
            .iter()
            .filter(|item| matches!(item.head(), Some("footprint" | "module")))
            .find_map(footprint_form_error)
        {
            return Err(crate::exit::input_error(format!(
                "--footprints-from {}: the board has {why}",
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
        let copper_layers = board.child("layers").map_or(0, |layers| {
            layers
                .children()
                .iter()
                .filter_map(|layer| layer.children().get(1).and_then(Expr::atom))
                .filter(|name| name.ends_with(".Cu"))
                .count()
        });
        Ok(Self {
            file: file.to_path_buf(),
            sha256: format!("{:x}", Sha256::digest(text.as_bytes())),
            footprints,
            net_codes: net_codes(&board),
            copper_layers,
        })
    }

    /// The board's own footprint for each `(path, reference)` in the schematic.
    /// A path unique on both sides survives renumbering. A repeated path
    /// needs a unique reference within that group. Finally match the
    /// remaining symbols and footprints by reference where both are unique.
    pub fn assign<'a>(
        &'a self,
        symbols: &[(Option<&str>, &str)],
    ) -> Vec<Option<&'a BoardFootprint>> {
        let symbols_by_path = by_key(
            symbols
                .iter()
                .enumerate()
                .filter_map(|(index, (path, _))| path.map(|path| (path, index))),
        );
        let footprints_by_path = by_key(
            self.footprints
                .iter()
                .enumerate()
                .filter_map(|(index, footprint)| {
                    footprint.path.as_deref().map(|path| (path, index))
                }),
        );
        let mut assigned = vec![None; symbols.len()];
        let mut used = vec![false; self.footprints.len()];
        let mut assign = |symbol, footprint| {
            assigned[symbol] = Some(&self.footprints[footprint]);
            used[footprint] = true;
        };
        for (path, group) in symbols_by_path {
            let Some(candidates) = footprints_by_path.get(path) else {
                continue;
            };
            if let ([symbol], [footprint]) = (group.as_slice(), candidates.as_slice()) {
                assign(*symbol, *footprint);
                continue;
            }
            let references = group.into_iter().map(|index| (symbols[index].1, index));
            let donors = candidates
                .iter()
                .map(|&index| (self.footprints[index].reference.as_str(), index));
            for (symbol, footprint) in unique_pairs(references, donors) {
                assign(symbol, footprint);
            }
        }
        let remaining = symbols
            .iter()
            .enumerate()
            .filter(|(index, _)| assigned[*index].is_none())
            .map(|(index, (_, reference))| (*reference, index));
        let donors = self
            .footprints
            .iter()
            .enumerate()
            .filter(|(index, _)| !used[*index])
            .map(|(index, footprint)| (footprint.reference.as_str(), index));
        for (symbol, footprint) in unique_pairs(remaining, donors) {
            assigned[symbol] = Some(&self.footprints[footprint]);
        }
        assigned
    }

    /// A copy of a footprint model by name, for symbols without a library model.
    pub fn by_name(&self, id: &str) -> Option<&BoardFootprint> {
        let name = name_of(id);
        let exact = self.footprints.iter().any(|footprint| footprint.id == id);
        let mut matches = self.footprints.iter().filter(|footprint| {
            if exact {
                footprint.id == id
            } else {
                footprint.name() == name
            }
        });
        let model = matches.next()?;
        let form = library_form(&model.expr, name).ok()?;
        matches
            .all(|other| {
                library_form(&other.expr, name).is_ok_and(|other| same_library_model(&form, &other))
            })
            .then_some(model)
    }

    pub fn donor_models(
        &self,
        id: &str,
        form: &Expr,
        normalize: impl Fn(&BoardFootprint, &mut Expr),
        equivalent_presentation: fn(&Expr, &Expr) -> bool,
    ) -> DonorModels {
        let mut result = DonorModels::Same;
        for donor in self.footprints.iter().filter(|other| other.id == id) {
            let Ok(mut other) = library_form(&donor.expr, name_of(id)) else {
                return DonorModels::Incompatible;
            };
            normalize(donor, &mut other);
            if same_library_model(form, &other) {
                continue;
            }
            if !equivalent_presentation(form, &other) {
                return DonorModels::Incompatible;
            }
            result = DonorModels::PerInstanceDocumentation;
        }
        result
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

fn by_key<'a>(items: impl Iterator<Item = (&'a str, usize)>) -> BTreeMap<&'a str, Vec<usize>> {
    let mut groups = BTreeMap::<_, Vec<_>>::new();
    for (key, index) in items.filter(|(key, _)| !key.is_empty()) {
        groups.entry(key).or_default().push(index);
    }
    groups
}

fn unique_pairs<'a>(
    symbols: impl Iterator<Item = (&'a str, usize)>,
    footprints: impl Iterator<Item = (&'a str, usize)>,
) -> Vec<(usize, usize)> {
    let footprints = by_key(footprints);
    by_key(symbols)
        .into_iter()
        .filter_map(|(reference, group)| {
            let ([symbol], [footprint]) = (group.as_slice(), footprints.get(reference)?.as_slice())
            else {
                return None;
            };
            Some((*symbol, *footprint))
        })
        .collect()
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
        .and_then(|atom| atom.parse::<f64>().ok())
        .filter(|angle| angle.is_finite());
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
                .and_then(|atom| atom.parse::<f64>().ok())
                .filter(|value| value.is_finite()),
            items
                .get(2)
                .and_then(Expr::atom)
                .and_then(|atom| atom.parse::<f64>().ok())
                .filter(|value| value.is_finite()),
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
    if let Some(why) = footprint_form_error(footprint) {
        return Err(why);
    }
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

/// Compare library-form models independently of item UUIDs and the board
/// instance's reference and value fields. The position of an explicitly
/// hidden property belongs to the instance, so it does not split the model.
/// Preserve property contents and all pad and artwork data.
pub(crate) fn same_library_model(first: &Expr, second: &Expr) -> bool {
    compare_library_models(first, second, InstancePresentation::Exact)
}

/// A shared ID can name instances with additional board documentation on
/// User.Drawings and different presentation for their fabrication reference
/// text. Keep those forms on the individual board instances.
pub(crate) fn same_library_model_with_instance_documentation(first: &Expr, second: &Expr) -> bool {
    compare_library_models(first, second, InstancePresentation::Documentation)
}

/// Compare instance text presentation while requiring identical drawings.
pub(crate) fn same_library_model_with_fab_text(first: &Expr, second: &Expr) -> bool {
    compare_library_models(first, second, InstancePresentation::FabText)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InstancePresentation {
    Exact,
    FabText,
    Documentation,
}

fn strip_instance_fab_text_presentation(item: &mut Expr) {
    if item.head() != Some("fp_text")
        || item.children().get(1).and_then(Expr::atom) != Some("user")
        || !matches!(form_atom(item, "layer", 1), Some("F.Fab" | "B.Fab"))
    {
        return;
    }
    let text = item.children().get(2).and_then(Expr::atom);
    let dynamic_reference = text == Some("${REFERENCE}");
    let legacy_placeholder = text == Some("REF**") && form_atom(item, "hide", 1) == Some("yes");
    let ignore_angle =
        (dynamic_reference || legacy_placeholder) && item.child("render_cache").is_none();
    let Expr::List(fields) = item else { return };
    if dynamic_reference {
        fields.retain(|field| {
            field.head() != Some("hide")
                || !matches!(
                    field.children().get(1).and_then(Expr::atom),
                    Some("yes" | "no")
                )
        });
    }
    if ignore_angle
        && let Some(Expr::List(at)) = fields.iter_mut().find(|field| field.head() == Some("at"))
        && at
            .get(3)
            .and_then(Expr::atom)
            .and_then(|angle| angle.parse::<f64>().ok())
            .is_some_and(f64::is_finite)
    {
        // KiCad can keep a fabrication reference upright as its footprint
        // turns. Only the display angle is instance-specific. The anchor,
        // layer, contents and font still belong to this model signature.
        at.remove(3);
    }
}

fn compare_library_models(first: &Expr, second: &Expr, presentation: InstancePresentation) -> bool {
    if footprint_form_error(first).is_some() || footprint_form_error(second).is_some() {
        return false;
    }
    fn strip_instance_ids(item: &mut Expr) {
        let Expr::List(parts) = item else { return };
        if parts.first().and_then(Expr::atom) == Some("at")
            && parts
                .get(3)
                .and_then(Expr::atom)
                .and_then(|a| a.parse::<f64>().ok())
                == Some(0.0)
        {
            parts.remove(3);
        }
        parts.retain(|part| !matches!(part.head(), Some("uuid" | "tstamp")));
        for part in parts {
            strip_instance_ids(part);
        }
    }
    let model_item = |item: &Expr| {
        let instance_only = match item.head() {
            Some("version" | "generator" | "generator_version") => true,
            Some("fp_text") => matches!(
                item.children().get(1).and_then(Expr::atom),
                Some("reference" | "value")
            ),
            Some("property") => matches!(
                item.children().get(1).and_then(Expr::atom),
                Some("Reference" | "Value" | "Sheetfile" | "Sheetname")
            ),
            Some("fp_arc" | "fp_circle" | "fp_curve" | "fp_line" | "fp_poly" | "fp_rect") => {
                presentation == InstancePresentation::Documentation
                    && matches!(
                        form_atom(item, "layer", 1),
                        Some("Dwgs.User" | "User.Drawings")
                    )
            }
            _ => false,
        };
        if instance_only {
            return None;
        }
        let mut item = item.clone();
        if presentation != InstancePresentation::Exact {
            strip_instance_fab_text_presentation(&mut item);
        }
        if item.head() == Some("property")
            && form_atom(&item, "hide", 1) == Some("yes")
            && let Expr::List(fields) = &mut item
        {
            fields.retain(|field| {
                let values = field.children();
                !(field.head() == Some("at")
                    && (3..=4).contains(&values.len())
                    && values[1..].iter().all(|value| {
                        value
                            .atom()
                            .and_then(|atom| atom.parse::<f64>().ok())
                            .is_some_and(f64::is_finite)
                    }))
            });
        }
        strip_instance_ids(&mut item);
        Some(encode(&item))
    };
    let first = &first.children()[2..];
    let second = &second.children()[2..];
    if first
        .iter()
        .chain(second)
        .any(|item| item.head() == Some("group"))
    {
        return groups::equivalent(first, second, model_item);
    }
    let signature = |items: &[Expr]| {
        let mut items: Vec<_> = items.iter().filter_map(model_item).collect();
        items.sort_unstable();
        items
    };
    signature(first) == signature(second)
}

/// A library-form footprint put at a pose on the front: `(at x y angle)`
/// set, pad and text angles made absolute, zones in board coordinates.
pub(crate) fn posed(footprint: &Expr, at: [f64; 3]) -> Result<Expr, String> {
    if let Some(why) = footprint_form_error(footprint) {
        return Err(why);
    }
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
    let (_, size, _) = crate::footprint_bodies::local_body(footprint).ok()?;
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

    #[test]
    fn donor_paths_reserve_renumbered_parts_and_references_resolve_copied_links() {
        let donors = [
            ("SW28", Some("/copied"), "1u"),
            ("SW29", Some("/copied"), "1.5u"),
            ("U5", Some("/renumbered"), "correct"),
            ("U2", None, "other"),
            ("T1", Some("/ambiguous"), "a"),
            ("T1", Some("/ambiguous"), "b"),
            ("X1", Some("/more-copies"), "x"),
            ("X2", Some("/more-copies"), "y"),
        ];
        let symbols = [
            (Some("/copied"), "SW29"),
            (Some("/stale"), "SW28"),
            (Some("/renumbered"), "U2"),
            (None, "U5"),
            (Some("/ambiguous"), "T1"),
            (Some("/more-copies"), "X2"),
            (Some("/more-copies"), "X3"),
        ];
        let expected = vec![
            Some("1.5u"),
            Some("1u"),
            Some("correct"),
            None,
            None,
            Some("y"),
            None,
        ];
        let mut board = BoardFootprints {
            file: PathBuf::new(),
            sha256: String::new(),
            footprints: donors
                .into_iter()
                .map(|(reference, path, id)| BoardFootprint {
                    reference: reference.into(),
                    path: path.map(str::to_owned),
                    id: id.into(),
                    uuid: None,
                    expr: Expr::List(Vec::new()),
                })
                .collect(),
            net_codes: BTreeMap::new(),
            copper_layers: 2,
        };
        for _ in 0..2 {
            let assigned: Vec<_> = board
                .assign(&symbols)
                .into_iter()
                .map(|donor| donor.map(|item| item.id.as_str()))
                .collect();
            assert_eq!(assigned, expected);
            board.footprints.reverse();
        }
    }

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

    #[test]
    fn library_equivalence_ignores_only_the_pose_of_hidden_properties() {
        let original = r#"(footprint "jack" (layer "F.Cu")
            (property "Supplier" "P-0123" (at 2 3 45) (layer "F.Fab")
                (hide yes) (effects (font (size 1 1))))
            (property "Label" "OUT" (at 3 2) (layer "F.SilkS")
                (effects (font (size 1 1))))
            (fp_rect (start -3 -3) (end 3 3) (layer "F.CrtYd") (width 0.05))
            (pad "1" thru_hole circle (at 0 0) (size 2 2) (drill 1) (layers "*.Cu" "*.Mask")))"#;
        let first = parse(original).unwrap();
        let hidden_moved = parse(&original.replace("(at 2 3 45)", "(at -15 8)")).unwrap();
        assert!(same_library_model(&first, &hidden_moved));
        assert_ne!(first, hidden_moved);
        for (before, after) in [
            ("P-0123", "P-0124"),
            ("(hide yes)", "(hide no)"),
            ("(at 3 2)", "(at 4 2)"),
            ("(end 3 3)", "(end 3 4)"),
            ("(drill 1)", "(drill 1.1)"),
        ] {
            let alternative = parse(&original.replace(before, after)).unwrap();
            assert!(!same_library_model(&first, &alternative), "{after}");
        }
    }

    #[test]
    fn instance_documentation_does_not_absorb_other_footprint_changes() {
        let original = r#"(footprint "module" (layer "F.Cu")
            (fp_rect (start -3 -3) (end 3 3) (layer "Eco2.User"))
            (fp_text user "${REFERENCE}" (at 0 0 0) (layer "F.Fab") (hide yes))
            (pad "1" thru_hole circle (at 0 0) (size 2 2) (drill 1) (layers "*.Cu" "*.Mask")))"#;
        let first = parse(original).unwrap();
        let variant = format!(
            r#"{} (fp_rect (start 0 0) (end 3 4) (layer "User.Drawings")))"#,
            original.strip_suffix(')').unwrap()
        );
        let variant = variant.replace("(hide yes)", "");
        let second = parse(&variant).unwrap();
        assert!(!same_library_model(&first, &second));
        assert!(same_library_model_with_instance_documentation(
            &first, &second
        ));
        for (before, after) in [
            ("(drill 1)", "(drill 1.1)"),
            ("(end 3 3)", "(end 4 3)"),
            ("(at 0 0 0)", "(at 1 0 0)"),
            ("User.Drawings", "F.SilkS"),
            ("User.Drawings", "F.Cu"),
            ("User.Drawings", "Edge.Cuts"),
        ] {
            let changed = parse(&variant.replace(before, after)).unwrap();
            assert!(
                !same_library_model_with_instance_documentation(&first, &changed),
                "{after}"
            );
        }
    }

    #[test]
    fn rotated_fab_reference_placeholder_keeps_the_model_and_its_own_angle() {
        let original = r#"(footprint "led" (layer "F.Cu")
            (fp_text user "REF**" (at 0 5 0) (layer "F.Fab") (hide yes)
                (effects (font (size 1 1))))
            (fp_line (start -1 -1) (end 1 -1) (layer "F.SilkS"))
            (pad "1" smd rect (at -1 0) (size 0.8 1) (layers "F.Cu" "F.Mask")))"#;
        let first = parse(original).unwrap();
        let variant = original.replace("(at 0 5 0)", "(at 0 5 180)");
        let second = parse(&variant).unwrap();
        assert!(!same_library_model(&first, &second));
        assert!(same_library_model_with_instance_documentation(
            &first, &second
        ));
        assert_eq!(
            form_atom(second.child("fp_text").unwrap(), "at", 3),
            Some("180")
        );

        for (before, after) in [
            ("REF**", "pin one"),
            ("(at 0 5 180)", "(at 0.1 5 180)"),
            ("(hide yes)", "(hide no)"),
            ("(size 1 1)", "(size 1 1.1)"),
            ("(end 1 -1)", "(end 1.2 -1)"),
            ("(size 0.8 1)", "(size 0.81 1)"),
        ] {
            let changed = parse(&variant.replace(before, after)).unwrap();
            assert!(
                !same_library_model_with_instance_documentation(&first, &changed),
                "{after}"
            );
        }
        for layer in ["F.SilkS", "F.Cu", "F.Mask"] {
            let on_layer = |text: &str| parse(&text.replace("F.Fab", layer)).unwrap();
            assert!(
                !same_library_model_with_instance_documentation(
                    &on_layer(original),
                    &on_layer(&variant)
                ),
                "{layer}"
            );
        }
    }
}
