// Copyright (C) 2026 Florian Loitsch.

//! Board-level mechanical inputs kept apart from footprint and stack donors.

use super::mechanical_parts::{
    MechanicalBinding, collect_ids, edge_cuts as footprint_edge, explicitly_board_only,
    has_physical_geometry, preserve,
};
use super::mechanical_perimeter::{
    detached_outlines, perimeter_owners, same_owner, same_pad_sites,
};
use super::*;
use pcb_kicad_file::{
    board_stats::is_rule_area,
    kicad_nets::{footprint_locked, glob_matches},
    outline,
};
use pcb_kicad_place::{
    KiCadBoardPlacerConfig,
    placement_constraints::{KiCadConstraintsSource, resolve_constraints},
};
use pcb_kicad_route::board_router::LayerTable;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize)]
pub struct KiCadImportedMechanics {
    pub source: PathBuf,
    pub source_sha256: String,
    pub geometry_source: PathBuf,
    pub geometry_source_sha256: String,
    pub preserved_existing_geometry: bool,
    pub edge_cuts_items: usize,
    pub rule_areas: usize,
    pub constraints_kept: bool,
    pub locks: Vec<KiCadImportedMechanicalLock>,
    /// Exact locked donor instances installed through constraints.add.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub matched_additions: Vec<KiCadImportedMechanicalAddition>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub retained_board_objects: Vec<KiCadImportedMechanicalObject>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub excluded_board_objects: Vec<KiCadExcludedMechanicalObject>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unmatched_locks: Vec<KiCadUnmatchedMechanicalLock>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflicting_locks: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadImportedMechanicalLock {
    pub reference: String,
    pub at: [f64; 3],
    pub layer: String,
    pub applied: bool,
    /// Footprint-local shapes kept with a required perimeter pose.
    #[serde(skip_serializing_if = "is_zero")]
    pub perimeter_edge_cuts: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadImportedMechanicalAddition {
    pub reference: String,
    pub uuid: String,
    pub at: [f64; 3],
    pub layer: String,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadUnmatchedMechanicalLock {
    pub reference: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
}

pub(super) struct Mechanical {
    donor: Expr,
    previous: Option<Expr>,
    objects: Vec<Expr>,
    retained_placer: Option<KiCadBoardPlacerConfig>,
    selected_footprints: BTreeMap<String, Option<String>>,
    pub copper_layers: usize,
    pub report: KiCadImportedMechanics,
}

fn read(path: &Path) -> Result<(Expr, String), String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let pcb = parse(std::str::from_utf8(&bytes).map_err(|error| error.to_string())?)?;
    if pcb.head() != Some("kicad_pcb") {
        return Err(format!("{} is not a KiCad PCB", path.display()));
    }
    Ok((pcb, format!("{:x}", Sha256::digest(bytes))))
}

pub(super) fn edge(item: &Expr) -> bool {
    item.head().is_some_and(|name| name.starts_with("gr_"))
        && form_atom(item, "layer", 1) == Some("Edge.Cuts")
}

fn has_outline_geometry(pcb: &Expr) -> bool {
    pcb.children().iter().any(edge)
        || board_parts(pcb).any(|part| part.children().iter().any(footprint_edge))
}

pub(super) fn board_parts(pcb: &Expr) -> impl Iterator<Item = &Expr> {
    pcb.children()
        .iter()
        .filter(|item| matches!(item.head(), Some("footprint" | "module")))
}

fn mechanical_error(why: impl std::fmt::Display) -> String {
    exit::input_error(format!("--mechanical-from: {why}"))
}

fn lock_conflict(
    conflicts: &mut Vec<String>,
    reference: &str,
    perimeter_owner: bool,
    reason: &str,
) -> Result<(), String> {
    if perimeter_owner {
        return Err(mechanical_error(format!(
            "perimeter owner {reference}: {reason}"
        )));
    }
    conflicts.push(format!("{reference}: {reason}"));
    Ok(())
}

fn retained_placer(directory: &Path) -> Result<Option<KiCadBoardPlacerConfig>, String> {
    #[derive(serde::Deserialize)]
    struct Layout {
        #[serde(default)]
        placer: KiCadBoardPlacerConfig,
    }
    let layout = directory.join("layout.json");
    let starter = PathBuf::from("constraints.json");
    let config = if layout.is_file() {
        let text = fs::read_to_string(&layout).map_err(mechanical_error)?;
        serde_json::from_str::<Layout>(&text)
            .map_err(|error| mechanical_error(format!("{}: {error}", layout.display())))?
            .placer
    } else if directory.join(&starter).is_file() {
        KiCadBoardPlacerConfig {
            constraints: Some(KiCadConstraintsSource::File(starter.clone())),
            ..Default::default()
        }
    } else {
        return Ok(None);
    };
    if matches!(&config.constraints, Some(KiCadConstraintsSource::File(path)) if *path == starter)
        && !directory.join(starter).is_file()
    {
        // The default brief will be created during this import. Honor any
        // explicit unlocks from the existing layout first.
        return Ok(Some(config));
    }
    resolve_constraints(&config, directory)
        .map(Some)
        .map_err(mechanical_error)
}

fn retained_pose(
    config: &KiCadBoardPlacerConfig,
    reference: &str,
    footprint: &Expr,
) -> Option<([f64; 3], &'static str)> {
    let Some(KiCadConstraintsSource::Inline(brief)) = &config.constraints else {
        return None;
    };
    let mut entries = brief.place.iter().filter(|entry| entry.part == reference);
    let entry = entries.next()?;
    if entries.next().is_some()
        || entry.at.is_some()
        || entry.origin.as_deref() != Some("absolute")
        || entry
            .uuid
            .as_ref()
            .is_some_and(|uuid| footprint_uuid(footprint).as_ref() != Some(uuid))
    {
        return None;
    }
    let matches = |patterns: &[String]| {
        patterns
            .iter()
            .any(|pattern| glob_matches(pattern, reference))
    };
    let layer = match (matches(&brief.front), matches(&brief.back)) {
        (true, false) => "F.Cu",
        (false, true) => "B.Cu",
        _ => return None,
    };
    let at = [entry.x?, entry.y?, entry.angle?];
    at.iter()
        .all(|value| value.is_finite())
        .then_some((at, layer))
}

impl Mechanical {
    pub fn read(source: &Path, output: &Path) -> Result<Self, String> {
        let directory = output.parent().expect("output PCB has a directory");
        let actual = fs::canonicalize(source).map_err(mechanical_error)?;
        if fs::canonicalize(output).is_ok_and(|path| path == actual) {
            return Err(mechanical_error(
                "use a separate mechanical source and output PCB",
            ));
        }
        let (donor, source_sha256) = read(source).map_err(mechanical_error)?;
        let previous = output
            .is_file()
            .then(|| read(output).map_err(mechanical_error))
            .transpose()?;
        let previous_geometry = previous
            .as_ref()
            .filter(|(board, _)| has_outline_geometry(board));
        let (geometry, geometry_source, geometry_hash) = match previous_geometry {
            Some((pcb, hash)) => (pcb, output, hash),
            None => (&donor, source, &source_sha256),
        };
        let copper_layers = LayerTable::from_pcb(geometry)
            .map_err(mechanical_error)?
            .len();
        let objects: Vec<_> = geometry
            .children()
            .iter()
            .filter(|item| edge(item) || is_rule_area(item))
            .cloned()
            .collect();
        let edge_cuts_items = objects.iter().filter(|item| edge(item)).count();
        let report = KiCadImportedMechanics {
            source: source.to_path_buf(),
            source_sha256: source_sha256.clone(),
            geometry_source: geometry_source.to_path_buf(),
            geometry_source_sha256: geometry_hash.clone(),
            preserved_existing_geometry: previous_geometry.is_some(),
            edge_cuts_items,
            rule_areas: objects.len() - edge_cuts_items,
            constraints_kept: directory.join("constraints.json").is_file(),
            locks: Vec::new(),
            matched_additions: Vec::new(),
            retained_board_objects: Vec::new(),
            excluded_board_objects: Vec::new(),
            unmatched_locks: Vec::new(),
            conflicting_locks: Vec::new(),
        };
        Ok(Self {
            donor,
            previous: previous.map(|(board, _)| board),
            objects,
            retained_placer: retained_placer(directory)?,
            selected_footprints: BTreeMap::new(),
            copper_layers,
            report,
        })
    }

