// Copyright (C) 2026 Toit contributors.

//! Front and back layer renders of a board through kicad-cli, with rule
//! areas and copper zones outlined.

use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KiCadLayerRenderConfig {
    pub width: u32,
    pub height: u32,
    pub quality: String,
    pub background: String,
    /// Include component models and markings. Comparisons default to hiding
    /// these so they cannot obscure copper; pads and board geometry remain.
    pub show_components: bool,
    /// Add outline-only silkscreen geometry to a disposable render input so
    /// native 3D renders expose non-physical rule-area constraints.
    pub show_rule_areas: bool,
    /// Label and outline copper zones on the disposable render input. This is
    /// opt-in because plane-heavy production boards would otherwise be noisy.
    pub show_copper_zones: bool,
}

impl Default for KiCadLayerRenderConfig {
    fn default() -> Self {
        Self {
            width: 1200,
            height: 750,
            quality: "basic".into(),
            background: "opaque".into(),
            show_components: false,
            show_rule_areas: true,
            show_copper_zones: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadLayerRenderReport {
    pub front: PathBuf,
    pub back: PathBuf,
    pub config: KiCadLayerRenderConfig,
    pub rule_area_overlays: usize,
    pub copper_zone_overlays: usize,
}

/// Renders deterministic top and bottom journal images through KiCad's native
/// board renderer. These images are inspection evidence, not a replacement
/// for ERC, DRC, parity, or connectivity verification.
pub fn render_board_layers(
    pcb_path: &Path,
    output_prefix: &Path,
    config: &KiCadLayerRenderConfig,
) -> Result<KiCadLayerRenderReport, String> {
    validate_layer_render_config(config)?;
    if !pcb_path.is_file() {
        return Err(format!("KiCad board {} does not exist", pcb_path.display()));
    }
    if let Some(parent) = output_prefix
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create render directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let front = PathBuf::from(format!("{}-front.png", output_prefix.display()));
    let back = PathBuf::from(format!("{}-back.png", output_prefix.display()));
    let render_input = PathBuf::from(format!(
        "{}-inspection-{}-{}.kicad_pcb",
        output_prefix.display(),
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("system clock precedes Unix epoch: {error}"))?
            .as_nanos()
    ));
    let overlay_counts =
        if !config.show_components || config.show_rule_areas || config.show_copper_zones {
            prepare_layer_render_board(pcb_path, &render_input, config)?
        } else {
            KiCadLayerRenderOverlayCounts::default()
        };
    let overlay_count = overlay_counts.rule_areas + overlay_counts.copper_zones;
    let disposable_input = !config.show_components || overlay_count != 0;
    let rendered_pcb = if !disposable_input {
        pcb_path
    } else {
        render_input.as_path()
    };
    let render_result = run_kicad_render(rendered_pcb, &front, "top", config)
        .and_then(|()| run_kicad_render(rendered_pcb, &back, "bottom", config));
    if disposable_input {
        let cleanup_result = fs::remove_file(&render_input).map_err(|error| {
            format!(
                "failed to remove disposable render input {}: {error}",
                render_input.display()
            )
        });
        render_result?;
        cleanup_result?;
    } else {
        render_result?;
    }
    Ok(KiCadLayerRenderReport {
        front,
        back,
        config: config.clone(),
        rule_area_overlays: overlay_counts.rule_areas,
        copper_zone_overlays: overlay_counts.copper_zones,
    })
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct KiCadLayerRenderOverlayCounts {
    rule_areas: usize,
    copper_zones: usize,
}

fn prepare_layer_render_board(
    source: &Path,
    destination: &Path,
    config: &KiCadLayerRenderConfig,
) -> Result<KiCadLayerRenderOverlayCounts, String> {
    let source_text = fs::read_to_string(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?;
    let mut pcb = parse(&source_text)
        .map_err(|error| format!("failed to parse {}: {error}", source.display()))?;
    if pcb.head() != Some("kicad_pcb") {
        return Err(format!("{} is not a kicad_pcb document", source.display()));
    }
    let mut overlays = Vec::new();
    let mut counts = KiCadLayerRenderOverlayCounts::default();
    for (zone_index, zone) in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone"))
        .enumerate()
    {
        let rule_area = is_rule_area(zone);
        if (rule_area && !config.show_rule_areas) || (!rule_area && !config.show_copper_zones) {
            continue;
        }
        let Ok(layer_mask) = rule_area_layer_mask(zone) else {
            continue;
        };
        let Ok(points) = rule_area_polygon_points(zone) else {
            continue;
        };
        for (layer_index, enabled) in layer_mask.into_iter().enumerate() {
            if enabled {
                let layer = if layer_index == 0 {
                    "F.SilkS"
                } else {
                    "B.SilkS"
                };
                overlays.push(zone_render_outline_expr(
                    &points, layer, zone_index, rule_area,
                ));
                if rule_area {
                    counts.rule_areas += 1;
                } else {
                    overlays.push(copper_zone_render_label_expr(
                        &points,
                        layer,
                        zone_index,
                        node_net(zone)
                            .or_else(|| form_atom(zone, "net_name", 1))
                            .unwrap_or("unnamed"),
                    ));
                    counts.copper_zones += 1;
                }
            }
        }
    }
    if overlays.is_empty() && config.show_components {
        return Ok(counts);
    }
    let Expr::List(items) = &mut pcb else {
        unreachable!("kicad_pcb root must be a list")
    };
    if !config.show_components {
        // Only the disposable visualization copy loses graphics. Footprints,
        // pads, copper, holes, outline and source project stay intact.
        let is_marking = |item: &Expr| {
            matches!(
                form_atom(item, "layer", 1),
                Some("F.SilkS" | "B.SilkS" | "F.Fab" | "B.Fab")
            )
        };
        items.retain(|item| !is_marking(item));
        for item in items.iter_mut() {
            if matches!(item.head(), Some("footprint" | "module")) {
                let Expr::List(children) = item else {
                    unreachable!()
                };
                children.retain(|child| child.head() != Some("model") && !is_marking(child));
            }
        }
    }
    items.extend(overlays);
    fs::write(destination, format!("{}\n", encode(&pcb))).map_err(|error| {
        format!(
            "failed to write disposable render input {}: {error}",
            destination.display()
        )
    })?;
    Ok(counts)
}

fn zone_render_outline_expr(
    points: &[[f64; 2]],
    layer: &str,
    index: usize,
    rule_area: bool,
) -> Expr {
    let mut point_items = vec![Expr::Atom("pts".into())];
    point_items.extend(
        points
            .iter()
            .copied()
            .map(|point| coordinate_form("xy", point)),
    );
    Expr::List(vec![
        Expr::Atom("gr_poly".into()),
        Expr::List(point_items),
        Expr::List(vec![
            Expr::Atom("stroke".into()),
            Expr::List(vec![Expr::Atom("width".into()), Expr::Atom("0.4".into())]),
            Expr::List(vec![
                Expr::Atom("type".into()),
                Expr::Atom(if rule_area { "dash" } else { "solid" }.into()),
            ]),
        ]),
        Expr::List(vec![Expr::Atom("fill".into()), Expr::Atom("no".into())]),
        string_form("layer", layer),
        Expr::List(vec![
            Expr::Atom("uuid".into()),
            Expr::Atom(quote(&deterministic_uuid_for(&format!(
                "pcb-maker:zone-render-outline:{rule_area}:{layer}:{index}:{points:?}"
            )))),
        ]),
    ])
}

fn copper_zone_render_label_expr(
    points: &[[f64; 2]],
    layer: &str,
    index: usize,
    net_name: &str,
) -> Expr {
    let centroid = points.iter().fold([0.0, 0.0], |sum, point| {
        [sum[0] + point[0], sum[1] + point[1]]
    });
    let centroid = [
        centroid[0] / points.len() as f64,
        centroid[1] / points.len() as f64,
    ];
    let mut effects = vec![
        Expr::Atom("effects".into()),
        Expr::List(vec![
            Expr::Atom("font".into()),
            Expr::List(vec![
                Expr::Atom("size".into()),
                Expr::Atom("1.2".into()),
                Expr::Atom("1.2".into()),
            ]),
            Expr::List(vec![
                Expr::Atom("thickness".into()),
                Expr::Atom("0.2".into()),
            ]),
        ]),
    ];
    if layer == "B.SilkS" {
        effects.push(Expr::List(vec![
            Expr::Atom("justify".into()),
            Expr::Atom("mirror".into()),
        ]));
    }
    Expr::List(vec![
        Expr::Atom("gr_text".into()),
        Expr::Atom(quote(&format!("COPPER ZONE {net_name}"))),
        coordinate_form("at", centroid),
        string_form("layer", layer),
        Expr::List(vec![
            Expr::Atom("uuid".into()),
            Expr::Atom(quote(&deterministic_uuid_for(&format!(
                "pcb-maker:copper-zone-render-label:{layer}:{index}:{net_name}:{centroid:?}"
            )))),
        ]),
        Expr::List(effects),
    ])
}

fn validate_layer_render_config(config: &KiCadLayerRenderConfig) -> Result<(), String> {
    if !(1..=8192).contains(&config.width) || !(1..=8192).contains(&config.height) {
        return Err("KiCad render width and height must be within 1..=8192".into());
    }
    if !matches!(
        config.quality.as_str(),
        "basic" | "high" | "user" | "job_settings"
    ) {
        return Err(format!(
            "unsupported KiCad render quality {:?}",
            config.quality
        ));
    }
    if !matches!(
        config.background.as_str(),
        "default" | "transparent" | "opaque"
    ) {
        return Err(format!(
            "unsupported KiCad render background {:?}",
            config.background
        ));
    }
    Ok(())
}

fn run_kicad_render(
    pcb_path: &Path,
    output_path: &Path,
    side: &str,
    config: &KiCadLayerRenderConfig,
) -> Result<(), String> {
    let mut command = Command::new(crate::native_report::kicad_cli());
    command.args([
        "pcb",
        "render",
        "--output",
        &output_path.to_string_lossy(),
        "--width",
        &config.width.to_string(),
        "--height",
        &config.height.to_string(),
        "--side",
        side,
        "--background",
        &config.background,
        "--quality",
        &config.quality,
        &pcb_path.to_string_lossy(),
    ]);
    let result = crate::ledger::kicad_check(|| command.output())
        .map_err(|error| format!("failed to run kicad-cli PCB renderer: {error}"))?;
    if !result.status.success() || !output_path.is_file() {
        return Err(format!(
            "kicad-cli failed to render {side} image {} with {}: {}{}",
            output_path.display(),
            result.status,
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_layer_render_config_is_fail_closed() {
        assert!(validate_layer_render_config(&KiCadLayerRenderConfig::default()).is_ok());
        let legacy: KiCadLayerRenderConfig = serde_json::from_str(
            r#"{"width": 1000, "height": 750, "quality": "basic", "background": "opaque"}"#,
        )
        .unwrap();
        assert!(legacy.show_rule_areas);
        assert!(!legacy.show_copper_zones);
        let invalid = KiCadLayerRenderConfig {
            width: 0,
            quality: "cinematic".into(),
            ..KiCadLayerRenderConfig::default()
        };
        assert_eq!(
            validate_layer_render_config(&invalid).unwrap_err(),
            "KiCad render width and height must be within 1..=8192"
        );
        let invalid = KiCadLayerRenderConfig {
            quality: "cinematic".into(),
            ..KiCadLayerRenderConfig::default()
        };
        assert!(
            validate_layer_render_config(&invalid)
                .unwrap_err()
                .contains("unsupported KiCad render quality")
        );
    }

    #[test]
    fn render_overlay_exposes_rule_areas_without_mutating_source() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-kicad-rule-area-render-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let source_path = directory.join("source.kicad_pcb");
        let render_path = directory.join("render.kicad_pcb");
        let source = r#"(kicad_pcb
          (zone (layer "F.Cu")
            (keepout (tracks not_allowed) (vias not_allowed))
            (polygon (pts (xy 1 1) (xy 4 1) (xy 4 4) (xy 1 4))))
          (zone (layers "F&B.Cu")
            (keepout (tracks not_allowed) (vias not_allowed))
            (polygon (pts (xy 5 1) (xy 8 1) (xy 8 4) (xy 5 4))))
          (zone (layer "B.Cu")
            (keepout (tracks not_allowed) (vias not_allowed))
            (polygon (pts (xy 9 1) (xy 12 1) (xy 12 4) (xy 9 4))))
          (zone (net_name "/GND") (layer "F.Cu")
            (polygon (pts (xy 1 5) (xy 12 5) (xy 12 8) (xy 1 8)))))"#;
        fs::write(&source_path, source).unwrap();

        let config = KiCadLayerRenderConfig {
            show_copper_zones: true,
            ..KiCadLayerRenderConfig::default()
        };
        assert_eq!(
            prepare_layer_render_board(&source_path, &render_path, &config).unwrap(),
            KiCadLayerRenderOverlayCounts {
                rule_areas: 4,
                copper_zones: 1
            }
        );
        assert_eq!(fs::read_to_string(&source_path).unwrap(), source);
        let rendered = parse(&fs::read_to_string(&render_path).unwrap()).unwrap();
        let overlays = rendered
            .children()
            .iter()
            .filter(|item| item.head() == Some("gr_poly"))
            .collect::<Vec<_>>();
        assert_eq!(overlays.len(), 5);
        assert_eq!(
            overlays
                .iter()
                .filter(|item| form_atom(item, "layer", 1) == Some("F.SilkS"))
                .count(),
            3
        );
        assert_eq!(
            overlays
                .iter()
                .filter(|item| form_atom(item, "layer", 1) == Some("B.SilkS"))
                .count(),
            2
        );
        assert!(overlays.iter().all(|item| {
            item.child("fill").and_then(|fill| fill.children().get(1))
                == Some(&Expr::Atom("no".into()))
        }));
        let labels = rendered
            .children()
            .iter()
            .filter(|item| item.head() == Some("gr_text"))
            .collect::<Vec<_>>();
        assert_eq!(labels.len(), 1);
        assert_eq!(
            labels[0].children().get(1).and_then(Expr::atom),
            Some("COPPER ZONE /GND")
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
