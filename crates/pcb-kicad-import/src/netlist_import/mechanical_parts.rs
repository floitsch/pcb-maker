// Copyright (C) 2026 Florian Loitsch.

//! Board-only holes, cutouts and fiducials carried in from the mechanical PCB.

use super::*;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadImportedMechanicalObject {
    pub reference: String,
    pub uuid: String,
    pub at: [f64; 3],
    pub layer: String,
    pub npth_drills: usize,
    pub edge_cuts_items: usize,
    pub kept_local: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub explicit_selection: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub plated_drills: Vec<KiCadMechanicalPlatedDrill>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optical_fiducial: Option<KiCadMechanicalFiducial>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reidentified_metadata: Vec<KiCadMechanicalMetadataUuid>,
}

fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadMechanicalPlatedDrill {
    pub uuid: String,
    pub pad: String,
    pub local_xy_mm: [f64; 2],
    pub drill_mm: f64,
    pub land_mm: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KiCadMechanicalFiducial {
    pub pad_uuid: String,
    pub local_xy_mm: [f64; 2],
    pub copper_diameter_mm: f64,
    pub mask_diameter_mm: f64,
    pub mask_margin_mm: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pad_clearance_mm: Option<f64>,
}

#[derive(Clone, Copy)]
pub(super) struct MechanicalBinding<'a> {
    /// Exact escaped name used by the imported schematic pads.
    pub target: &'a str,
    pub donor: &'a Expr,
    pub local: Option<&'a Expr>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadMechanicalMetadataUuid {
    pub field: String,
    pub donor_uuid: String,
    pub output_uuid: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadExcludedMechanicalObject {
    pub reference: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    pub reason: String,
}

pub(super) fn edge_cuts(item: &Expr) -> bool {
    item.head().is_some_and(|name| name.starts_with("fp_"))
        && form_atom(item, "layer", 1) == Some("Edge.Cuts")
}

pub(super) fn has_physical_geometry(part: &Expr) -> bool {
    part.children()
        .iter()
        .any(|item| edge_cuts(item) || item.head() == Some("pad") && item.child("drill").is_some())
}

fn unnetted(pad: &Expr) -> bool {
    !pad.children()
        .iter()
        .filter(|form| form.head() == Some("net"))
        .flat_map(|form| form.children().iter().skip(1))
        .any(|value| !matches!(value, Expr::Atom(raw) if raw == "0" || raw == "\"\""))
}

pub(super) fn explicitly_board_only(part: &Expr) -> bool {
    part.child("attr").is_some_and(|attr| {
        attr.children()
            .iter()
            .skip(1)
            .any(|field| field.atom() == Some("board_only"))
    })
}

fn optical_fiducial(part: &Expr, pad: &Expr) -> Option<KiCadMechanicalFiducial> {
    let tagged = form_atom(part, "tags", 1)?
        .split_ascii_whitespace()
        .any(|tag| tag.eq_ignore_ascii_case("fiducial"));
    if !explicitly_board_only(part)
        || !tagged
        || !pad.children().get(1)?.atom()?.is_empty()
        || pad.children().get(3)?.atom()? != "circle"
    {
        return None;
    }
    let mask_or_paste = |layer: &str| layer.ends_with(".Mask") || layer.ends_with(".Paste");
    if part.children().iter().any(|item| {
        !std::ptr::eq(item, pad)
            && (item.head() == Some("pad")
                || edge_cuts(item)
                || form_atom(item, "layer", 1).is_some_and(mask_or_paste)
                || item
                    .child("layers")
                    .into_iter()
                    .flat_map(|form| form.children().iter().skip(1))
                    .filter_map(Expr::atom)
                    .any(mask_or_paste))
    }) {
        return None;
    }
    let (copper, mask) = match form_atom(part, "layer", 1)? {
        "F.Cu" => ("F.Cu", "F.Mask"),
        "B.Cu" => ("B.Cu", "B.Mask"),
        _ => return None,
    };
    let [_, a, b] = pad.child("layers")?.children() else {
        return None;
    };
    let [a, b] = [a.atom()?, b.atom()?];
    if (a, b) != (copper, mask) && (b, a) != (copper, mask) {
        return None;
    }
    let mut fields = BTreeSet::new();
    if !pad.children().iter().skip(4).all(|field| {
        field.head().is_some_and(|name| {
            fields.insert(name) && match name {
                "at" => matches!(field.children().len(), 3 | 4),
                "size" | "layers" => field.children().len() == 3,
                "uuid" | "tstamp" | "clearance" | "solder_mask_margin" => field.children().len() == 2,
                "net" => matches!(field.children(),
                    [_, Expr::Atom(raw)] if raw == "0" || raw == "\"\"")
                    || matches!(field.children(), [_, Expr::Atom(code), Expr::Atom(raw)] if code == "0" && raw == "\"\""),
                _ => false,
            }
        })
    }) || fields.contains("uuid") && fields.contains("tstamp") {
        return None;
    }
    let [diameter, y] = form_xy(pad, "size").ok()?;
    let margin = form_f64(pad, "solder_mask_margin", 1).ok()?;
    let mask_diameter_mm = diameter + 2.0 * margin;
    if !(diameter > 0.0 && diameter == y && margin > 0.0 && mask_diameter_mm.is_finite()) {
        return None;
    }
    let pad_clearance_mm = match pad.child("clearance") {
        Some(_) => Some(form_f64(pad, "clearance", 1).ok().filter(|&mm| mm >= 0.0)?),
        None => None,
    };
    let [x, y, _angle] = form_at(pad).ok()?;
    Some(KiCadMechanicalFiducial {
        pad_uuid: footprint_uuid(pad)?,
        local_xy_mm: [x, y],
        copper_diameter_mm: diameter,
        mask_diameter_mm,
        mask_margin_mm: margin,
        pad_clearance_mm,
    })
}

fn unnetted_npth(pad: &Expr) -> bool {
    if pad.children().get(2).and_then(Expr::atom) != Some("np_thru_hole")
        || pad.child("padstack").is_some()
        || !unnetted(pad)
    {
        return false;
    }
    let Some(drill) = pad.child("drill") else {
        return false;
    };
    let raw = &drill.children()[1..];
    let oval = raw.first().and_then(Expr::atom) == Some("oval");
    let dimensions: Vec<_> = raw
        .iter()
        .skip(usize::from(oval))
        .filter_map(Expr::atom)
        .map(str::parse::<f64>)
        .collect::<Result<_, _>>()
        .unwrap_or_default();
    let drill_size = match (oval, dimensions.as_slice()) {
        (false, &[d]) => [d, d],
        (false, &[x, y]) if x == y => [x, y],
        (true, &[x, y]) => [x, y],
        _ => return false,
    };
    let expected_shape = if oval { "oval" } else { "circle" };
    // No separate copper land. A displaced hole could leave a land even
    // when both size tuples agree.
    pad.children().get(3).and_then(Expr::atom) == Some(expected_shape)
        && drill_size
            .iter()
            .all(|size| size.is_finite() && *size > 0.0)
        && form_xy(pad, "size") == Ok(drill_size)
        && drill
            .children()
            .iter()
            .filter(|field| matches!(field, Expr::List(_)))
            .all(|field| {
                field.head() == Some("offset") && form_xy(drill, "offset") == Ok([0.0, 0.0])
            })
}

fn teardrop_settings(form: &Expr) -> bool {
    let flag = |name| {
        matches!(
            name,
            "enabled" | "allow_two_segments" | "prefer_zone_connections"
        )
    };
    let mut seen = BTreeSet::new();
    form.children().iter().skip(1).all(|field| {
        if let Some(name) = field.atom() {
            return flag(name) && seen.insert(name);
        }
        let [name, value] = field.children() else {
            return false;
        };
        let (Some(name), Some(value)) = (name.atom(), value.atom()) else {
            return false;
        };
        seen.insert(name)
            && if flag(name) {
                matches!(value, "yes" | "no")
            } else if name == "curve_points" {
                value.parse::<u32>().is_ok()
            } else {
                matches!(
                    name,
                    "best_length_ratio"
                        | "max_length"
                        | "best_width_ratio"
                        | "max_width"
                        | "filter_ratio"
                ) && value
                    .parse::<f64>()
                    .is_ok_and(|value| value.is_finite() && value >= 0.0)
            }
    })
}

fn quoted_field(field: &Expr) -> Option<&str> {
    match field {
        Expr::Atom(raw) if raw.starts_with('"') && raw.ends_with('"') => field.atom(),
        _ => None,
    }
}

fn round_plated_drill(pad: &Expr, bound: bool) -> Option<KiCadMechanicalPlatedDrill> {
    let fields = pad.children();
    if fields.get(2)?.atom()? != "thru_hole" || fields.get(3)?.atom()? != "circle" {
        return None;
    }
    // Admit only concentric circular annuli on the full copper stack.
    // In particular, a custom stack, slot, offset or layer removal can
    // create a different obstacle from the one described by size/drill.
    let mut seen = BTreeSet::new();
    for field in &fields[4..] {
        let name = field.head()?;
        if !seen.insert(name)
            || !match name {
                "at" | "size" | "drill" | "layers" | "uuid" | "tstamp" | "net" => true,
                "remove_unused_layers" => {
                    field.children().get(1).and_then(Expr::atom) == Some("no")
                }
                "teardrops" if bound => teardrop_settings(field),
                "pinfunction" | "pintype" if bound => {
                    let [_, metadata] = field.children() else {
                        return None;
                    };
                    quoted_field(metadata)
                        .is_some_and(|value| name == "pinfunction" || value == "passive")
                }
                _ => false,
            }
        {
            return None;
        }
    }
    let [_, diameter] = pad.child("drill")?.children() else {
        return None;
    };
    let drill_mm: f64 = diameter.atom()?.parse().ok()?;
    let [land_mm, height] = form_xy(pad, "size").ok()?;
    if !drill_mm.is_finite()
        || drill_mm <= 0.0
        || !land_mm.is_finite()
        || land_mm <= drill_mm
        || land_mm != height
    {
        return None;
    }
    let mut layers = BTreeSet::new();
    for layer in &pad.child("layers")?.children()[1..] {
        let layer = layer.atom()?;
        if !matches!(layer, "*.Cu" | "*.Mask" | "F.Mask" | "B.Mask") || !layers.insert(layer) {
            return None;
        }
    }
    if !layers.contains("*.Cu") {
        return None;
    }
    let [x, y, angle] = form_at(pad).ok()?;
    if ![x, y, angle].iter().all(|value| value.is_finite()) {
        return None;
    }
    Some(KiCadMechanicalPlatedDrill {
        uuid: footprint_uuid(pad)?,
        pad: fields.get(1)?.atom()?.into(),
        local_xy_mm: [x, y],
        drill_mm,
        land_mm,
        net: None,
    })
}

fn pad_binding<'a>(pad: &'a Expr, board: &'a Expr) -> Result<Option<&'a str>, &'static str> {
    let mut nets = pad
        .children()
        .iter()
        .filter(|field| field.head() == Some("net"));
    let Some(form) = nets.next() else {
        return Ok(None);
    };
    let invalid = "a plated pad has an ambiguous or invalid donor net assignment";
    if nets.next().is_some() {
        return Err(invalid);
    }
    let numeric = |atom: &'a Expr| match atom {
        Expr::Atom(raw) if raw.bytes().all(|byte| byte.is_ascii_digit()) => raw.parse::<u32>().ok(),
        _ => None,
    };
    let (code, written) = match form.children() {
        [_, field] => {
            if let Some(name) = quoted_field(field) {
                return Ok((!name.is_empty()).then_some(name));
            }
            (numeric(field).ok_or(invalid)?, None)
        }
        [_, code, written] => (
            numeric(code).ok_or(invalid)?,
            Some(quoted_field(written).ok_or(invalid)?),
        ),
        _ => return Err(invalid),
    };
    if code == 0 {
        return written
            .is_none_or(str::is_empty)
            .then_some(None)
            .ok_or(invalid);
    }
    let table = || {
        board
            .children()
            .iter()
            .filter(|net| net.head() == Some("net"))
    };
    let mut declarations =
        table().filter(|item| item.children().get(1).and_then(numeric) == Some(code));
    let [_, _, field] = declarations.next().ok_or(invalid)?.children() else {
        return Err(invalid);
    };
    let name = quoted_field(field)
        .filter(|name| !name.is_empty())
        .ok_or(invalid)?;
    if declarations.next().is_some()
        || written.is_some_and(|written| unescape(written) != unescape(name))
        || table()
            .filter_map(|item| item.children().get(2).and_then(quoted_field))
            .filter(|other| unescape(other) == unescape(name))
            .count()
            != 1
    {
        return Err(invalid);
    }
    Ok(Some(name))
}

