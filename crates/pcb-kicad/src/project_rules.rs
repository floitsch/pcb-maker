// Copyright (C) 2026 Toit contributors.

//! Routing rules resolved from the KiCad project file, so a board can be
//! routed without any hand-written or helper-generated configuration.

use super::*;

/// KiCad's built-in defaults for a project without net settings.
const DEFAULT_CLEARANCE: f64 = 0.2;
const DEFAULT_TRACK: f64 = 0.2;
const DEFAULT_VIA: f64 = 0.6;
const DEFAULT_DRILL: f64 = 0.3;

fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if let Some(position) = star {
            p = position + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

/// The most common value, in micrometres, of a list of dimensions.
fn most_common(values: &[f64]) -> Option<f64> {
    let mut counts = BTreeMap::<i64, usize>::new();
    for value in values {
        *counts.entry((value * 1000.0).round() as i64).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(value, count)| (*count, -*value))
        .map(|(value, _)| value as f64 / 1000.0)
}

/// Resolves per-connection rules for every net on `pcb` from the project
/// file next to it. Classes come from `net_settings`; nets are assigned by
/// explicit assignment, then by pattern, then to `Default`. Every value is
/// floored at the board's design-rule minimum. A project without net
/// settings takes its default geometry from the board's existing copper.
pub fn resolve_project_rules(
    project_path: &Path,
    pcb_path: &Path,
) -> Result<KiCadBoardRouterConfig, String> {
    let project: serde_json::Value = match fs::read_to_string(project_path) {
        Ok(source) => serde_json::from_str(&source)
            .map_err(|error| format!("failed to parse {}: {error}", project_path.display()))?,
        Err(_) => serde_json::Value::Null,
    };
    let pcb_source = fs::read_to_string(pcb_path)
        .map_err(|error| format!("failed to read {}: {error}", pcb_path.display()))?;
    let pcb = parse(&pcb_source)?;

    let rules = &project["board"]["design_settings"]["rules"];
    let minimum = |key: &str| rules[key].as_f64().unwrap_or(0.0);

    let mut widths = Vec::new();
    let mut via_sizes = Vec::new();
    let mut via_drills = Vec::new();
    for item in pcb.children() {
        match item.head() {
            Some("segment") => widths.extend(form_f64(item, "width", 1).ok()),
            Some("via") => {
                via_sizes.extend(form_f64(item, "size", 1).ok());
                via_drills.extend(form_f64(item, "drill", 1).ok());
            }
            _ => {}
        }
    }

    let mut classes = BTreeMap::<String, KiCadConnectionRoutingRules>::new();
    for class in project["net_settings"]["classes"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let Some(name) = class["name"].as_str() else {
            continue;
        };
        let value = |key: &str, fallback: f64| class[key].as_f64().filter(|v| *v > 0.0).unwrap_or(fallback);
        classes.insert(
            name.to_string(),
            KiCadConnectionRoutingRules {
                trace_width_mm: value("track_width", DEFAULT_TRACK),
                clearance_mm: value("clearance", DEFAULT_CLEARANCE),
                via_size_mm: value("via_diameter", DEFAULT_VIA),
                via_drill_mm: value("via_drill", DEFAULT_DRILL),
            },
        );
    }
    if !classes.contains_key("Default") {
        classes.insert(
            "Default".into(),
            KiCadConnectionRoutingRules {
                trace_width_mm: most_common(&widths).unwrap_or(DEFAULT_TRACK),
                clearance_mm: DEFAULT_CLEARANCE,
                via_size_mm: most_common(&via_sizes).unwrap_or(DEFAULT_VIA),
                via_drill_mm: most_common(&via_drills).unwrap_or(DEFAULT_DRILL),
            },
        );
    }
    for class in classes.values_mut() {
        class.trace_width_mm = class.trace_width_mm.max(minimum("min_track_width"));
        class.clearance_mm = class.clearance_mm.max(minimum("min_clearance"));
        class.via_size_mm = class.via_size_mm.max(minimum("min_via_diameter"));
        class.via_drill_mm = class.via_drill_mm.max(minimum("min_through_hole_diameter"));
        let annulus = minimum("min_via_annular_width").max(0.05);
        class.via_size_mm = class.via_size_mm.max(class.via_drill_mm + 2.0 * annulus);
    }

    let assignments = &project["net_settings"]["netclass_assignments"];
    let patterns: Vec<(String, String)> = project["net_settings"]["netclass_patterns"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some((
                entry["pattern"].as_str()?.to_string(),
                entry["netclass"].as_str()?.to_string(),
            ))
        })
        .collect();

    let mut nets = BTreeSet::<String>::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            if let Some(net) = node_net(pad) {
                nets.insert(net.to_string());
            }
        }
    }
    let mut connection_rules = BTreeMap::new();
    for raw in nets {
        let class = assignments[raw.as_str()]
            .as_str()
            .or_else(|| assignments[raw.as_str()][0].as_str())
            .map(str::to_owned)
            .or_else(|| {
                patterns
                    .iter()
                    .find(|(pattern, _)| wildcard_match(pattern, &raw))
                    .map(|(_, class)| class.clone())
            })
            .filter(|class| classes.contains_key(class))
            .unwrap_or_else(|| "Default".into());
        connection_rules.insert(normalize_net(&raw).to_string(), classes[&class].clone());
    }
    let narrowest = classes
        .values()
        .map(|class| class.trace_width_mm)
        .fold(f64::INFINITY, f64::min);
    Ok(KiCadBoardRouterConfig {
        neck_width_mm: Some(narrowest.max(minimum("min_track_width")).max(0.1)),
        connection_rules,
        default_rules: Some(classes["Default"].clone()),
        // KiCad's built-in board setup applies when the project is silent.
        edge_clearance_mm: rules["min_copper_edge_clearance"].as_f64().unwrap_or(0.5),
        hole_clearance_mm: rules["min_hole_clearance"].as_f64().unwrap_or(0.25),
        hole_to_hole_clearance_mm: rules["min_hole_to_hole"].as_f64().unwrap_or(0.25),
        ..KiCadBoardRouterConfig::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards_follow_kicad_pattern_semantics() {
        assert!(wildcard_match("GND", "GND"));
        assert!(wildcard_match("/GPIO*", "/GPIO18\\USB_D-"));
        assert!(wildcard_match("Net-(U?-Pad1)", "Net-(U3-Pad1)"));
        assert!(!wildcard_match("GND", "GNDA"));
        assert!(wildcard_match("*", ""));
    }
}
