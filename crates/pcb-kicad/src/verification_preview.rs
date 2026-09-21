// Copyright (C) 2026 Toit contributors.

//! Cheap visual journal of every native verification attempt. Preview errors
//! are diagnostic only and must never change the native admission decision.

use super::{
    VerificationReport, board_outline, file_sha256, footprint_reference, form_atom, form_xy, parse,
    remove_if_present, write_typed_json,
};
use serde_json::json;
use std::{
    fs,
    path::Path,
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub(super) fn capture(
    directory: &Path,
    board_id: &str,
    verify: impl FnOnce() -> Result<VerificationReport, String>,
) -> Result<VerificationReport, String> {
    let result = verify();
    let started = Instant::now();
    let board = directory.join(format!("{board_id}.kicad_pcb"));
    let source_sha256 = file_sha256(&board).ok();
    let rendered = render(&board, directory);
    let metadata = json!({
        "schema_version": 1,
        "source": board.file_name().map(|name| name.to_string_lossy()),
        "source_sha256": source_sha256,
        "verification": result.as_ref().ok(),
        "verification_error": result.as_ref().err(),
        "image": rendered.as_ref().ok().map(|()| "preview.svg"),
        "labels": directory.join("preview-labels.json").is_file().then_some("preview-labels.json"),
        "render_error": rendered.as_ref().err(),
        "render_elapsed_micros": started.elapsed().as_micros() as u64,
        "view": "F.Cu and B.Cu overlaid, viewed from top; pads, vias and board outline; component bodies and markings hidden",
        "scope": "Persisted board geometry; not a DRC overlay or proof of validity. Zone fills are plotted as stored."
    });
    if let Err(error) = rendered {
        eprintln!("preview for {} unavailable: {error}", board.display());
    }
    if let Err(error) = write_typed_json(&directory.join("preview.json"), &metadata) {
        eprintln!("failed to record preview status: {error}");
    }
    result
}

pub(super) fn render(board: &Path, directory: &Path) -> Result<(), String> {
    // Remove old output first: a failed export must not leave an image of the
    // previous board masquerading as the current attempt.
    let destination = directory.join("preview.svg");
    remove_if_present(&destination)?;
    remove_if_present(&directory.join("preview.json"))?;
    remove_if_present(&directory.join("preview-labels.json"))?;
    let annotation = annotations(board).ok();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let temporary = directory.join(format!(".preview-{}-{nonce}.svg", std::process::id()));
    let result = (|| {
        let output = Command::new("kicad-cli")
            .args([
                "pcb",
                "export",
                "svg",
                "--mode-single",
                "--layers",
                "F.Cu,B.Cu,Edge.Cuts",
                "--page-size-mode",
                if annotation.is_some() { "0" } else { "2" },
                "--exclude-drawing-sheet",
                "--output",
            ])
            .arg(&temporary)
            .arg(board)
            .output()
            .map_err(|error| format!("failed to start SVG export: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "SVG export failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let mut svg = fs::read_to_string(&temporary)
            .map_err(|error| format!("failed to read exported SVG: {error}"))?;
        if !svg.contains("<svg") || !svg.contains("</svg>") {
            return Err("SVG export did not produce an SVG document".into());
        }
        if let Some((bounds, labels, overlay)) = &annotation {
            // Page-mode 0 exports absolute board coordinates. Crop explicitly
            // so labels share the exact native coordinate frame.
            for (name, value) in [
                (
                    "viewBox",
                    format!(
                        "{} {} {} {}",
                        bounds[0] - 0.2,
                        bounds[1] - 0.2,
                        bounds[2] - bounds[0] + 0.4,
                        bounds[3] - bounds[1] + 0.4
                    ),
                ),
                ("width", format!("{}mm", bounds[2] - bounds[0] + 0.4)),
                ("height", format!("{}mm", bounds[3] - bounds[1] + 0.4)),
            ] {
                let root = svg.find("<svg").ok_or("SVG root missing")?;
                let start = root
                    + svg[root..]
                        .find(&format!("{name}=\""))
                        .ok_or("SVG dimension missing")?
                    + name.len()
                    + 2;
                let end = start + svg[start..].find('"').ok_or("SVG dimension unterminated")?;
                svg.replace_range(start..end, &value);
            }
            svg = svg.replacen("</svg>", &format!("{overlay}</svg>"), 1);
            fs::write(&temporary, svg).map_err(|e| e.to_string())?;
            write_typed_json(&directory.join("preview-labels.json"), labels)?;
        }
        fs::rename(&temporary, &destination)
            .map_err(|error| format!("failed to publish SVG preview: {error}"))
    })();
    let _ = remove_if_present(&temporary);
    result
}

fn annotations(board: &Path) -> Result<([f64; 4], serde_json::Value, String), String> {
    let pcb = parse(&fs::read_to_string(board).map_err(|e| e.to_string())?)?;
    let bounds = board_outline(&pcb)?.ok_or("outline unavailable")?.bounds;
    let escape = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let mut labels = Vec::new();
    let mut overlay = String::from(
        "<g id=\"inspection-labels\" font-family=\"sans-serif\" font-size=\"0.65\" fill=\"#172b3a\" stroke=\"white\" stroke-width=\"0.12\" paint-order=\"stroke fill\" stroke-linejoin=\"round\">",
    );
    for item in pcb.children() {
        let (label, kind) = match item.head() {
            Some("footprint") => (footprint_reference(item).unwrap_or_default(), "component"),
            // UUID fragments stay stable when another via is removed. The
            // sidecar retains the full UUID, position and net for queries.
            Some("via") => {
                let id = form_atom(item, "uuid", 1).unwrap_or("");
                (
                    format!("V-{}", id.chars().take(6).collect::<String>()),
                    "via",
                )
            }
            _ => continue,
        };
        if label.is_empty() {
            continue;
        }
        let at = form_xy(item, "at")?;
        let x = (at[0] + 0.7)
            .min(bounds[2] - label.len() as f64 * 0.42)
            .max(bounds[0] + 0.2);
        let y = (at[1] - 0.7).max(bounds[1] + 0.8).min(bounds[3] - 0.2);
        let net = item
            .child("net")
            .and_then(|n| n.children().last())
            .and_then(|n| n.atom())
            .unwrap_or("");
        overlay.push_str(&format!(
            "<text x=\"{x}\" y=\"{y}\"><title>{}</title>{}</text>",
            escape(&format!("{kind} {label}; {net}; {}, {} mm", at[0], at[1])),
            escape(&label)
        ));
        labels.push(json!({"label": label, "kind": kind, "at_mm": at, "uuid": form_atom(item,"uuid",1), "net": net}));
    }
    overlay.push_str("</g>");
    Ok((
        bounds,
        json!({"scope":"Reference labels only; component bodies remain hidden. Via labels use stable UUID prefixes.","labels":labels}),
        overlay,
    ))
}

/// A committed board carries the preview of the native candidate that produced
/// it. Missing/failed previews clear any image belonging to an earlier board.
pub(super) fn copy_to(source: &Path, destination: &Path) {
    for name in ["preview.svg", "preview.json", "preview-labels.json"] {
        let result = (|| {
            let target = destination.join(name);
            remove_if_present(&target)?;
            if source.join(name).is_file() {
                fs::copy(source.join(name), target).map_err(|error| error.to_string())?;
            }
            Ok::<(), String>(())
        })();
        if let Err(error) = result {
            eprintln!(
                "failed to copy {name} to {}: {error}",
                destination.display()
            );
        }
    }
}
