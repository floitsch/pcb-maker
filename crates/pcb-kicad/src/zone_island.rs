// Copyright (C) 2026 Florian Loitsch.

//! A bounded, opt-in stitch for a single open between islands of a zone.

use super::*;
use serde_json::Value;

const MAX_TRIALS: usize = 6;

/// KiCad gives each end of an island open the same zone UUID.
fn island_endpoints(report: &Value) -> Option<(&str, [&str; 2])> {
    let [open] = report.get("unconnected_items")?.as_array()?.as_slice() else {
        return None;
    };
    let [a, b] = open.get("items")?.as_array()?.as_slice() else {
        return None;
    };
    let uuid = a.get("uuid")?.as_str()?;
    let descriptions = [
        a.get("description")?.as_str()?,
        b.get("description")?.as_str()?,
    ];
    (!uuid.is_empty()
        && b.get("uuid")?.as_str()? == uuid
        && descriptions
            .iter()
            .all(|description| description.starts_with("Zone ")))
    .then_some((uuid, descriptions))
}

pub(crate) fn zone_island_open(report: &Value) -> Option<&str> {
    let (uuid, descriptions) = island_endpoints(report)?;
    descriptions
        .iter()
        .all(|description| {
            description
                .rsplit_once(" on ")
                .is_some_and(|(_, layers)| layers.matches(".Cu").take(2).count() == 2)
        })
        .then_some(uuid)
}

pub(crate) fn single_layer_island_action(report: &Value) -> Option<String> {
    let (_, [description, other]) = island_endpoints(report)?;
    if description != other {
        return None;
    }
    let (_, layers) = description.rsplit_once(" on ")?;
    let layer = layers.split(", priority ").next()?;
    if layer != "F.Cu" && layer != "B.Cu" {
        let inner = layer
            .strip_prefix("In")?
            .strip_suffix(".Cu")?
            .parse::<u8>()
            .ok()?;
        if !(1..=30).contains(&inner) {
            return None;
        }
    }
    Some(format!(
        "KiCad reports two disconnected filled regions of {description}. Inspect the filled copper on {layer} for a gap caused by a competing pour, keepout, or track clearance. Connect the regions with copper on a permitted layer, then refill and check connectivity again."
    ))
}

pub(crate) const ZONE_ISLAND_ACTION: &str = "KiCad reports one open between islands of the same zone. Try drc-kicad-board with --stitch-zone-island <new-output-dir> to check whether one through via can connect them.";

pub fn zone_island_action(report: &Value) -> Option<&'static str> {
    zone_island_open(report).map(|_| ZONE_ISLAND_ACTION)
}

#[derive(Clone, Debug, serde::Deserialize, Serialize)]
pub struct KiCadZoneIslandRecovery {
    pub zone_uuid: String,
    pub net: String,
    pub retry_argv: Vec<String>,
}

/// Identify the exact returned zone from the native open. The reported
/// identical item positions do not identify individual filled polygons.
pub(crate) fn retry(
    report: &Value,
    pcb: &Expr,
    directory: &Path,
    board_id: &str,
) -> Option<KiCadZoneIslandRecovery> {
    let uuid = zone_island_open(report)?;
    if findings_at_report_cap(report) || directory.file_name().is_none() {
        return None;
    }
    let (_, _, net) = named_zone(pcb, uuid)?;
    let (baseline, output) = (0..=99).find_map(|index| {
        let suffix = if index == 0 {
            String::new()
        } else {
            format!("-{index}")
        };
        let baseline = directory.with_extension(format!("zone-island-before{suffix}.json"));
        let output = directory.with_extension(format!("zone-island-repair{suffix}"));
        (fs::symlink_metadata(&baseline).is_err() && fs::symlink_metadata(&output).is_err())
            .then_some((baseline, output))
    })?;
    Some(KiCadZoneIslandRecovery {
        zone_uuid: uuid.to_string(),
        net: unescape(&net).into_owned(),
        retry_argv: vec![
            "pcb-maker".into(),
            "drc-kicad-board".into(),
            directory.display().to_string(),
            board_id.into(),
            baseline.display().to_string(),
            "--stitch-zone-island".into(),
            output.display().to_string(),
        ],
    })
}