    pub fn select_footprints(&mut self, requested: &[String]) -> Result<(), String> {
        let mut selected = BTreeMap::new();
        for selection in requested {
            let (uuid, net) = selection
                .as_str()
                .split_once('=')
                .map_or((selection.as_str(), None), |(uuid, net)| {
                    (uuid, Some(net.to_owned()))
                });
            let valid = uuid.len() == 36
                && uuid.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_hexdigit()
                    }
                });
            if !valid || uuid == "00000000-0000-0000-0000-000000000000" {
                return Err(mechanical_error(format!(
                    "invalid --mechanical-footprint UUID {uuid}"
                )));
            }
            if net.as_deref() == Some("") {
                return Err(mechanical_error(format!(
                    "--mechanical-footprint {uuid}= requires an exact schematic net name"
                )));
            }
            if selected.insert(uuid.to_ascii_lowercase(), net).is_some() {
                return Err(mechanical_error(format!(
                    "repeated --mechanical-footprint UUID {uuid}"
                )));
            }
            let matches = board_parts(&self.donor)
                .filter(|part| footprint_uuid(part).is_some_and(|id| id.eq_ignore_ascii_case(uuid)))
                .count();
            if matches != 1 {
                return Err(mechanical_error(format!(
                    "--mechanical-footprint UUID {uuid} matched {matches} donor footprints"
                )));
            }
        }
        self.selected_footprints = selected;
        Ok(())
    }

    pub(super) fn validate_schematic_nets(&self, netlist: &Expr) -> Result<(), String> {
        for (uuid, net) in self
            .selected_footprints
            .iter()
            .filter_map(|(uuid, net)| net.as_deref().map(|net| (uuid, net)))
        {
            let matches = netlist
                .child("nets")
                .into_iter()
                .flat_map(Expr::children)
                .filter(|form| form.head() == Some("net"))
                .filter_map(|form| form_atom(form, "name", 1))
                .filter(|raw| unescape(raw) == net)
                .count();
            if matches != 1 {
                return Err(mechanical_error(format!(
                    "--mechanical-footprint {uuid}={net} matched {matches} schematic net declarations"
                )));
            }
        }
        Ok(())
    }

    pub fn validate_layers(&self, board: &Expr) -> Result<(), String> {
        let table: BTreeSet<_> = board
            .child("layers")
            .into_iter()
            .flat_map(Expr::children)
            .filter_map(|layer| layer.children().get(1).and_then(Expr::atom))
            .collect();
        for area in self.objects.iter().filter(|item| is_rule_area(item)) {
            let layers = area.child("layer").or_else(|| area.child("layers"));
            let Some(layers) = layers else {
                return Err(mechanical_error("a rule area has no layer"));
            };
            for layer in layers.children().iter().skip(1).map(Expr::atom) {
                if !layer
                    .is_some_and(|name| matches!(name, "*.Cu" | "F&B.Cu") || table.contains(name))
                {
                    return Err(mechanical_error(format!(
                        "rule area {} uses an unavailable PCB layer {}",
                        form_atom(area, "name", 1).unwrap_or("<unnamed>"),
                        layer.unwrap_or("<invalid>")
                    )));
                }
            }
            if layers.children().len() < 2
                || !area
                    .children()
                    .iter()
                    .any(|item| item.head() == Some("polygon"))
            {
                return Err(mechanical_error(
                    "a rule area needs PCB layers and an outline polygon",
                ));
            }
            for polygon in area
                .children()
                .iter()
                .filter(|item| item.head() == Some("polygon"))
            {
                if !outline::pts_points(polygon).is_ok_and(|points| {
                    points.len() >= 3 && points.iter().flatten().all(|x| x.is_finite())
                }) {
                    return Err(mechanical_error("invalid rule-area polygon"));
                }
            }
        }
        Ok(())
    }

    pub fn apply(
        &mut self,
        board: &mut Expr,
        schematic_parts: usize,
        schematic_references: &BTreeSet<&str>,
        same_donor_additions: Option<&BTreeMap<String, String>>,
    ) -> Result<(), String> {
        let codes = net_codes(board);
        let Expr::List(items) = board else {
            return Err(mechanical_error("generated board is not a list"));
        };
        let donor: Vec<_> = board_parts(&self.donor).collect();
        let geometry = if self.report.preserved_existing_geometry {
            self.previous.as_ref().expect("existing geometry source")
        } else {
            &self.donor
        };
        let donor_outline = perimeter_owners(&self.donor, |_| true);
        let outline_owners = if self.report.preserved_existing_geometry {
            // Reimported movable footprints can still be stacked at the
            // origin. Their independent slots do not become board pieces.
            perimeter_owners(geometry, |part| {
                footprint_locked(part) || donor_outline.iter().any(|owner| same_owner(part, owner))
            })
        } else {
            donor_outline.clone()
        };
        let results: Vec<_> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| matches!(item.head(), Some("footprint" | "module")))
            .take(schematic_parts)
            .collect();
        let mut bound_targets = BTreeMap::new();
        for (uuid, requested) in self
            .selected_footprints
            .iter()
            .filter_map(|(uuid, net)| net.as_deref().map(|net| (uuid, net)))
        {
            let names: BTreeSet<_> = results
                .iter()
                .flat_map(|(_, footprint)| footprint.children())
                .filter(|pad| pad.head() == Some("pad"))
                .filter_map(|pad| item_net(pad, &codes))
                .filter(|raw| unescape(raw) == requested)
                .collect();
            if names.len() != 1 {
                return Err(mechanical_error(format!(
                    "--mechanical-footprint {uuid}={requested} requires an unambiguous net on an imported schematic pad"
                )));
            }
            bound_targets.insert(uuid.as_str(), names.first().unwrap().to_string());
        }
        let counts = |parts: &[&Expr]| -> BTreeMap<String, usize> {
            let mut seen = BTreeMap::new();
            for part in parts {
                if let Some(reference) = footprint_reference(part) {
                    *seen.entry(reference).or_default() += 1;
                }
            }
            seen
        };
        let donor_counts = counts(&donor);
        let donor_net_codes = same_donor_additions
            .filter(|additions| !additions.is_empty())
            .map(|_| net_codes(&self.donor));
        let geometry_counts = counts(&board_parts(geometry).collect::<Vec<_>>());
        let result_counts = counts(&results.iter().map(|(_, part)| *part).collect::<Vec<_>>());
        let detached = if self.report.preserved_existing_geometry {
            detached_outlines(geometry, &donor_outline, &results)
        } else {
            Vec::new()
        };
        let outline_references: BTreeSet<_> = outline_owners
            .iter()
            .filter_map(|part| footprint_reference(part))
            .filter(|reference| result_counts.contains_key(reference))
            .collect();
        let outline_uuids: BTreeSet<_> = outline_owners
            .iter()
            .filter_map(|part| footprint_uuid(part))
            .collect();
        let mut transfers = Vec::new();
        let mut physical = Vec::new();
        let mut outlines = Vec::new();
        let selected = |part: &Expr| {
            footprint_uuid(part).is_some_and(|id| {
                self.selected_footprints
                    .contains_key(&id.to_ascii_lowercase())
            })
        };
        let binding_for = |part: &Expr| {
            let id = footprint_uuid(part)?.to_ascii_lowercase();
            let raw = bound_targets.get(id.as_str())?;
            Some(MechanicalBinding {
                target: raw,
                donor: &self.donor,
                local: self.previous.as_ref(),
            })
        };
        let locked = donor.iter().copied().filter(|part| {
            (footprint_locked(part) || selected(part))
                && footprint_reference(part)
                    .is_none_or(|reference| !outline_references.contains(&reference))
                && footprint_uuid(part).is_none_or(|uuid| !outline_uuids.contains(&uuid))
        });
        for (part, outline_owner) in outline_owners
            .into_iter()
            .map(|part| (part, true))
            .chain(locked.map(|part| (part, false)))
        {
            let source = footprint_reference(part).unwrap_or_default();
            let explicit = selected(part);
            if !explicit
                && !outline_owner
                && !schematic_references.contains(source.as_str())
                && !result_counts.contains_key(&source)
                && let Some(uuid) =
                    same_donor_additions.and_then(|additions| additions.get(&source))
                && footprint_uuid(part).is_some_and(|id| uuid.eq_ignore_ascii_case(&id))
            {
                let receipt = if donor_counts.get(&source) == Some(&1) {
                    confirm_added_instance(
                        items,
                        part,
                        &source,
                        uuid,
                        donor_net_codes.as_ref().expect("same donor"),
                        &codes,
                    )
                } else {
                    None
                };
                if let Some(receipt) = receipt {
                    self.report.matched_additions.push(receipt);
                } else {
                    self.report.conflicting_locks.push(format!(
                        "{source}: the explicit board addition does not reproduce this exact donor footprint"
                    ));
                }
                continue;
            }
            if explicit {
                if schematic_references.contains(source.as_str())
                    || result_counts.contains_key(&source)
                {
                    if binding_for(part).is_some() {
                        return Err(mechanical_error(format!(
                            "{source}: the selected footprint reference belongs to the schematic"
                        )));
                    }
                    self.report
                        .excluded_board_objects
                        .push(KiCadExcludedMechanicalObject {
                            reference: source,
                            uuid: footprint_uuid(part),
                            reason: "the selected footprint reference belongs to the schematic"
                                .into(),
                        });
                } else {
                    physical.push((part, true));
                }
                continue;
            }
            let invalid_outline_owner =
                |reason: &str| mechanical_error(format!("perimeter owner {source}: {reason}"));
            let mut conflict = |reason| {
                lock_conflict(
                    &mut self.report.conflicting_locks,
                    &source,
                    outline_owner,
                    reason,
                )
            };
            let Some(target_count) = result_counts.get(&source) else {
                if !schematic_references.contains(source.as_str())
                    && footprint_locked(part)
                    && has_physical_geometry(part)
                {
                    physical.push((part, false));
                    continue;
                }
                if outline_owner {
                    return Err(invalid_outline_owner(
                        "no matching schematic footprint; import its footprint or supply the outline as board-level Edge.Cuts",
                    ));
                }
                self.report
                    .unmatched_locks
                    .push(KiCadUnmatchedMechanicalLock {
                        reference: source,
                        uuid: footprint_uuid(part),
                    });
                continue;
            };
            let native_counts = if outline_owner {
                &geometry_counts
            } else {
                &donor_counts
            };
            if source.is_empty() || native_counts.get(&source) != Some(&1) {
                conflict("mechanical reference is ambiguous")?;
                continue;
            }
            if *target_count > 1 {
                conflict("imported reference is ambiguous")?;
                continue;
            }
            let &(index, target) = results
                .iter()
                .find(|(_, item)| footprint_reference(item).as_deref() == Some(source.as_str()))
                .expect("imports counted by reference");
            if form_atom(part, "path", 1)
                .zip(form_atom(target, "path", 1))
                .is_some_and(|(from, to)| from != to)
            {
                conflict("different schematic paths")?;
                continue;
            }
            let layer = form_atom(part, "layer", 1).unwrap_or_default();
            let pose = form_at(part)
                .ok()
                .filter(|at| at.iter().all(|value| value.is_finite()));
            let Some(at) = pose.filter(|_| matches!(layer, "F.Cu" | "B.Cu")) else {
                conflict("invalid pose or side")?;
                continue;
            };
            let retained = match self.retained_placer.as_ref() {
                Some(config) if config.free.contains(&source) => None,
                Some(config)
                    if !matches!(config.constraints, Some(KiCadConstraintsSource::File(_))) =>
                {
                    retained_pose(config, &source, target)
                }
                _ => Some((at, layer)),
            };
            if outline_owner
                && !retained.is_some_and(|(next, face)| {
                    layer == face
                        && (next[0] - at[0]).abs() <= 1e-6
                        && (next[1] - at[1]).abs() <= 1e-6
                        && ((next[2] - at[2] + 180.0).rem_euclid(360.0) - 180.0).abs() <= 1e-6
                })
            {
                return Err(invalid_outline_owner(
                    "the current constraints must keep its authored absolute pose and side; move the outline to board-level Edge.Cuts before releasing this part",
                ));
            }
            let applied = retained.is_some();
            let (at, layer) = retained.unwrap_or((at, layer));
            if applied {
                let back = layer == "B.Cu";
                let mut placed = crate::board_footprints::posed(
                    target,
                    [at[0], at[1], if back { -at[2] } else { at[2] }],
                )?;
                if back {
                    crate::flip::flip_footprint(&mut placed)?;
                }
                if outline_owner {
                    if !same_pad_sites(part, &placed) {
                        return Err(invalid_outline_owner(
                            "the imported numbered pad sites use a different local frame; use the matching footprint model or separate the perimeter into board-level Edge.Cuts",
                        ));
                    }
                    if let Expr::List(fields) = &mut placed {
                        fields.retain(|field| !footprint_edge(field));
                    }
                    outlines.push((index, part));
                }
                set_native_lock(&mut placed, true)?;
                transfers.push((index, placed));
            } else if footprint_locked(target) {
                let mut unlocked = target.clone();
                set_native_lock(&mut unlocked, false)?;
                transfers.push((index, unlocked));
            }
            self.report.locks.push(KiCadImportedMechanicalLock {
                reference: source,
                at,
                layer: layer.to_string(),
                applied,
                perimeter_edge_cuts: if outline_owner {
                    part.children()
                        .iter()
                        .filter(|item| footprint_edge(item))
                        .count()
                } else {
                    0
                },
            });
        }
        for (index, placed) in transfers {
            items[index] = placed;
        }
        for index in detached {
            if let Expr::List(fields) = &mut items[index] {
                fields.retain(|field| !footprint_edge(field));
            }
        }
        let mut ids = BTreeSet::new();
        for item in items.iter() {
            collect_ids(item, &mut ids);
        }
        for (index, source) in outlines {
            let Expr::List(fields) = &mut items[index] else {
                return Err(mechanical_error("generated footprint is not a list"));
            };
            for item in source.children().iter().filter(|item| footprint_edge(item)) {
                if let Some(uuid) = footprint_uuid(item)
                    && !ids.insert(uuid.to_ascii_lowercase())
                {
                    return Err(mechanical_error(format!(
                        "duplicate perimeter item UUID {uuid}"
                    )));
                }
                fields.push(item.clone());
            }
        }
        for mut object in self.objects.drain(..) {
            if let Some(uuid) = footprint_uuid(&object)
                && !ids.insert(uuid.to_ascii_lowercase())
            {
                return Err(mechanical_error(format!(
                    "duplicate board item UUID {uuid}"
                )));
            }
            if is_rule_area(&object)
                && let Expr::List(fields) = &mut object
            {
                fields.retain(|field| {
                    !matches!(
                        field.head(),
                        Some("net" | "net_name" | "filled_polygon" | "fill_segments")
                    )
                });
            }
            items.push(object);
        }
        let mut previous = BTreeMap::new();
        if let Some(board) = &self.previous {
            for part in board_parts(board) {
                if let Some(uuid) = footprint_uuid(part) {
                    previous
                        .entry(uuid.to_ascii_lowercase())
                        .or_insert_with(Vec::new)
                        .push(part);
                }
            }
        }
        for (part, explicit) in physical {
            let binding = binding_for(part);
            let prior =
                footprint_uuid(part).and_then(|uuid| previous.get(&uuid.to_ascii_lowercase()));
            if prior.is_some_and(|forms| forms.len() != 1) {
                if binding.is_some() {
                    return Err(mechanical_error(format!(
                        "the output PCB repeats mechanical UUID {}",
                        footprint_uuid(part).unwrap_or_default()
                    )));
                }
                self.report
                    .excluded_board_objects
                    .push(KiCadExcludedMechanicalObject {
                        reference: footprint_reference(part).unwrap_or_default(),
                        uuid: footprint_uuid(part),
                        reason: "the output PCB repeats this mechanical UUID".into(),
                    });
                continue;
            }
            match preserve(
                part,
                prior.map(|forms| forms[0]),
                &mut ids,
                explicit,
                binding,
            ) {
                Ok((part, receipt)) => {
                    items.push(part);
                    self.report.retained_board_objects.push(receipt);
                }
                Err(excluded) if binding.is_some() => {
                    return Err(mechanical_error(format!(
                        "{}: {}",
                        excluded.reference, excluded.reason
                    )));
                }
                Err(excluded) => self.report.excluded_board_objects.push(excluded),
            }
        }
        if let Some(uuid) = bound_targets.keys().find(|uuid| {
            !self
                .report
                .retained_board_objects
                .iter()
                .any(|object| object.uuid.eq_ignore_ascii_case(uuid))
        }) {
            return Err(mechanical_error(format!(
                "the selected net-bound footprint {uuid} was omitted"
            )));
        }
        // Footprint-local Edge.Cuts can join the outer board contour.
        // Validate those at their assigned poses and retain their native
        // local forms in the output. Loose schematic footprints are still
        // at import placeholders and cannot define a mechanical outline.
        let retained: BTreeSet<_> = self
            .report
            .retained_board_objects
            .iter()
            .map(|part| part.uuid.as_str())
            .collect();
        let mut geometry = vec![Expr::Atom("kicad_pcb".into())];
        geometry.extend(
            items
                .iter()
                .filter(|item| {
                    edge(item)
                        || item.head() == Some("footprint")
                            && item.children().iter().any(footprint_edge)
                            && (footprint_locked(item)
                                || footprint_uuid(item)
                                    .is_some_and(|uuid| retained.contains(uuid.as_str())))
                })
                .cloned(),
        );
        if let Err(error) = outline::board_loops_strict(&Expr::List(geometry)) {
            let omitted = self
                .report
                .excluded_board_objects
                .iter()
                .map(|part| format!("{}: {}", part.reference, part.reason))
                .collect::<Vec<_>>();
            return Err(mechanical_error(if omitted.is_empty() {
                error
            } else {
                format!(
                    "{error}. Excluded physical footprints: {}",
                    omitted.join(", ")
                )
            }));
        }
        Ok(())
    }

    pub fn constrain(&self, starter: &mut serde_json::Value) {
        let locked: Vec<_> = self
            .report
            .locks
            .iter()
            .filter(|lock| lock.applied)
            .collect();
        starter["place"] = serde_json::json!(
            locked
                .iter()
                .map(|lock| serde_json::json!({
                    "part": lock.reference,
                    "origin": "absolute",
                    "x": lock.at[0],
                    "y": lock.at[1],
                    "angle": lock.at[2],
                }))
                .collect::<Vec<_>>()
        );
        starter["back"] = serde_json::json!(
            locked
                .iter()
                .filter(|lock| lock.layer == "B.Cu")
                .map(|lock| &lock.reference)
                .collect::<Vec<_>>()
        );
        starter["front"] = serde_json::json!(
            locked
                .iter()
                .filter(|lock| lock.layer == "F.Cu")
                .map(|lock| &lock.reference)
                .collect::<Vec<_>>()
        );
    }
}

