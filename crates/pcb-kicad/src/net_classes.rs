// Copyright (C) 2026 Toit contributors.

//! Net classes asked for in the router config (`net_classes`): track width,
//! clearance and via size for named nets. They set the router's rules for
//! those nets and are written into the project's `.kicad_pro`, so KiCad and
//! its DRC use the same rules.

use super::*;
use crate::project_rules::wildcard_match;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadNetClassRequest {
    pub name: String,
    /// Net names or KiCad wildcard patterns (`*`, `?`), with or without the
    /// sheet path's leading `/`.
    pub nets: Vec<String>,
    #[serde(default)]
    pub track_width_mm: Option<f64>,
    #[serde(default)]
    pub clearance_mm: Option<f64>,
    #[serde(default)]
    pub via_diameter_mm: Option<f64>,
    #[serde(default)]
    pub via_drill_mm: Option<f64>,
}

/// The board's nets (raw names) each class takes; a net goes to the first
/// class naming it. A pattern that matches no net is an error.
fn assign(pcb: &Expr, classes: &[KiCadNetClassRequest]) -> Result<Vec<Vec<String>>, String> {
    let mut nets = BTreeSet::new();
    for footprint in pcb.children().iter().filter(|item| item.head() == Some("footprint")) {
        for pad in footprint.children().iter().filter(|child| child.head() == Some("pad")) {
            if let Some(net) = node_net(pad) {
                nets.insert(net.to_string());
            }
        }
    }
    let mut taken = BTreeSet::new();
    let mut assigned = Vec::new();
    for class in classes {
        if class.name.is_empty() || class.name == "Default" {
            return Err(format!("net class name {:?} is reserved or empty", class.name));
        }
        for value in [class.track_width_mm, class.clearance_mm, class.via_diameter_mm, class.via_drill_mm] {
            if value.is_some_and(|value| !(value > 0.0)) {
                return Err(format!("net class {}: sizes must be positive", class.name));
            }
        }
        let mut members = Vec::new();
        for pattern in &class.nets {
            let matched: Vec<&String> = nets
                .iter()
                .filter(|net| wildcard_match(pattern, net) || wildcard_match(pattern, normalize_net(net)))
                .collect();
            if matched.is_empty() {
                return Err(format!("net class {}: {pattern:?} matches no net", class.name));
            }
            for net in matched {
                if taken.insert(net.clone()) {
                    members.push(net.clone());
                }
            }
        }
        assigned.push(members);
    }
    Ok(assigned)
}

/// `config` with the rules of the requested classes' nets replaced.
pub(super) fn with_net_classes(config: &KiCadBoardRouterConfig, pcb: &Expr) -> Result<KiCadBoardRouterConfig, String> {
    let mut config = config.clone();
    let assigned = assign(pcb, &config.net_classes)?;
    for (class, members) in config.net_classes.clone().iter().zip(assigned) {
        for net in members {
            let key = normalize_net(&net).to_string();
            let mut rules = config
                .connection_rules
                .get(&key)
                .or(config.default_rules.as_ref())
                .cloned()
                .unwrap_or(KiCadConnectionRoutingRules {
                    trace_width_mm: 0.2,
                    clearance_mm: 0.2,
                    via_size_mm: 0.6,
                    via_drill_mm: 0.3,
                });
            rules.trace_width_mm = class.track_width_mm.unwrap_or(rules.trace_width_mm);
            rules.clearance_mm = class.clearance_mm.unwrap_or(rules.clearance_mm);
            rules.via_size_mm = class.via_diameter_mm.unwrap_or(rules.via_size_mm);
            rules.via_drill_mm = class.via_drill_mm.unwrap_or(rules.via_drill_mm);
            config.connection_rules.insert(key, rules);
        }
    }
    Ok(config)
}