struct Geometry {
    drills: usize,
    edges: usize,
    plated_drills: Vec<KiCadMechanicalPlatedDrill>,
    fiducial: Option<KiCadMechanicalFiducial>,
}

fn physical_geometry(
    part: &Expr,
    explicit: bool,
    binding: Option<(&str, &Expr)>,
) -> Result<Geometry, &'static str> {
    if part.child("path").is_some() {
        return Err("a donor schematic path is still present");
    }
    if binding.is_some() && !explicitly_board_only(part) {
        return Err("a net-bound drill requires a footprint explicitly marked board_only");
    }
    let mut drills = 0;
    let mut edges = 0;
    let mut plated_drills = Vec::new();
    let mut fiducial = None;
    for item in part.children() {
        match item.head() {
            Some("pad") if unnetted_npth(item) => drills += 1,
            Some("pad")
                if explicit && item.children().get(2).and_then(Expr::atom) == Some("smd") =>
            {
                if binding.is_some() {
                    return Err("a board-only optical fiducial cannot use a schematic net binding");
                }
                fiducial = Some(optical_fiducial(part, item).ok_or(
                    "a surface pad requires a tagged board-only fiducial with one unnumbered netless circular copper land and an explicit same-side mask opening",
                )?);
            }
            Some("pad") if explicit => {
                if binding.is_none() && !unnetted(item) {
                    return Err(
                        "a pad is assigned to an electrical net; authorize it with UUID=NET",
                    );
                }
                let mut drill = round_plated_drill(item, binding.is_some())
                    .ok_or("a pad is not a supported concentric circular plated drill")?;
                if let Some((target, board)) = binding
                    && let Some(raw) = pad_binding(item, board)?
                {
                    if unescape(raw) != unescape(target) {
                        return Err(
                            "a plated pad names a different net from the selected schematic net",
                        );
                    }
                    drill.net = Some(unescape(target).into_owned());
                }
                plated_drills.push(drill);
            }
            Some("pad") => {
                return Err("a pad has plating, an assigned net, or separate copper geometry");
            }
            Some("zone" | "segment" | "via" | "arc") => {
                return Err("the footprint contains added copper or a zone");
            }
            Some(_)
                if form_atom(item, "layer", 1).is_some_and(|layer| layer.ends_with(".Cu"))
                    || explicit
                        && item.child("layers").is_some_and(|layers| {
                            layers
                                .children()
                                .iter()
                                .skip(1)
                                .any(|layer| layer.atom().is_some_and(|name| name.ends_with(".Cu")))
                        }) =>
            {
                return Err("the footprint contains copper artwork");
            }
            Some("fp_arc" | "fp_circle" | "fp_curve" | "fp_line" | "fp_poly" | "fp_rect")
                if edge_cuts(item) =>
            {
                edges += 1;
            }
            Some(_) if edge_cuts(item) => return Err("unsupported footprint Edge.Cuts item"),
            _ => {}
        }
    }
    if drills + edges == 0 && plated_drills.is_empty() && fiducial.is_none() {
        return Err("no transferable NPTH drills or footprint Edge.Cuts");
    }
    if binding.is_some() && plated_drills.iter().all(|drill| drill.net.is_none()) {
        return Err("no donor plated pad carries the selected schematic net");
    }
    Ok(Geometry {
        drills,
        edges,
        plated_drills,
        fiducial,
    })
}