#[derive(Debug, Serialize)]
pub struct KiCadZoneIslandRepair {
    pub source: PathBuf,
    pub source_sha256: String,
    pub zone: Option<String>,
    pub net: Option<String>,
    pub attempts: Vec<KiCadZoneIslandTrial>,
    pub kept: bool,
    pub reason: &'static str,
}

#[derive(Debug, Serialize)]
pub struct KiCadZoneIslandTrial {
    pub at_mm: [f64; 2],
    pub intersecting_layers: [String; 2],
    pub diameter_mm: f64,
    pub drill_mm: f64,
    pub unconnected: usize,
    pub new_findings: Vec<String>,
}

struct Fill<'a> {
    layer: &'a str,
    points: Vec<[f64; 2]>,
    minimum: [f64; 2],
    maximum: [f64; 2],
    area: f64,
}

fn polygon_points(polygon: &Expr) -> Option<Vec<[f64; 2]>> {
    let points: Vec<[f64; 2]> = polygon
        .child("pts")?
        .children()
        .iter()
        .filter(|point| point.head() == Some("xy"))
        .map(|point| {
            let fields = point.children();
            Some([
                fields.get(1)?.atom()?.parse().ok()?,
                fields.get(2)?.atom()?.parse().ok()?,
            ])
        })
        .collect::<Option<_>>()?;
    (points.len() >= 3 && points.iter().flatten().all(|n| n.is_finite())).then_some(points)
}

impl<'a> Fill<'a> {
    fn from_expr(polygon: &'a Expr) -> Option<Self> {
        let layer = form_atom(polygon, "layer", 1)?;
        if !layer.ends_with(".Cu") {
            return None;
        }
        let points = polygon_points(polygon)?;
        let mut minimum = [f64::INFINITY; 2];
        let mut maximum = [f64::NEG_INFINITY; 2];
        for point in &points {
            for axis in 0..2 {
                minimum[axis] = minimum[axis].min(point[axis]);
                maximum[axis] = maximum[axis].max(point[axis]);
            }
        }
        let area = 0.5
            * points
                .iter()
                .zip(points.iter().cycle().skip(1))
                .map(|(a, b)| a[0] * b[1] - a[1] * b[0])
                .sum::<f64>()
                .abs();
        Some(Self {
            layer,
            points,
            minimum,
            maximum,
            area,
        })
    }

    fn contains(&self, at: [f64; 2]) -> bool {
        (0..2).all(|axis| at[axis] >= self.minimum[axis] && at[axis] <= self.maximum[axis])
            && point_in_polygon(at, &self.points)
    }

    fn admits(&self, at: [f64; 2], radius: f64) -> bool {
        self.contains(at) && clears_polygon_edges(at, radius, &self.points)
    }
}

struct Pad {
    at: [f64; 2],
    radius: f64,
    plated: bool,
    same_net: bool,
}

/// Avoid new vias in pads, even where the DRC would accept one. A pad's
/// circumscribed circle is conservative for rectangular pads.
fn pads(board: &Expr, net: &str, codes: &BTreeMap<String, String>) -> Vec<Pad> {
    let mut pads = Vec::new();
    for footprint in board
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let Ok([x, y, angle]) = form_at(footprint) else {
            continue;
        };
        for pad in footprint
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
        {
            let (Ok([width, height]), Ok([dx, dy, _])) = (form_xy(pad, "size"), form_at(pad))
            else {
                continue;
            };
            let offset = rotate_vector([dx, dy], -angle);
            pads.push(Pad {
                at: [x + offset[0], y + offset[1]],
                radius: width.hypot(height) / 2.0,
                plated: pad.children().get(2).and_then(Expr::atom) == Some("thru_hole"),
                same_net: item_net(pad, codes) == Some(net),
            });
        }
    }
    pads
}

fn clears_polygon_edges(at: [f64; 2], radius: f64, polygon: &[[f64; 2]]) -> bool {
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .all(|(a, b)| point_segment_distance_squared(at, *a, *b) > radius * radius)
}