/// Writes the classes into `<directory>/<board_id>.kicad_pro` (created if
/// missing): each class with the exact names of its nets as patterns, ahead
/// of the project's own classes.
pub(super) fn write_net_classes(
    directory: &Path,
    board_id: &str,
    pcb: &Expr,
    classes: &[KiCadNetClassRequest],
) -> Result<(), String> {
    let assigned = assign(pcb, classes)?;
    let path = directory.join(format!("{board_id}.kicad_pro"));
    let mut project: serde_json::Value = match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|error| format!("failed to parse {}: {error}", path.display()))?,
        Err(_) => serde_json::json!({"meta": {"filename": format!("{board_id}.kicad_pro"), "version": 1}}),
    };
    if !project["net_settings"].is_object() {
        project["net_settings"] = serde_json::json!({"meta": {"version": 3}});
    }
    let settings = &mut project["net_settings"];
    let mut existing: Vec<serde_json::Value> = settings["classes"].as_array().cloned().unwrap_or_default();
    existing.retain(|entry| !classes.iter().any(|class| entry["name"] == class.name.as_str()));
    let template = existing
        .iter()
        .find(|entry| entry["name"] == "Default")
        .cloned()
        .unwrap_or_else(|| {
            serde_json::json!({
                "name": "Default", "clearance": 0.2, "track_width": 0.2, "via_diameter": 0.6, "via_drill": 0.3,
                "microvia_diameter": 0.3, "microvia_drill": 0.1, "diff_pair_gap": 0.25, "diff_pair_via_gap": 0.25,
                "diff_pair_width": 0.2, "bus_width": 12, "wire_width": 6, "line_style": 0,
                "pcb_color": "rgba(0, 0, 0, 0.000)", "schematic_color": "rgba(0, 0, 0, 0.000)",
                "priority": 2147483647
            })
        });
    if !existing.iter().any(|entry| entry["name"] == "Default") {
        existing.push(template.clone());
    }
    // Ours first: KiCad takes a net's rules from its highest-priority class
    // (the smallest number).
    for entry in existing.iter_mut().filter(|entry| entry["name"] != "Default") {
        if let Some(priority) = entry["priority"].as_i64() {
            entry["priority"] = (priority + classes.len() as i64).into();
        }
    }
    let mut result = Vec::new();
    for (index, class) in classes.iter().enumerate() {
        let mut entry = template.clone();
        entry["name"] = class.name.clone().into();
        entry["priority"] = (index as i64).into();
        for (key, value) in [
            ("track_width", class.track_width_mm),
            ("clearance", class.clearance_mm),
            ("via_diameter", class.via_diameter_mm),
            ("via_drill", class.via_drill_mm),
        ] {
            if let Some(value) = value {
                entry[key] = value.into();
            }
        }
        result.push(entry);
    }
    result.extend(existing);
    settings["classes"] = result.into();
    let mut patterns: Vec<serde_json::Value> = settings["netclass_patterns"].as_array().cloned().unwrap_or_default();
    patterns.retain(|entry| !classes.iter().any(|class| entry["netclass"] == class.name.as_str()));
    let mut ours = Vec::new();
    for (class, members) in classes.iter().zip(assigned) {
        for net in members {
            ours.push(serde_json::json!({"netclass": class.name, "pattern": net}));
        }
    }
    ours.extend(patterns);
    settings["netclass_patterns"] = ours.into();
    fs::write(
        &path,
        serde_json::to_string_pretty(&project).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOARD: &str = r#"(kicad_pcb (footprint "J" (layer "F.Cu") (at 0 0)
        (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/VBUS"))
        (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net "GND"))
        (pad "3" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "/SDA"))))"#;

    fn classes(json: &str) -> Vec<KiCadNetClassRequest> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn classes_set_the_rules_of_their_nets() {
        let pcb = parse(BOARD).unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            net_classes: classes(
                r#"[{"name": "Power", "nets": ["VBUS", "GND"], "track_width_mm": 0.5, "clearance_mm": 0.3},
                    {"name": "Thin", "nets": ["*"], "track_width_mm": 0.15}]"#,
            ),
            ..KiCadBoardRouterConfig::default()
        };
        let config = with_net_classes(&config, &pcb).unwrap();
        let rules = |net: &str| config.connection_rules[net].clone();
        assert_eq!((rules("VBUS").trace_width_mm, rules("VBUS").clearance_mm), (0.5, 0.3));
        assert_eq!(rules("GND").via_size_mm, 0.6);
        // The first class naming a net wins.
        assert_eq!((rules("SDA").trace_width_mm, rules("SDA").clearance_mm), (0.15, 0.2));
        for bad in [
            r#"[{"name": "Power", "nets": ["VCC"]}]"#,
            r#"[{"name": "Default", "nets": ["GND"]}]"#,
            r#"[{"name": "Power", "nets": ["GND"], "track_width_mm": 0}]"#,
        ] {
            let config = KiCadBoardRouterConfig { net_classes: classes(bad), ..KiCadBoardRouterConfig::default() };
            assert!(with_net_classes(&config, &pcb).is_err(), "{bad} was accepted");
        }
    }

    #[test]
    fn classes_are_written_into_the_project_ahead_of_its_own() {
        let directory = std::env::temp_dir().join(format!("pcb-maker-net-classes-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("board.kicad_pro"),
            r#"{"net_settings": {"classes": [{"name": "Default", "track_width": 0.25, "priority": 2147483647},
                {"name": "Old", "track_width": 1.0, "priority": 0}],
                "netclass_patterns": [{"netclass": "Old", "pattern": "GND"}]}}"#,
        )
        .unwrap();
        let pcb = parse(BOARD).unwrap();
        write_net_classes(
            &directory,
            "board",
            &pcb,
            &classes(r#"[{"name": "Power", "nets": ["VBUS", "GND"], "track_width_mm": 0.5}]"#),
        )
        .unwrap();
        let project: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(directory.join("board.kicad_pro")).unwrap()).unwrap();
        fs::remove_dir_all(&directory).unwrap();
        let classes = project["net_settings"]["classes"].as_array().unwrap();
        let power = classes.iter().find(|class| class["name"] == "Power").unwrap();
        assert_eq!((power["track_width"].as_f64(), power["priority"].as_i64()), (Some(0.5), Some(0)));
        let old = classes.iter().find(|class| class["name"] == "Old").unwrap();
        assert_eq!(old["priority"].as_i64(), Some(1));
        let patterns = &project["net_settings"]["netclass_patterns"];
        assert_eq!(patterns[0], serde_json::json!({"netclass": "Power", "pattern": "/VBUS"}));
        assert_eq!(patterns[1], serde_json::json!({"netclass": "Power", "pattern": "GND"}));
        assert_eq!(patterns[2]["netclass"], "Old");
    }
}