pub(super) fn collect_ids(item: &Expr, ids: &mut BTreeSet<String>) -> bool {
    let mut unique = footprint_uuid(item).is_none_or(|uuid| ids.insert(uuid.to_ascii_lowercase()));
    for child in item.children() {
        unique &= collect_ids(child, ids);
    }
    unique
}

pub(super) fn preserve(
    donor: &Expr,
    local: Option<&Expr>,
    used: &mut BTreeSet<String>,
    explicit: bool,
    binding: Option<MechanicalBinding<'_>>,
) -> Result<(Expr, KiCadImportedMechanicalObject), KiCadExcludedMechanicalObject> {
    let reference = footprint_reference(donor).unwrap_or_default();
    let uuid = footprint_uuid(donor);
    let excluded = |reason: &str| KiCadExcludedMechanicalObject {
        reference: reference.clone(),
        uuid: uuid.clone(),
        reason: reason.into(),
    };
    let (Some(id), Some(kind @ ("footprint" | "module"))) = (&uuid, donor.head()) else {
        return Err(excluded("the donor footprint has no stable UUID"));
    };
    let donor_geometry = physical_geometry(
        donor,
        explicit,
        binding.map(|binding| (binding.target, binding.donor)),
    )
    .map_err(excluded)?;
    let source = local.unwrap_or(donor);
    let geometry = if local.is_some() {
        let local_geometry = physical_geometry(
            source,
            explicit,
            binding.map(|binding| {
                (
                    binding.target,
                    binding.local.expect("local footprint has a source board"),
                )
            }),
        )
        .map_err(excluded)?;
        if binding.is_some() {
            let assignments = |geometry: &Geometry| {
                geometry
                    .plated_drills
                    .iter()
                    .map(|drill| (drill.uuid.to_ascii_lowercase(), drill.net.clone()))
                    .collect::<BTreeMap<_, _>>()
            };
            if footprint_reference(source).as_deref() != Some(reference.as_str())
                || assignments(&local_geometry) != assignments(&donor_geometry)
            {
                return Err(excluded(
                    "the local footprint's reference or plated pad net assignments differ from the donor",
                ));
            }
        }
        if (donor_geometry.fiducial.is_some() || local_geometry.fiducial.is_some())
            && (footprint_reference(source).as_deref() != Some(reference.as_str())
                || local_geometry.fiducial != donor_geometry.fiducial)
        {
            return Err(excluded(
                "the local fiducial's reference, pad identity or optical geometry differs from the donor",
            ));
        }
        local_geometry
    } else {
        donor_geometry
    };
    let at = form_at(source)
        .ok()
        .filter(|at| at.iter().all(|number| number.is_finite()))
        .ok_or_else(|| excluded("the footprint pose is invalid"))?;
    let layer = form_atom(source, "layer", 1)
        .filter(|value| matches!(*value, "F.Cu" | "B.Cu"))
        .ok_or_else(|| excluded("the mounting side is invalid"))?;
    let mut part = source.clone();
    let Expr::List(fields) = &mut part else {
        return Err(excluded("the footprint form is invalid"));
    };
    if kind == "module" {
        fields[0] = Expr::Atom("footprint".into());
    }
    if let Some(binding) = binding {
        for pad in fields.iter_mut().filter(|item| item.head() == Some("pad")) {
            let uuid = footprint_uuid(pad);
            let Expr::List(fields) = pad else { continue };
            fields.retain(|field| field.head() != Some("net"));
            if geometry
                .plated_drills
                .iter()
                .any(|drill| drill.net.is_some() && uuid.as_deref() == Some(drill.uuid.as_str()))
            {
                fields.push(Expr::List(vec![
                    Expr::Atom("net".into()),
                    Expr::Atom(requote(binding.target)),
                ]));
            }
        }
    }
    super::mechanical::set_native_lock(&mut part, explicit || footprint_locked(source))
        .map_err(|error| excluded(&error))?;
    mark_board_only(&mut part);
    let reidentified_metadata = reserve_instance_ids(&mut part, id, used).map_err(excluded)?;
    Ok((
        part,
        KiCadImportedMechanicalObject {
            reference: footprint_reference(source).unwrap_or(reference),
            uuid: id.clone(),
            at,
            layer: layer.into(),
            npth_drills: geometry.drills,
            edge_cuts_items: geometry.edges,
            kept_local: local.is_some(),
            explicit_selection: explicit,
            plated_drills: geometry.plated_drills,
            optical_fiducial: geometry.fiducial,
            reidentified_metadata,
        },
    ))
}