fn sites(zone: &Expr, board: &Expr, net: &str, radius: f64) -> Vec<([f64; 2], [String; 2])> {
    let fills: Vec<Fill<'_>> = zone
        .children()
        .iter()
        .filter(|item| item.head() == Some("filled_polygon"))
        .filter_map(Fill::from_expr)
        .collect();
    let codes = net_codes(board);
    let pads = pads(board, net, &codes);
    let mut contacts: Vec<[f64; 2]> = pads
        .iter()
        .filter(|pad| pad.plated && pad.same_net)
        .map(|pad| pad.at)
        .collect();
    let vias: Vec<_> = board
        .children()
        .iter()
        .filter(|item| item.head() == Some("via"))
        .collect();
    contacts.extend(
        vias.iter()
            .filter(|item| {
                item_net(item, &codes) == Some(net)
                    && form_atom(item, "layers", 1) == Some("F.Cu")
                    && form_atom(item, "layers", 2) == Some("B.Cu")
            })
            .filter_map(|via| form_xy(via, "at").ok()),
    );
    let via_sites: Vec<_> = vias
        .iter()
        .filter_map(|via| {
            Some((
                form_xy(via, "at").ok()?,
                form_f64(via, "size", 1).ok()? / 2.0,
            ))
        })
        .collect();
    let keepouts: Vec<_> = board
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone"))
        .filter(|item| {
            item.child("keepout")
                .and_then(|keepout| form_atom(keepout, "vias", 1))
                == Some("not_allowed")
        })
        .flat_map(|item| {
            item.children()
                .iter()
                .filter(|child| child.head() == Some("polygon"))
        })
        .filter_map(polygon_points)
        .collect();
    let fractions = [0.5, 0.25, 0.75, 0.375, 0.625, 0.125, 0.875];
    let mut by_island = Vec::new();
    for fill in &fills {
        // Favor the smaller pieces with no existing plated contact.
        if !fills
            .iter()
            .any(|other| other.layer == fill.layer && other.area > fill.area)
            || contacts.iter().any(|&at| fill.contains(at))
        {
            continue;
        }
        let mut candidates = Vec::new();
        for fx in fractions {
            for fy in fractions {
                let at = std::array::from_fn(|axis| {
                    let fraction = [fx, fy][axis];
                    let value =
                        fill.minimum[axis] + fraction * (fill.maximum[axis] - fill.minimum[axis]);
                    (value * 1e6).round() / 1e6
                });
                let Some(other) = fills
                    .iter()
                    .find(|other| other.layer != fill.layer && other.admits(at, radius))
                else {
                    continue;
                };
                if fill.admits(at, radius)
                    && pads.iter().all(|pad| {
                        (pad.at[0] - at[0]).hypot(pad.at[1] - at[1]) > pad.radius + radius
                    })
                    && via_sites.iter().all(|(old, old_radius)| {
                        (old[0] - at[0]).hypot(old[1] - at[1]) > old_radius + radius
                    })
                    && keepouts.iter().all(|polygon| {
                        !point_in_polygon(at, polygon) && clears_polygon_edges(at, radius, polygon)
                    })
                {
                    candidates.push((at, [fill.layer.to_string(), other.layer.to_string()]));
                    if candidates.len() == MAX_TRIALS {
                        break;
                    }
                }
            }
            if candidates.len() == MAX_TRIALS {
                break;
            }
        }
        if !candidates.is_empty() {
            by_island.push(candidates);
        }
    }
    // One site on each likely island before spending a check on a second.
    (0..MAX_TRIALS)
        .flat_map(|index| {
            by_island
                .iter()
                .filter_map(move |island| island.get(index).cloned())
        })
        .take(MAX_TRIALS)
        .collect()
}

type NativeFinding<'a> = (&'a str, &'a str, &'a str, Vec<String>);

fn findings(report: &Value) -> Result<Vec<NativeFinding<'_>>, String> {
    let malformed = || "KiCad DRC report has a malformed finding".to_string();
    let values = report["violations"]
        .as_array()
        .ok_or_else(|| "KiCad DRC report has no violations array".to_string())?;
    let mut entries = Vec::new();
    for finding in values {
        let kind = finding["type"].as_str().ok_or_else(malformed)?;
        let severity = finding["severity"].as_str().ok_or_else(malformed)?;
        if verdict::is_cosmetic(kind)
            || finding["excluded"].as_bool() == Some(true)
            || severity == "exclusion"
        {
            continue;
        }
        if !matches!(severity, "warning" | "error") {
            return Err(malformed());
        }
        let description = finding["description"].as_str().ok_or_else(malformed)?;
        let mut items = Vec::new();
        for item in finding["items"].as_array().ok_or_else(malformed)? {
            let uuid = item["uuid"].as_str().ok_or_else(malformed)?;
            let x = item["pos"]["x"].as_f64().ok_or_else(malformed)?;
            let y = item["pos"]["y"].as_f64().ok_or_else(malformed)?;
            items.push(format!("{uuid} {x} {y}"));
        }
        items.sort();
        entries.push((kind, severity, description, items));
    }
    Ok(entries)
}