fn confirm_added_instance(
    items: &[Expr],
    source: &Expr,
    reference: &str,
    uuid: &str,
    donor_nets: &BTreeMap<String, String>,
    board_nets: &BTreeMap<String, String>,
) -> Option<KiCadImportedMechanicalAddition> {
    use crate::board_footprints::{library_form, same_library_model};

    let mut matching = items
        .iter()
        .filter(|item| footprint_uuid(item).is_some_and(|id| id.eq_ignore_ascii_case(uuid)));
    let installed = matching.next()?;
    let at = form_at(source).ok()?;
    let placed = form_at(installed).ok()?;
    let layer = form_atom(source, "layer", 1)?;
    if matching.next().is_some()
        || !explicitly_board_only(installed)
        || !footprint_locked(installed)
        || footprint_reference(installed).as_deref() != Some(reference)
        || form_atom(installed, "layer", 1) != Some(layer)
        || !at.into_iter().chain(placed).all(f64::is_finite)
        || (at[0] - placed[0]).abs() > 1e-6
        || (at[1] - placed[1]).abs() > 1e-6
        || ((at[2] - placed[2] + 180.0).rem_euclid(360.0) - 180.0).abs() > 1e-6
    {
        return None;
    }
    let signature = |footprint: &Expr, nets: &BTreeMap<String, String>| {
        footprint
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
            .map(|pad| {
                (
                    footprint_uuid(pad),
                    item_net(pad, nets)
                        .filter(|name| !name.is_empty())
                        .map(str::to_owned),
                )
            })
            .collect::<Vec<_>>()
    };
    if signature(source, donor_nets) != signature(installed, board_nets)
        || !same_library_model(
            &library_form(source, "footprint").ok()?,
            &library_form(installed, "footprint").ok()?,
        )
    {
        return None;
    }
    Some(KiCadImportedMechanicalAddition {
        reference: reference.into(),
        uuid: uuid.into(),
        at,
        layer: layer.into(),
    })
}