fn reserve_instance_ids(
    part: &mut Expr,
    uuid: &str,
    used: &mut BTreeSet<String>,
) -> Result<Vec<KiCadMechanicalMetadataUuid>, &'static str> {
    let mut ids = BTreeSet::from([uuid.to_ascii_lowercase()]);
    let collision = "a footprint or physical child UUID already belongs to another board item";
    if !used.is_disjoint(&ids) {
        return Err(collision);
    }
    let Expr::List(fields) = part else {
        return Err(collision);
    };
    let has_groups = fields.iter().any(|field| field.head() == Some("group"));
    let mut changed = Vec::new();
    for (index, child) in fields.iter_mut().enumerate().skip(2) {
        if !has_groups
            && let Some((name, old)) = repeated_hidden_metadata(child, &ids, used)
            && let Expr::List(values) = child
            && let Some(value) = values
                .iter_mut()
                .find(|item| matches!(item.head(), Some("uuid" | "tstamp")))
        {
            let new = deterministic_uuid_for(&format!(
                "pcb-maker:mechanical-field:{uuid}:{index}:{name}:{old}"
            ));
            *value = Expr::List(vec![Expr::Atom("uuid".into()), Expr::Atom(requote(&new))]);
            changed.push(KiCadMechanicalMetadataUuid {
                field: name,
                donor_uuid: old,
                output_uuid: new,
            });
        }
        if !collect_ids(child, &mut ids) || !used.is_disjoint(&ids) {
            return Err(collision);
        }
    }
    used.extend(ids);
    Ok(changed)
}