fn new_findings(before: &Value, after: &Value) -> Result<Vec<String>, String> {
    let mut before = findings(before)?;
    let mut new = Vec::new();
    for finding in findings(after)? {
        if let Some(index) = before.iter().position(|older| *older == finding) {
            before.swap_remove(index);
        } else {
            new.push(finding.0.to_string());
        }
    }
    Ok(new)
}

fn findings_at_report_cap(report: &Value) -> bool {
    let mut by_kind = BTreeMap::<&str, usize>::new();
    report["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|finding| {
            let Some(kind) = finding["type"]
                .as_str()
                .filter(|kind| !verdict::is_cosmetic(kind))
            else {
                return false;
            };
            let count = by_kind.entry(kind).or_default();
            *count += 1;
            *count >= verdict::KICAD_FINDING_CAP
        })
}

fn with_via(
    source: &str,
    zone_net: &Expr,
    net: &str,
    at: [f64; 2],
    diameter: f64,
    drill: f64,
) -> Result<String, String> {
    let end = source
        .trim_end()
        .len()
        .checked_sub(1)
        .filter(|&end| source.as_bytes()[end] == b')')
        .ok_or_else(|| "no final ')' in the KiCad board".to_string())?;
    let uuid = deterministic_uuid_for(&format!(
        "pcb-maker:zone-stitch:{net}:{at:?}:{diameter}:{drill}"
    ));
    Ok(format!(
        "{}\n\t(via (at {} {}) (size {diameter}) (drill {drill}) (layers \"F.Cu\" \"B.Cu\") {} (uuid \"{uuid}\"))\n{}",
        &source[..end],
        at[0],
        at[1],
        encode(zone_net),
        &source[end..]
    ))
}

fn zone<'a>(pcb: &'a Expr, uuid: &str) -> Option<&'a Expr> {
    let mut matches = pcb.children().iter().filter(|item| {
        item.head() == Some("zone")
            && (form_atom(item, "uuid", 1) == Some(uuid)
                || form_atom(item, "tstamp", 1) == Some(uuid))
            && item.child("keepout").is_none()
    });
    let found = matches.next()?;
    matches.next().is_none().then_some(found)
}

fn named_zone<'a>(pcb: &'a Expr, uuid: &str) -> Option<(&'a Expr, &'a Expr, String)> {
    let zone = zone(pcb, uuid)?;
    let net_form = zone.child("net")?;
    let codes = net_codes(pcb);
    let net = item_net(zone, &codes)?;
    if net.is_empty()
        || net_form.children().len() != 2
        || form_atom(zone, "net_name", 1).is_some_and(|label| label != net)
    {
        return None;
    }
    Some((zone, net_form, net.to_owned()))
}

fn copy_sidecars(source: &Path, output: &Path, board_id: &str) -> Result<(), String> {
    for name in [
        format!("{board_id}.kicad_pro"),
        format!("{board_id}.kicad_dru"),
    ] {
        let from = source.join(&name);
        if from.is_file() {
            fs::copy(&from, output.join(&name))
                .map_err(|error| format!("failed to copy {}: {error}", from.display()))?;
        }
    }
    Ok(())
}

fn check_output(source: &Path, output: &Path) -> Result<(), String> {
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let source_dir = fs::canonicalize(source).map_err(|error| error.to_string())?;
    let output_parent = fs::canonicalize(parent).map_err(|error| error.to_string())?;
    if output_parent.starts_with(source_dir) {
        return Err("place the repair output outside the source project".into());
    }
    if fs::symlink_metadata(output).is_ok() {
        return Err(format!(
            "repair output already exists: {}",
            output.display()
        ));
    }
    Ok(())
}