pub(super) fn set_native_lock(footprint: &mut Expr, locked: bool) -> Result<(), String> {
    let Expr::List(fields) = footprint else {
        return Err(mechanical_error("generated footprint is not a list"));
    };
    let mut index = 0;
    fields.retain(|field| {
        index += 1;
        index <= 2
            || (field.head() != Some("locked")
                && !matches!(field, Expr::Atom(raw) if raw == "locked"))
    });
    if locked {
        fields.push(parse("(locked yes)")?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_addition_must_keep_the_original_instance() {
        let part = r#"(footprint "lib:Mount"
            (layer "F.Cu") (locked yes) (uuid "mount") (at 24 30 0)
            (property "Reference" "H1") (attr board_only)
            (pad "1" thru_hole circle (at 0 0) (size 2 2) (drill 1)
                (layers "*.Cu" "*.Mask") (uuid "land") (net "GND")))"#;
        let source = parse(part).unwrap();
        let codes = BTreeMap::new();
        let confirms = |parts: &[Expr]| {
            confirm_added_instance(parts, &source, "H1", "mount", &codes, &codes).is_some()
        };
        assert!(confirms(std::slice::from_ref(&source)));
        for (old, new) in [
            ("H1", "Alias"),
            ("(at 24 30 0)", "(at 24 32 0)"),
            ("F.Cu", "B.Cu"),
            ("board_only", "dnp"),
            ("GND", "OTHER"),
            ("(drill 1)", "(drill 1.1)"),
            ("land", "different-land"),
        ] {
            let alternative = parse(&part.replace(old, new)).unwrap();
            assert!(!confirms(&[alternative]), "{old} -> {new}");
        }
        assert!(!confirms(&[source.clone(), source.clone()]));
    }

    #[test]
    fn a_schematic_outline_owner_is_held_with_native_shapes_and_current_pad_nets() {
        let directory = temporary_next_to(&std::env::temp_dir().join("pcb-maker-perimeter-owner"));
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("donor.kicad_pcb");
        let output = directory.join("output.kicad_pcb");
        let layers = r#"(layers (0 "F.Cu" signal) (2 "B.Cu" signal))"#;
        let outline = r#"
            (fp_rect (start -5 -7) (end 5 7) (layer "Edge.Cuts") (uuid "perimeter"))
            (fp_circle (center 2 2) (end 2 3) (layer "Edge.Cuts") (uuid "hole"))"#;
        let donor = format!(
            r#"(kicad_pcb {layers}
              (footprint "Connector" (layer "B.Cu") (at 50 60 37)
                (property "Reference" "J1") (path "/same-schematic-symbol")
                (pad "1" smd rect (at 2 0) (size .2 .2) (layers "B.Cu") (net "OLD"))
                {outline})
              (footprint "Slotted" (layer "F.Cu") (at 50 60)
                (property "Reference" "S1")
                (fp_circle (center 0 0) (end 0 1) (layer "Edge.Cuts"))))"#,
        );
        fs::write(&source, donor).unwrap();
        let inputs = format!(
            r#"(kicad_pcb {layers}
              (footprint "Connector" (layer "F.Cu") (at 0 0)
                (property "Reference" "J1") (path "/same-schematic-symbol")
                (pad "1" smd rect (at 2 0) (size .2 .2) (layers "F.Cu") (net "CURRENT"))
                (fp_rect (start -5 -7) (end 5 7) (layer "Edge.Cuts")))
              (footprint "Slotted" (layer "F.Cu") (at 0 0)
                (property "Reference" "S1")
                (fp_circle (center 0 0) (end 0 1) (layer "Edge.Cuts"))))"#,
        );
        let mut transfer = Mechanical::read(&source, &output).unwrap();
        let references = BTreeSet::from(["J1", "S1"]);
        let mut board = parse(&inputs).unwrap();
        transfer.apply(&mut board, 2, &references, None).unwrap();
        let [held] = transfer.report.locks.as_slice() else {
            panic!("only the perimeter owner is held")
        };
        assert_eq!((&*held.reference, held.perimeter_edge_cuts), ("J1", 2));
        let [kept, slotted]: [&Expr; 2] =
            board_parts(&board).collect::<Vec<_>>().try_into().unwrap();
        assert!(footprint_locked(kept));
        assert_eq!(form_at(kept).unwrap(), [50.0, 60.0, 37.0]);
        assert_eq!(form_atom(kept, "layer", 1), Some("B.Cu"));
        assert_eq!(
            form_atom(kept.child("pad").unwrap(), "net", 1),
            Some("CURRENT")
        );
        let shapes = kept.children().iter().filter(|item| footprint_edge(item));
        assert_eq!(
            shapes.cloned().collect::<Vec<_>>(),
            parse(&format!("({outline})")).unwrap().children()
        );
        assert!(!footprint_locked(slotted));
        assert_eq!(form_at(slotted).unwrap(), [0.0; 3]);

        fs::write(&output, encode(&board)).unwrap();
        let mut brief = serde_json::json!({"version": 1});
        transfer.constrain(&mut brief);
        let constraints = directory.join("constraints.json");
        fs::write(&constraints, brief.to_string()).unwrap();
        let mut current = parse(&inputs).unwrap();
        Mechanical::read(&source, &output)
            .unwrap()
            .apply(&mut current, 2, &references, None)
            .unwrap();
        assert_eq!(current, board);

        brief["place"] = serde_json::json!([]);
        fs::write(&constraints, brief.to_string()).unwrap();
        let error = Mechanical::read(&source, &output)
            .unwrap()
            .apply(&mut current, 2, &references, None)
            .unwrap_err();
        assert!(error.contains("perimeter owner J1"), "{error}");
        assert!(error.contains("authored absolute pose"), "{error}");

        // A deliberate edit that moves the shape to the board frees J1.
        let Expr::List(items) = &mut board else {
            panic!()
        };
        let Expr::List(edited) = &mut items[2] else {
            panic!()
        };
        edited.retain(|field| !footprint_edge(field));
        items.push(parse(r#"(gr_rect (start 44 53) (end 56 67) (layer "Edge.Cuts"))"#).unwrap());
        fs::write(&output, encode(&board)).unwrap();
        let mut reimported = parse(&inputs).unwrap();
        Mechanical::read(&source, &output)
            .unwrap()
            .apply(&mut reimported, 2, &references, None)
            .unwrap();
        let freed = reimported.child("footprint").unwrap();
        assert!(!footprint_locked(freed));
        assert!(!freed.children().iter().any(footprint_edge));
        assert_eq!(reimported.child("gr_rect"), board.child("gr_rect"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_board_only_footprint_can_complete_the_supplied_outer_contour() {
        let source =
            temporary_next_to(&std::env::temp_dir().join("pcb-maker-joined-outline.kicad_pcb"));
        let output = source.with_extension("generated.kicad_pcb");
        let edges = r#"
          (gr_line (start 0 0) (end 10 0) (layer "Edge.Cuts"))
          (gr_line (start 10 0) (end 10 10) (layer "Edge.Cuts"))
          (gr_line (start 10 10) (end 0 10) (layer "Edge.Cuts"))"#;
        let footprint = r#"
          (footprint "Breakaway" locked (layer "F.Cu") (at 0 10)
            (uuid "ebd8b87c-8129-4bec-8563-b60f1c3a44be")
            (fp_text reference "MB1" (at 0 0) (layer "F.SilkS"))
            (fp_line (start 0 0) (end 0 -10) (layer "Edge.Cuts"))
            (pad "" np_thru_hole circle (at 1 -5) (size 0.4 0.4) (drill 0.4) (layers "*.Cu")))"#;
        let layers = r#"(layers (0 "F.Cu" signal) (2 "B.Cu" signal))"#;
        fs::write(&source, format!("(kicad_pcb {layers} {edges} {footprint})")).unwrap();
        let mut transfer = Mechanical::read(&source, &output).unwrap();
        let mut board = parse(&format!("(kicad_pcb {layers})")).unwrap();
        transfer
            .apply(&mut board, 0, &BTreeSet::new(), None)
            .unwrap();
        assert_eq!(transfer.report.edge_cuts_items, 3);
        let [part] = transfer.report.retained_board_objects.as_slice() else {
            panic!()
        };
        assert_eq!((part.edge_cuts_items, part.npth_drills), (1, 1));
        assert_eq!(board_parts(&board).count(), 1);
        assert!(outline::board_loops_strict(&board).is_ok());

        let plated = footprint.replace("np_thru_hole", "thru_hole");
        fs::write(&source, format!("(kicad_pcb {layers} {edges} {plated})")).unwrap();
        let mut transfer = Mechanical::read(&source, &output).unwrap();
        let mut board = parse(&format!("(kicad_pcb {layers})")).unwrap();
        let error = transfer
            .apply(&mut board, 0, &BTreeSet::new(), None)
            .unwrap_err();
        assert!(error.contains("Edge.Cuts"), "{error}");
        assert!(
            error.contains("MB1") && error.contains("plating"),
            "{error}"
        );
        fs::remove_file(source).unwrap();
    }

    #[test]
    fn explicit_plated_drills_match_native_uuids_and_keep_admissible_local_edits() {
        let source =
            temporary_next_to(&std::env::temp_dir().join("pcb-maker-plated-mechanics.kicad_pcb"));
        let output = source.with_extension("generated.kicad_pcb");
        let first = "11111111-1111-1111-1111-111111111111";
        let second = "22222222-2222-2222-2222-222222222222";
        let pad_id = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
        let layers = r#"(layers (0 "F.Cu" signal) (2 "B.Cu" signal))"#;
        let pad = format!(
            r#"(pad "1" thru_hole circle (at 0 2) (size 2 2) (drill 1.2)
                   (layers "*.Cu" "*.Mask") (remove_unused_layers no) (uuid "{pad_id}"))"#
        );
        let bracket = |uuid, at, layer, pad: &str| {
            format!(
                r#"(footprint "Bracket" (layer "{layer}") (at {at})
                    (uuid "{uuid}") (property "Reference" "REF**") (attr through_hole) {pad})"#
            )
        };
        let write_source = |second_pad: &str| {
            fs::write(
                &source,
                format!(
                    r#"(kicad_pcb {layers}
                         (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
                         {} {})"#,
                    bracket(first, "3 4", "F.Cu", &pad),
                    bracket(second, "7 8 37", "B.Cu", second_pad),
                ),
            )
            .unwrap();
        };
        let second_pad = pad.replace(pad_id, "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
        let blank = || parse(&format!("(kicad_pcb {layers})")).unwrap();
        let select_both = [first.into(), second.into()];
        let apply = |transfer: &mut Mechanical| {
            let mut board = blank();
            transfer
                .apply(&mut board, 0, &BTreeSet::new(), None)
                .unwrap();
            board
        };
        write_source(&second_pad);
        let mut ordinary = Mechanical::read(&source, &output).unwrap();
        assert_eq!(board_parts(&apply(&mut ordinary)).count(), 0);
        assert!(
            ordinary
                .select_footprints(&[first.into(), first.into()])
                .is_err()
        );
        assert!(ordinary.select_footprints(&["REF**".into()]).is_err());
        assert!(
            ordinary
                .select_footprints(&["33333333-3333-3333-3333-333333333333".into()])
                .is_err()
        );

        let mut explicit = Mechanical::read(&source, &output).unwrap();
        explicit.select_footprints(&select_both).unwrap();
        let board = apply(&mut explicit);
        assert_eq!(board_parts(&board).count(), 2);
        assert!(board_parts(&board).all(footprint_locked));
        assert!(explicit.report.excluded_board_objects.is_empty());
        for receipt in &explicit.report.retained_board_objects {
            assert!(receipt.explicit_selection);
            assert_eq!(receipt.reference, "REF**");
            assert_eq!(receipt.npth_drills, 0);
            let [hole] = receipt.plated_drills.as_slice() else {
                panic!("expected a plated hole")
            };
            assert_eq!(hole.local_xy_mm, [0.0, 2.0]);
            assert_eq!((hole.drill_mm, hole.land_mm), (1.2, 2.0));
        }
        let local = encode(&board).replace("(at 7 8 37)", "(at 8 9 37)");
        fs::write(&output, &local).unwrap();
        let mut reimport = Mechanical::read(&source, &output).unwrap();
        reimport.select_footprints(&select_both).unwrap();
        let saved = apply(&mut reimport);
        assert_eq!(encode(&saved), local);
        let receipt = &reimport.report.retained_board_objects[1];
        assert!(receipt.kept_local);
        assert_eq!(receipt.at, [8.0, 9.0, 37.0]);
        assert_eq!(receipt.layer, "B.Cu");

        for rejected_pad in [
            second_pad.replacen("(at 0 2)", r#"(at 0 2) (net "GND")"#, 1),
            second_pad.replace("(drill 1.2)", "(drill oval 1.2 1.5)"),
            second_pad.replace("(remove_unused_layers no)", "(remove_unused_layers yes)"),
        ] {
            write_source(&rejected_pad);
            let mut rejected = Mechanical::read(&source, &output).unwrap();
            rejected.select_footprints(&select_both).unwrap();
            assert_eq!(board_parts(&apply(&mut rejected)).count(), 1);
            assert_eq!(
                rejected.report.excluded_board_objects[0].uuid.as_deref(),
                Some(second)
            );
        }
        fs::remove_file(source).unwrap();
        fs::remove_file(output).unwrap();
    }

    #[test]
    fn explicitly_bound_plated_drills_keep_only_their_existing_schematic_net() {
        let source =
            temporary_next_to(&std::env::temp_dir().join("pcb-maker-bound-mechanics.kicad_pcb"));
        let output = source.with_extension("generated.kicad_pcb");
        let id = "11111111-1111-1111-1111-111111111111";
        let selected = [format!("{id}=GND")];
        let layers = r#"(layers (0 "F.Cu" signal) (2 "B.Cu" signal))"#;
        let donor = format!(
            r#"(kicad_pcb {layers} (net 7 "GND") (net 8 "OTHER")
            (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
            (footprint "Mount" (layer "F.Cu") (at 4 5) (uuid "{id}")
              (property "Reference" "M1") (attr through_hole board_only)
              (pad "" thru_hole circle (at 0 0) (size 2 2) (drill 1)
                (layers "*.Cu" "*.Mask") (net 7 "GND")
                (teardrops (enabled yes) (max_width 2))
                (pinfunction "GND") (pintype "passive") (uuid "1111"))
              (pad "" thru_hole circle (at 0 2) (size 1 1) (drill 0.5)
                (layers "*.Cu" "*.Mask") (net 0 "") (uuid "2222"))))"#
        );
        let netlist = parse(r#"(export (nets (net (name "GND"))))"#).unwrap();
        let blank = || {
            parse(&format!(
                r#"(kicad_pcb {layers}
            (footprint "Schematic" (layer "F.Cu") (at 0 0) (uuid "3333")
                (property "Reference" "J1") (path "/1234")
                (pad "1" smd rect (net "GND"))))"#
            ))
            .unwrap()
        };
        let references = BTreeSet::from(["J1"]);
        let import = |body: &str| -> Result<(Expr, Mechanical), String> {
            fs::write(&source, body).unwrap();
            let mut transfer = Mechanical::read(&source, &output)?;
            transfer.select_footprints(&selected)?;
            transfer.validate_schematic_nets(&netlist)?;
            let mut board = blank();
            transfer.apply(&mut board, 1, &references, None)?;
            Ok((board, transfer))
        };
        let (board, report) = import(&donor).unwrap();
        let receipt = &report.report.retained_board_objects[0];
        assert_eq!(receipt.plated_drills[0].net.as_deref(), Some("GND"));
        assert!(receipt.plated_drills[1].net.is_none());
        let saved = encode(&board);
        assert!(!saved.contains("(net 7"));
        let mount = board_parts(&board)
            .find(|part| footprint_reference(part).as_deref() == Some("M1"))
            .unwrap();
        let pad = mount.child("pad").unwrap();
        assert_eq!(form_atom(pad, "pinfunction", 1), Some("GND"));
        assert_eq!(form_atom(pad, "pintype", 1), Some("passive"));
        let teardrops = pad.child("teardrops").unwrap();
        assert_eq!(form_atom(teardrops, "enabled", 1), Some("yes"));
        assert_eq!(form_atom(teardrops, "max_width", 1), Some("2"));
        fs::write(&output, &saved).unwrap();
        let (reimported, _) = import(&donor).unwrap();
        assert_eq!(encode(&reimported), saved);

        let Expr::List(mut edited) = board else {
            unreachable!()
        };
        let Expr::List(fields) = edited.last_mut().unwrap() else {
            unreachable!()
        };
        for pad in fields.iter_mut().filter(|item| item.head() == Some("pad")) {
            let id = footprint_uuid(pad).unwrap();
            let Expr::List(fields) = pad else {
                unreachable!()
            };
            fields.retain(|field| field.head() != Some("net"));
            if id == "2222" {
                fields.push(parse(r#"(net "GND")"#).unwrap());
            }
        }
        let changed = encode(&Expr::List(edited));
        fs::write(&output, &changed).unwrap();
        assert!(import(&donor).err().unwrap().contains("assignments differ"));
        assert_eq!(fs::read_to_string(&output).unwrap(), changed);
        fs::remove_file(&output).unwrap();

        assert!(import(&donor.replace("(net 0 \"\")", "(net 8 \"OTHER\")")).is_err());
        assert!(
            import(&donor.replace("(net 7 \"GND\") (net 8", "(net 7 \"OTHER\") (net 8")).is_err()
        );
        assert!(import(&donor.replace(" board_only", "")).is_err());
        assert!(import(&donor.replace("\"passive\"", "\"no_connect\"")).is_err());
        let mut transfer = Mechanical::read(&source, &output).unwrap();
        transfer.select_footprints(&selected).unwrap();
        let absent = parse("(export (nets))").unwrap();
        assert!(transfer.validate_schematic_nets(&absent).is_err());
        fs::remove_file(source).unwrap();
    }

    #[test]
    fn a_mechanical_starter_keeps_a_clipped_back_lock_and_preserves_reimport_edits() {
        let directory =
            temporary_next_to(&std::env::temp_dir().join("pcb-maker-mechanical-starter"));
        let library = directory.join("local.pretty");
        fs::create_dir_all(&library).unwrap();
        fs::write(
            library.join("Test.kicad_mod"),
            r#"(footprint "Test" (layer "F.Cu")
              (fp_rect (start -1 -1) (end 1 1) (layer "F.CrtYd")
                (stroke (width 0.05) (type solid)) (fill none))
              (pad "1" smd rect (at 0 0) (size 0.2 0.2) (layers "F.Cu" "F.Paste" "F.Mask")))"#,
        )
        .unwrap();
        let netlist = directory.join("input.net");
        fs::write(
            &netlist,
            r#"(export (version "E")
              (components
                (comp (ref "J1") (value "one") (footprint "local:Test") (tstamps "1111"))
                (comp (ref "J2") (value "two") (footprint "local:Test") (tstamps "2222")))
              (nets (net (code "1") (name "G")
                (node (ref "J1") (pin "1")) (node (ref "J2") (pin "1")))))"#,
        )
        .unwrap();
        let donor = directory.join("starter.kicad_pcb");
        fs::write(
            &donor,
            r#"(kicad_pcb
              (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (2 "B.Cu" signal))
              (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts")
                (stroke (width 0.05) (type solid)) (fill none))
              (footprint "other:Donor" locked (layer "B.Cu") (at 0.5 6 37)
                (fp_text reference locked "J1" (at 0 0) (layer "B.SilkS")))
              (footprint "other:Donor" (layer "F.Cu") (at 14 10) (locked yes)
                (property "Reference" "J2"))
              (footprint "other:Logo" (layer "F.Cu") (at 10 10) (locked yes)
                (property "Reference" "REF**") (uuid "logo-1"))
              (footprint "other:Logo" (layer "F.Cu") (at 12 12) (locked yes)
                (property "Reference" "REF**") (uuid "logo-2"))
              (zone (name "hatch boundary") (layers "F&B.Cu" "In1.Cu" "Eco1.User")
                (keepout (tracks not_allowed) (vias allowed) (pads allowed)
                  (copperpour not_allowed) (footprints allowed))
                (polygon (pts (xy 4 4) (xy 8 4) (xy 8 8) (xy 4 8))))
              (zone (net "G") (layer "B.Cu")
                (polygon (pts (xy 4 4) (xy 8 4) (xy 8 8))))
              (segment (start 2 2) (end 3 3) (layer "B.Cu") (width 0.2) (net "G")))"#,
        )
        .unwrap();
        let options = KiCadNetlistImportOptions {
            mechanical_from: Some(&donor),
            ..Default::default()
        };
        let output = directory.join("output");
        let report =
            import_kicad_netlist_with_options(&netlist, &output, "board", options).unwrap();
        assert_eq!(report.copper_layers, 4);
        assert!(report.missing.is_empty(), "{:?}", report.missing);
        let mechanical = report.mechanical_from.unwrap();
        assert_eq!(mechanical.edge_cuts_items, 1);
        assert_eq!(mechanical.rule_areas, 1);
        assert!(mechanical.conflicting_locks.is_empty());
        assert_eq!(mechanical.unmatched_locks.len(), 2);
        assert!(
            mechanical
                .unmatched_locks
                .iter()
                .all(|lock| lock.reference == "REF**")
        );
        assert_eq!(
            mechanical.unmatched_locks[0].uuid.as_deref(),
            Some("logo-1")
        );
        assert_eq!(
            mechanical.unmatched_locks[1].uuid.as_deref(),
            Some("logo-2")
        );
        assert_eq!(mechanical.locks.len(), 2);
        assert_eq!(mechanical.locks[0].reference, "J1");
        assert_eq!(mechanical.locks[0].at, [0.5, 6.0, 37.0]);
        assert!(mechanical.locks[0].applied);
        let board = fs::read_to_string(&report.board).unwrap();
        let pcb = parse(&board).unwrap();
        let j1 = board_parts(&pcb)
            .find(|part| footprint_reference(part).as_deref() == Some("J1"))
            .unwrap();
        let j2 = board_parts(&pcb)
            .find(|part| footprint_reference(part).as_deref() == Some("J2"))
            .unwrap();
        assert_eq!(form_at(j1).unwrap(), [0.5, 6.0, 37.0]);
        assert_eq!(form_atom(j1, "layer", 1), Some("B.Cu"));
        assert_eq!(
            form_atom(j1.child("pad").unwrap(), "layers", 1),
            Some("B.Cu")
        );
        assert_eq!(form_atom(j2, "layer", 1), Some("F.Cu"));
        assert!(footprint_locked(j1));
        assert!(footprint_locked(j2));
        assert_eq!(
            pcb.children()
                .iter()
                .filter(|item| item.head() == Some("zone"))
                .count(),
            1
        );
        assert!(pcb.child("segment").is_none());
        let area = pcb.child("zone").unwrap();
        let layers = LayerTable::from_pcb(&pcb).unwrap();
        assert_eq!(layers.mask_of_item(area).unwrap(), 0b1011);
        assert_eq!(form_atom(area, "layers", 3), Some("Eco1.User"));
        let brief = output.join("constraints.json");
        let text = fs::read_to_string(&brief).unwrap();
        let constraints: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(constraints["move_all"], true);
        assert_eq!(constraints["back"], serde_json::json!(["J1"]));
        assert_eq!(constraints["place"][0]["x"], 0.5);
        assert!(constraints.get("outline").is_none());
        assert!(constraints.get("edge").is_none());

        let unchanged =
            import_kicad_netlist_with_options(&netlist, &output, "board", options).unwrap();
        assert!(
            unchanged
                .mechanical_from
                .unwrap()
                .locks
                .iter()
                .all(|lock| lock.applied)
        );
        assert_eq!(fs::read_to_string(&brief).unwrap(), text);

        let edited = text.replace("0.5", "0.75");
        fs::write(&brief, &edited).unwrap();
        let mut local = pcb;
        let Expr::List(items) = &mut local else {
            unreachable!()
        };
        items.retain(|item| !is_rule_area(item));
        let source = encode(&local).replace("(end 20 20)", "(end 22 20)");
        fs::write(&report.board, source).unwrap();
        let reimport =
            import_kicad_netlist_with_options(&netlist, &output, "board", options).unwrap();
        let mechanical = reimport.mechanical_from.unwrap();
        assert!(mechanical.constraints_kept && mechanical.preserved_existing_geometry);
        assert!(mechanical.locks[0].applied);
        assert_eq!(mechanical.locks[0].at, [0.75, 6.0, 37.0]);
        assert_eq!(mechanical.rule_areas, 0);
        assert_eq!(fs::read_to_string(&brief).unwrap(), edited);
        assert!(
            fs::read_to_string(&report.board)
                .unwrap()
                .contains("(end 22 20)")
        );

        let mut constraints: serde_json::Value = serde_json::from_str(&edited).unwrap();
        // Keeping the side permission alone must leave J1 movable.
        constraints["place"].as_array_mut().unwrap().remove(0);
        fs::write(&brief, serde_json::to_string(&constraints).unwrap()).unwrap();
        let released =
            import_kicad_netlist_with_options(&netlist, &output, "board", options).unwrap();
        let pcb = parse(&fs::read_to_string(&released.board).unwrap()).unwrap();
        let lock_refs: Vec<_> = board_parts(&pcb)
            .filter(|part| footprint_locked(part))
            .filter_map(footprint_reference)
            .collect();
        assert_eq!(lock_refs, ["J2"]);
        assert!(!released.mechanical_from.unwrap().locks[0].applied);

        // The current layout can select a custom brief and explicitly free J1.
        fs::write(output.join("renamed-brief.json"), edited).unwrap();
        fs::write(
            output.join("layout.json"),
            r#"{"placer":{"constraints":"renamed-brief.json","free":["J1"]}}"#,
        )
        .unwrap();
        let free = import_kicad_netlist_with_options(&netlist, &output, "board", options).unwrap();
        let pcb = parse(&fs::read_to_string(&free.board).unwrap()).unwrap();
        let lock_refs: Vec<_> = board_parts(&pcb)
            .filter(|part| footprint_locked(part))
            .filter_map(footprint_reference)
            .collect();
        assert_eq!(lock_refs, ["J2"]);

        let too_few_layers = KiCadNetlistImportOptions {
            layers: Some(2),
            ..options
        };
        let refused = directory.join("refused");
        let error = import_kicad_netlist_with_options(&netlist, &refused, "board", too_few_layers)
            .unwrap_err();
        assert!(error.contains("In1.Cu"), "{error}");
        assert!(!refused.exists());
        let conflicting_donor = directory.join("conflict.kicad_pcb");
        fs::write(
            &conflicting_donor,
            fs::read_to_string(&donor).unwrap().replace(
                "(at 14 10) (locked yes)\n                (property \"Reference\" \"J2\")",
                "(at 14 10)\n                (property \"Reference\" \"J1\")",
            ),
        )
        .unwrap();
        let conflicting = KiCadNetlistImportOptions {
            mechanical_from: Some(&conflicting_donor),
            ..options
        };
        let mechanical = import_kicad_netlist_with_options(
            &netlist,
            &directory.join("conflicting"),
            "board",
            conflicting,
        )
        .unwrap()
        .mechanical_from
        .unwrap();
        assert_eq!(
            mechanical.conflicting_locks,
            ["J1: mechanical reference is ambiguous"]
        );
        assert!(mechanical.locks.is_empty());
        fs::remove_dir_all(directory).unwrap();
    }
}