fn repeated_hidden_metadata(
    field: &Expr,
    ids: &BTreeSet<String>,
    used: &BTreeSet<String>,
) -> Option<(String, String)> {
    let name = field.children().get(1)?.atom()?;
    if field.head() != Some("property")
        || !matches!(name, "Footprint" | "Datasheet" | "Description")
        || !matches!(form_atom(field, "layer", 1), Some("F.Fab" | "B.Fab"))
        || form_atom(field, "hide", 1) != Some("yes")
    {
        return None;
    }
    let uuid = footprint_uuid(field)?;
    let lower = uuid.to_ascii_lowercase();
    (ids.contains(&lower) || used.contains(&lower)).then(|| (name.into(), uuid))
}

fn mark_board_only(part: &mut Expr) {
    let Expr::List(items) = part else { return };
    let index = items.iter().position(|item| item.head() == Some("attr"));
    let flags = if let Some(index) = index {
        &mut items[index]
    } else {
        items.push(Expr::List(vec![Expr::Atom("attr".into())]));
        items.last_mut().expect("just added attributes")
    };
    let Expr::List(flags) = flags else { return };
    if !flags.iter().any(|field| field.atom() == Some("board_only")) {
        flags.push(Expr::Atom("board_only".into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OVAL: &str = r#"(footprint "Slot" locked (layer "B.Cu") (at 7 8 37)
        (tstamp 344c8b57-74f4-4b9b-9dd0-f7d143c63c91)
        (fp_text reference "REF**" (at 0 0) (layer "B.SilkS"))
        (pad "" np_thru_hole oval locked (at 1 2 67) (size 2 3)
            (drill oval 2 3) (layers "*.Cu" "*.Mask")
            (tstamp 48ae3aac-5f7b-469c-b9a0-8e27b4775b42)))"#;

    const FIDUCIAL: &str = r#"(footprint "Fiducial:Fiducial_0.75mm_Mask1.5mm"
        (layer "B.Cu") (at 60.3 137.9 180) (tags "registration fiducial")
        (uuid "1de12049-f612-470a-9ffd-220ebf48c112") (attr smd board_only exclude_from_bom)
        (property "Reference" "FID2")
        (pad "" smd circle (at 0.1 0.2 180) (size 0.75 0.75) (layers "B.Cu" "B.Mask")
            (solder_mask_margin 0.375) (clearance 0.475)
            (uuid "69cb0483-cf7e-4a38-a6da-fe158f3aa17b")))"#;

    #[test]
    fn explicit_optical_fiducials_keep_the_same_face_land_mask_and_pad_identity() {
        let donor = parse(FIDUCIAL).unwrap();
        assert!(!has_physical_geometry(&donor));
        assert!(preserve(&donor, None, &mut BTreeSet::new(), false, None).is_err());
        let (saved, receipt) = preserve(&donor, None, &mut BTreeSet::new(), true, None).unwrap();
        assert_eq!(receipt.layer, "B.Cu");
        assert_eq!(receipt.at, [60.3, 137.9, 180.0]);
        assert!(footprint_locked(&saved));
        assert_eq!(saved.child("pad"), donor.child("pad"));
        let mark = receipt.optical_fiducial.unwrap();
        assert_eq!(mark.pad_uuid, "69cb0483-cf7e-4a38-a6da-fe158f3aa17b");
        assert_eq!(mark.local_xy_mm, [0.1, 0.2]);
        assert_eq!(
            (mark.copper_diameter_mm, mark.mask_diameter_mm),
            (0.75, 1.5)
        );
        assert_eq!(mark.pad_clearance_mm, Some(0.475));
        let (again, report) =
            preserve(&donor, Some(&saved), &mut BTreeSet::new(), true, None).unwrap();
        assert!(report.kept_local);
        assert_eq!(again, saved);
    }

    #[test]
    fn optical_selection_refuses_electrical_ambiguous_or_changed_pad_geometry() {
        let donor = parse(FIDUCIAL).unwrap();
        for changed in [
            FIDUCIAL.replace(" board_only", ""),
            FIDUCIAL.replace("registration fiducial", "registration"),
            FIDUCIAL.replace("(pad \"\"", "(pad \"1\""),
            FIDUCIAL.replace("(clearance 0.475)", "(net \"GND\")"),
            FIDUCIAL.replace("\"B.Mask\"", "\"B.Mask\" \"B.Paste\""),
            FIDUCIAL.replace("\"B.Mask\"", "\"F.Mask\""),
            FIDUCIAL.replace("(solder_mask_margin 0.375)", "(solder_mask_margin 0)"),
            FIDUCIAL.replace(
                "(property \"Reference\"",
                "(path \"/symbol\") (property \"Reference\"",
            ),
        ] {
            let changed = parse(&changed).unwrap();
            assert!(preserve(&changed, None, &mut BTreeSet::new(), true, None).is_err());
        }
        let local =
            parse(&FIDUCIAL.replace("(solder_mask_margin 0.375)", "(solder_mask_margin 0.5)"))
                .unwrap();
        assert!(preserve(&donor, Some(&local), &mut BTreeSet::new(), true, None).is_err());
        let binding = MechanicalBinding {
            target: "GND",
            donor: &donor,
            local: None,
        };
        assert!(preserve(&donor, None, &mut BTreeSet::new(), true, Some(binding)).is_err());
    }

    #[test]
    fn a_board_only_slot_preserves_pose_geometry_and_native_identity() {
        let donor = parse(OVAL).unwrap();
        let mut ids = BTreeSet::new();
        let (slot, report) = preserve(&donor, None, &mut ids, false, None).unwrap();
        assert_eq!(report.npth_drills, 1);
        assert_eq!(report.reference, "REF**");
        assert_eq!(report.at, [7.0, 8.0, 37.0]);
        assert_eq!(report.layer, "B.Cu");
        assert_eq!(report.uuid, "344c8b57-74f4-4b9b-9dd0-f7d143c63c91");
        assert!(footprint_locked(&slot));
        assert_eq!(slot.child("pad"), donor.child("pad"));
        assert_eq!(slot.child("at"), donor.child("at"));
        assert_eq!(form_atom(&slot, "attr", 1), Some("board_only"));
        assert_eq!(slot.child("attr").unwrap().children().len(), 2);
        assert!(preserve(&donor, None, &mut ids, false, None).is_err());

        let moved = parse(&OVAL.replace("(at 7 8 37)", "(at 7 9 42)")).unwrap();
        let (_, report) =
            preserve(&donor, Some(&moved), &mut BTreeSet::new(), false, None).unwrap();
        assert!(report.kept_local);
        assert_eq!(report.at, [7.0, 9.0, 42.0]);
    }

    #[test]
    fn plated_and_electrical_features_are_reported_as_omitted() {
        let donor = parse(OVAL).unwrap();
        for changed in [
            OVAL.replace(" np_thru_hole ", " thru_hole "),
            OVAL.replace("(size 2 3)", "(size 2.2 3.2)"),
            OVAL.replace("(drill oval 2 3)", "(drill oval 2 3) (net 1 \"GND\")"),
            OVAL.replace("(drill oval 2 3)", "(drill oval 2 3 (offset 0.1 0))"),
            OVAL.replace("(drill oval 2 3)", "(drill oval 2 3) (net \"0\")"),
        ] {
            let changed = parse(&changed).unwrap();
            assert!(has_physical_geometry(&changed));
            assert!(preserve(&changed, None, &mut BTreeSet::new(), false, None).is_err());
            assert!(preserve(&donor, Some(&changed), &mut BTreeSet::new(), false, None).is_err());
        }
    }

    #[test]
    fn foot_owned_edges_do_not_require_drills_or_unique_display_references() {
        let text = r#"(footprint "Edge" (locked yes) (layer "F.Cu") (at 20 25 11)
            (uuid "44e1a9c9-f601-4149-a711-cc15170a594b")
            (property "Reference" "REF**")
            (fp_circle (center 1 2) (end 2 2) (layer "Edge.Cuts")
                (stroke (width 0.05) (type solid)) (fill none)))"#;
        let edge = parse(text).unwrap();
        let mut ids = BTreeSet::new();
        let donor = parse(OVAL).unwrap();
        preserve(&donor, None, &mut ids, false, None).unwrap();
        let (saved, receipt) = preserve(&edge, None, &mut ids, false, None).unwrap();
        assert_eq!((receipt.npth_drills, receipt.edge_cuts_items), (0, 1));
        assert_eq!(receipt.reference, "REF**");
        assert_eq!(saved.child("fp_circle"), edge.child("fp_circle"));
        assert_ne!(footprint_uuid(&saved), footprint_uuid(&donor));
    }

    #[test]
    fn duplicated_hidden_metadata_does_not_duplicate_native_item_ids() {
        let field = r#"(property "Datasheet" "" (layer "B.Fab") (hide yes)
            (uuid "30dc548d-811b-449a-bed9-caadb2d51ac4"))"#;
        let base = format!("{} {field})", &OVAL[..OVAL.len() - 1]);
        let first = parse(&base).unwrap();
        let second = parse(
            &base
                .replace(
                    "344c8b57-74f4-4b9b-9dd0-f7d143c63c91",
                    "ff816428-a684-474f-aa18-e82d26e79af6",
                )
                .replace(
                    "48ae3aac-5f7b-469c-b9a0-8e27b4775b42",
                    "23dfe1a8-a7e7-45ce-b75f-5254bab9bf26",
                ),
        )
        .unwrap();
        let mut used = BTreeSet::new();
        let (_, original) = preserve(&first, None, &mut used, false, None).unwrap();
        assert!(original.reidentified_metadata.is_empty());
        let (saved, changed) = preserve(&second, None, &mut used, false, None).unwrap();
        assert_eq!(saved.child("pad"), second.child("pad"));
        assert_eq!(footprint_uuid(&saved), footprint_uuid(&second));
        let [field] = changed.reidentified_metadata.as_slice() else {
            panic!()
        };
        assert_eq!(field.field, "Datasheet");
        assert_ne!(field.output_uuid, field.donor_uuid);
        assert_eq!(
            footprint_uuid(saved.child("property").unwrap()).as_deref(),
            Some(field.output_uuid.as_str())
        );

        let mut reused = BTreeSet::new();
        preserve(&first, None, &mut reused, false, None).unwrap();
        let collision = parse(&base.replace(
            "344c8b57-74f4-4b9b-9dd0-f7d143c63c91",
            "ff816428-a684-474f-aa18-e82d26e79af6",
        ))
        .unwrap();
        assert!(preserve(&collision, None, &mut reused, false, None).is_err());
    }
}