fn publish(
    source: &Path,
    output: &Path,
    board_id: &str,
    candidate: &str,
    report: &Path,
    result: &KiCadZoneIslandRepair,
) -> Result<(), String> {
    let stage = temporary_next_to(output);
    let published = (|| {
        copy_directory_tree(source, &stage)?;
        native_report::remove_stale_board_run_reports(&stage)?;
        fs::write(stage.join(format!("{board_id}.kicad_pcb")), candidate)
            .map_err(|error| error.to_string())?;
        fs::copy(report, stage.join("drc.json")).map_err(|error| error.to_string())?;
        fs::write(
            stage.join("zone-island-repair.json"),
            serde_json::to_vec_pretty(&result).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        // Reserve the name without overwriting another writer's output.
        fs::create_dir(output)
            .map_err(|error| format!("failed to create {}: {error}", output.display()))?;
        if let Err(error) = fs::rename(&stage, output) {
            let _ = fs::remove_dir(output);
            return Err(format!("failed to publish {}: {error}", output.display()));
        }
        Ok(())
    })();
    if published.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    published
}

/// Try one additive through via, keeping it only on a strictly better
/// refilled KiCad report. The caller provides its baseline DRC report.
pub fn stitch_zone_island(
    source: &Path,
    board_id: &str,
    before: &Value,
    output: &Path,
) -> Result<KiCadZoneIslandRepair, String> {
    check_board_id(board_id)?;
    check_output(source, output)?;
    let board_path = source.join(format!("{board_id}.kicad_pcb"));
    let original = fs::read_to_string(&board_path)
        .map_err(|error| format!("failed to read {}: {error}", board_path.display()))?;
    let mut result = KiCadZoneIslandRepair {
        source: board_path.clone(),
        source_sha256: format!("{:x}", Sha256::digest(original.as_bytes())),
        zone: None,
        net: None,
        attempts: Vec::new(),
        kept: false,
        reason: "KiCad did not report a single open between islands of one zone",
    };
    let Some(uuid) = zone_island_open(before) else {
        return Ok(result);
    };
    result.zone = Some(uuid.to_string());
    if findings_at_report_cap(before) {
        result.reason = "KiCad capped findings at baseline, so new findings cannot be ruled out";
        return Ok(result);
    }
    let pcb = parse(&original)?;
    let Some((_, zone_net, net)) = named_zone(&pcb, uuid) else {
        result.reason = "the reported zone has no named net on this board";
        return Ok(result);
    };
    result.net = Some(unescape(&net).into_owned());
    let config = resolve_project_rules(&source.join(format!("{board_id}.kicad_pro")), &board_path)?;
    let names = NetNames::of(&pcb);
    let Some(rules) = config
        .connection_rules
        .get(names.normalize(&net))
        .or(config.default_rules.as_ref())
    else {
        result.reason = "the project has no resolved via geometry";
        return Ok(result);
    };
    let diameter = rules.via_size_mm;
    let drill = rules.via_drill_mm;
    if !diameter.is_finite() || !drill.is_finite() || drill <= 0.0 || diameter <= drill {
        result.reason = "the project's resolved through via geometry is invalid";
        return Ok(result);
    }
    let scratch = temporary_next_to(&std::env::temp_dir().join("pcb-maker-zone-stitch"));
    fs::create_dir(&scratch)
        .map_err(|error| format!("failed to create repair scratch directory: {error}"))?;
    let repaired = (|| {
        copy_sidecars(source, &scratch, board_id)?;
        let trial_board = scratch.join(format!("{board_id}.kicad_pcb"));
        fs::write(&trial_board, &original).map_err(|error| error.to_string())?;
        let native = scratch.join("refill.json");
        native_report::run_kicad_save_board(&trial_board, &native)?;
        if zone_island_open(&read_json(&native)?) != Some(uuid) {
            result.reason = "KiCad's fresh refill did not reproduce the reported open";
            return Ok(result);
        }
        let filled = parse(&fs::read_to_string(&trial_board).map_err(|error| error.to_string())?)?;
        copy_sidecars(source, &scratch, board_id)?;
        let Some((filled_zone, _, _)) =
            named_zone(&filled, uuid).filter(|(_, _, filled_net)| filled_net == &net)
        else {
            result.reason = "the reported zone is absent from KiCad's refill";
            return Ok(result);
        };
        result.reason = "no overlapping fill sites with room for a through via";
        let report_path = scratch.join("trial.json");
        for (at, intersecting_layers) in sites(filled_zone, &pcb, &net, diameter / 2.0) {
            let candidate = with_via(&original, zone_net, &net, at, diameter, drill)?;
            fs::write(&trial_board, &candidate).map_err(|error| error.to_string())?;
            let after = kicad_drc_report(&scratch, board_id, &report_path, true)?;
            let unconnected = after
                .get("unconnected_items")
                .and_then(Value::as_array)
                .map(Vec::len)
                .ok_or_else(|| "KiCad DRC report has no unconnected_items array".to_string())?;
            let new = new_findings(before, &after)?;
            let kept = unconnected == 0 && new.is_empty();
            result.attempts.push(KiCadZoneIslandTrial {
                at_mm: at,
                intersecting_layers,
                diameter_mm: diameter,
                drill_mm: drill,
                unconnected,
                new_findings: new,
            });
            result.reason = "KiCad rejected every tested stitch or the island stayed open";
            if kept {
                result.kept = true;
                result.reason =
                    "KiCad found zero opens and no new findings after adding one through via";
                publish(source, output, board_id, &candidate, &report_path, &result)?;
                break;
            }
        }
        Ok(result)
    })();
    let _ = fs::remove_dir_all(&scratch);
    repaired
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_legacy_zone_uses_the_board_net_table_and_keeps_its_via_encoding() {
        let legacy = r#"(kicad_pcb (net 5 "GND")
            (zone (tstamp "zone") (net 5) (net_name "GND") (layers "F.Cu" "B.Cu")))"#;
        let board = parse(legacy).unwrap();
        let (_, via_net, named) = named_zone(&board, "zone").unwrap();
        assert_eq!(named, "GND");
        let candidate = with_via(legacy, via_net, &named, [1.0, 2.0], 0.6, 0.3).unwrap();
        let added = parse(&candidate).unwrap();
        let via = added.child("via").unwrap();
        assert_eq!(encode(via.child("net").unwrap()), "(net 5)");
        assert_eq!(item_net(via, &net_codes(&added)), Some("GND"));
        assert!(candidate.starts_with(&legacy[..legacy.len() - 1]));

        let item = json!({"uuid":"zone","description":"Zone [GND] on F.Cu, B.Cu, priority 0"});
        let report = json!({"unconnected_items":[{"items":[item,item]}],"violations":[]});
        let directory = Path::new("run/result");
        let recovery = retry(&report, &board, directory, "test").unwrap();
        assert_eq!(recovery.net, "GND");
        assert_eq!(recovery.zone_uuid, "zone");
        assert_eq!(
            recovery.retry_argv,
            [
                "pcb-maker",
                "drc-kicad-board",
                "run/result",
                "test",
                "run/result.zone-island-before.json",
                "--stitch-zone-island",
                "run/result.zone-island-repair"
            ]
        );
        for changed in [
            legacy.replace(r#"(net 5 "GND")"#, ""),
            legacy.replace(r#"(net_name "GND")"#, r#"(net_name "OTHER")"#),
            legacy.replace(
                r#"(layers "F.Cu" "B.Cu")))"#,
                r#"(layers "F.Cu" "B.Cu")) (zone (uuid "zone") (net "GND")))"#,
            ),
        ] {
            assert!(retry(&report, &parse(&changed).unwrap(), directory, "test").is_none());
        }
    }

    #[test]
    fn the_action_requires_two_ends_of_one_zone_and_no_other_open() {
        let a = json!({"description":"Zone 'GND' [GND] on F.Cu and B.Cu", "uuid":"one"});
        let b = json!({"description":"Zone 'GND' [GND] on F.Cu and B.Cu", "uuid":"two"});
        let pad = json!({"description":"Pad 1 [GND] of U1 on F.Cu", "uuid":"one"});
        let report = |a: &Value, b: &Value| json!({"unconnected_items":[{"items":[a,b]}]});
        assert_eq!(zone_island_open(&report(&a, &a)), Some("one"));
        assert_eq!(zone_island_open(&report(&a, &b)), None);
        assert_eq!(zone_island_open(&report(&a, &pad)), None);
        let inner =
            json!({"description":"Zone 'prefuse' [H_PWR] on In2.Cu, priority 0", "uuid":"one"});
        let single = report(&inner, &inner);
        let advice = single_layer_island_action(&single).unwrap();
        assert!(advice.contains("Zone 'prefuse' [H_PWR] on In2.Cu"));
        assert!(zone_island_action(&single).is_none());
        assert!(single_layer_island_action(&report(&a, &a)).is_none());
        assert!(single_layer_island_action(&report(&inner, &pad)).is_none());
        let mut two = report(&a, &a);
        two["unconnected_items"]
            .as_array_mut()
            .unwrap()
            .push(json!({"items":[a,a]}));
        assert_eq!(zone_island_open(&two), None);
    }

    #[test]
    fn candidates_cross_two_fills_and_avoid_via_keepouts() {
        let fill = parse(
            r#"(zone (net "GND")
          (filled_polygon (layer "F.Cu") (pts (xy 0 0) (xy 10 0) (xy 10 10) (xy 0 10)))
          (filled_polygon (layer "B.Cu") (pts (xy 0 0) (xy 4 0) (xy 4 10) (xy 0 10)))
          (filled_polygon (layer "B.Cu") (pts (xy 6 2) (xy 8 2) (xy 8 4) (xy 6 4))))"#,
        )
        .unwrap();
        let bare = parse("(kicad_pcb)").unwrap();
        let sites_without_keepout = sites(&fill, &bare, "GND", 0.2);
        assert_eq!(
            sites_without_keepout[0],
            ([7.0, 3.0], ["B.Cu".into(), "F.Cu".into()])
        );
        let kept_out = parse(
            r#"(kicad_pcb (zone (layer "B.Cu")
          (keepout (vias not_allowed))
          (polygon (pts (xy 5 1) (xy 9 1) (xy 9 5) (xy 5 5)))))"#,
        )
        .unwrap();
        assert!(sites(&fill, &kept_out, "GND", 0.2).is_empty());
    }

    #[test]
    fn an_exchanged_error_of_the_same_type_rejects_a_stitch() {
        let report = |id: &str, severity| {
            json!({"violations":[{
                "type":"clearance", "severity":severity, "description":"Clearance violation",
                "items":[{"uuid":id,"pos":{"x":0.0,"y":0.0}}]
            }]})
        };
        let before = report("old", "error");
        assert!(new_findings(&before, &before).unwrap().is_empty());
        assert_eq!(
            new_findings(&before, &report("via", "error")).unwrap(),
            ["clearance"]
        );
        assert_eq!(
            new_findings(&before, &report("via", "warning")).unwrap(),
            ["clearance"]
        );
        assert!(new_findings(&before, &json!({"violations":[{"type":"clearance"}]})).is_err());
    }

    #[test]
    fn an_unsupported_open_leaves_the_source_and_destination_alone() {
        let source = temporary_next_to(&std::env::temp_dir().join("zone-island-test"));
        fs::create_dir(&source).unwrap();
        let board = source.join("test.kicad_pcb");
        let original = "(kicad_pcb)\n";
        fs::write(&board, original).unwrap();
        let output = source.with_extension("repaired");
        let result =
            stitch_zone_island(&source, "test", &json!({"unconnected_items":[]}), &output).unwrap();
        assert!(!result.kept);
        assert!(result.attempts.is_empty());
        assert!(!output.exists());
        assert_eq!(fs::read_to_string(board).unwrap(), original);
        fs::remove_dir_all(source).unwrap();
    }

    #[test]
    fn a_native_zone_island_stitch_adds_only_one_via() {
        if check_kicad_cli().is_err() {
            eprintln!("KiCad 10 is unavailable");
            return;
        }
        let source = temporary_next_to(&std::env::temp_dir().join("native-zone-island-test"));
        fs::create_dir(&source).unwrap();
        let original = r#"(kicad_pcb (version 20260206) (generator "pcbnew")
  (general (thickness 1.6))
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user)
    (5 "F.SilkS" user) (7 "B.SilkS" user))
  (footprint "Fixture:J1" (layer "F.Cu") (at 6 5)
    (property "Reference" "J1" (at 0 0) (layer "F.SilkS")
      (effects (font (size 1 1) (thickness 0.15))))
    (pad "1" thru_hole circle (at 0 0) (size 0.8 0.8) (drill 0.4)
      (layers "*.Cu") (net "GND")
      (uuid "150fd7f2-966a-4d9c-9416-de787638c6fd")))
  (footprint "Fixture:J2" (layer "B.Cu") (at 35 15)
    (property "Reference" "J2" (at 0 0) (layer "B.SilkS")
      (effects (font (size 1 1) (thickness 0.15))))
    (pad "1" smd circle (at 0 0) (size 0.8 0.8) (layers "B.Cu")
      (net "GND") (uuid "e4196f38-8428-4e6f-9bd0-1b4536320dfc")))
  (gr_rect (start 0 0) (end 40 20) (stroke (width 0.05) (type solid))
    (fill no) (layer "Edge.Cuts") (uuid "97481663-205f-4713-88ad-1e0b2f1f5984"))
  (zone (net "GND") (layers "F.Cu" "B.Cu")
    (uuid "605d5b9a-7b33-4479-b8c2-901b63ccd311")
    (name "GND") (hatch edge 0.5) (connect_pads yes (clearance 0.5))
    (min_thickness 0.2)
    (fill (thermal_gap 0.5) (thermal_bridge_width 0.5) (island_removal_mode 1))
    (polygon (pts (xy 0.5 0.5) (xy 39.5 0.5) (xy 39.5 19.5) (xy 0.5 19.5))))
  (zone (layer "B.Cu") (uuid "728190b9-18a8-44bf-9acc-130c134e428a")
    (hatch edge 0.5) (connect_pads (clearance 0)) (min_thickness 0.25)
    (keepout (tracks not_allowed) (vias not_allowed) (pads not_allowed)
      (copperpour not_allowed) (footprints allowed))
    (fill (thermal_gap 0.5) (thermal_bridge_width 0.5) (island_removal_mode 0))
    (polygon (pts (xy 23 -1) (xy 25 -1) (xy 25 21) (xy 23 21)))))
"#;
        let board = source.join("fixture.kicad_pcb");
        fs::write(&board, original).unwrap();
        let project = source.join("fixture.kicad_pro");
        fs::write(
            &project,
            r#"{"board":{"design_settings":{"rules":{"min_clearance":0.1,"min_track_width":0.1}}},
            "net_settings":{"classes":[{"name":"Default","clearance":0.2,"track_width":0.2,
            "via_diameter":0.6,"via_drill":0.3}]}}"#,
        )
        .unwrap();
        fs::create_dir(source.join("sheets")).unwrap();
        fs::write(source.join("sheets/supply.kicad_sch"), "source sheet").unwrap();
        fs::write(source.join("board-router.json"), "stale run report").unwrap();
        let before = kicad_drc_report(&source, "fixture", &source.join("drc.json"), true).unwrap();
        let out = source.with_extension("repaired");
        let repair = stitch_zone_island(&source, "fixture", &before, &out).unwrap();
        assert!(repair.kept, "{repair:?}");
        assert_eq!(repair.net.as_deref(), Some("GND"));
        let repaired = fs::read_to_string(out.join("fixture.kicad_pcb")).unwrap();
        let end = original.trim_end().len() - 1;
        assert_eq!(&repaired[..end], &original[..end]);
        let before_board = parse(original).unwrap();
        let mut after_board = parse(&repaired).unwrap();
        let Expr::List(items) = &mut after_board else {
            unreachable!()
        };
        let via = items.pop().unwrap();
        assert_eq!(via.head(), Some("via"));
        assert_eq!(node_net(&via), Some("GND"));
        assert_eq!(form_f64(&via, "size", 1).unwrap(), 0.6);
        assert_eq!(form_f64(&via, "drill", 1).unwrap(), 0.3);
        assert_eq!(after_board, before_board);
        assert_eq!(
            read_json(&out.join("drc.json")).unwrap()["unconnected_items"],
            json!([])
        );
        assert_eq!(
            fs::read_to_string(out.join("sheets/supply.kicad_sch")).unwrap(),
            "source sheet"
        );
        assert!(!out.join("board-router.json").exists());
        assert_eq!(fs::read_to_string(board).unwrap(), original);
        let bad_out = source.with_extension("failed");
        assert!(
            publish(
                &source,
                &bad_out,
                "fixture",
                &repaired,
                &source.join("missing.json"),
                &repair,
            )
            .is_err()
        );
        assert!(!bad_out.exists());
        fs::remove_dir_all(source).unwrap();
        fs::remove_dir_all(out).unwrap();
    }
}
