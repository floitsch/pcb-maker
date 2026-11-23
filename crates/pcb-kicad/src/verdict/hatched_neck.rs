// Copyright (C) 2026 Florian Loitsch.

use super::{DrcFinding, KiCadRunFacts};
use pcb_kicad_route::necks::hatched_neck_target;
use std::{fs, path::Path};

pub(super) fn action(facts: &KiCadRunFacts, own: &[&DrcFinding]) -> Option<String> {
    let raw_drc =
        crate::native_report::read_kicad_report(Path::new(facts.outputs.get("drc")?)).ok()?;
    let path = Path::new(facts.outputs.get("board")?);
    let board = crate::parse(&fs::read_to_string(path).ok()?).ok()?;
    let (zone, current) = hatched_neck_target(&board, &raw_drc)?;
    let mut witnesses = own.iter().filter(|finding| {
        finding.kind == "connection_width"
            && matches!(finding.items.as_slice(), [item] if item.zone && item.uuid.as_deref() == Some(zone))
    });
    let label = witnesses.next()?.item_descriptions.first()?;
    let count = witnesses.count() + 1;
    let trial = if current < 0.6 {
        0.6
    } else {
        current + (1.0 - current) / 2.0
    };
    if trial <= current || trial >= 1.0 {
        return None;
    }
    let directory = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path.file_stem()?.to_str()?;
    let output = directory.with_file_name(format!("{name}-hatch-repair"));
    let quote = |argument: &str| format!("'{}'", argument.replace('\'', "'\"'\"'"));
    let command = format!(
        "pcb-maker repair-kicad-necks {} {} --retain-clipped-hatch {trial} {}",
        quote(&directory.to_string_lossy()),
        quote(name),
        quote(&output.to_string_lossy()),
    );
    Some(format!(
        "KiCad names the unsmoothed hatched {label} (UUID {zone}) alone in {count} new connection-width {}. Its current minimum hole-area fraction is {current}. If the design permits retaining more copper at clipped hatch holes, test an explicit change to {trial} on a separate copy with `{command}`. This keeps the minimum connection-width rule and may change the fill on every layer of the zone.",
        if count == 1 { "error" } else { "errors" },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hatch_advice_requires_a_new_error_naming_a_unique_eligible_zone() {
        let path = crate::fs_util::temporary_next_to(Path::new("/tmp/pcb-hatch-advice"));
        fs::create_dir(&path).unwrap();
        let board = path.join("test board.kicad_pcb");
        let drc = path.join("drc.json");
        let zone = |id: &str, policy: &str| format!("(zone (uuid {id}) (fill yes {policy}))");
        let hatch = zone("hatch", "(mode hatch) (hatch_min_hole_area 0.4)");
        let issue = json!({"type":"connection_width", "severity":"error", "items":[
            {"uuid":"hatch", "description":"Zone [GND] on F.Cu and B.Cu"}
        ]});
        let finding = crate::verdict::drc_finding(&issue);
        let facts = KiCadRunFacts {
            outputs: [
                ("board".into(), board.to_string_lossy().into_owned()),
                ("drc".into(), drc.to_string_lossy().into_owned()),
            ]
            .into(),
            ..KiCadRunFacts::new("layout-kicad-board")
        };
        let fixture = |zones: &str, finding| {
            fs::write(&board, format!("(kicad_pcb {zones})")).unwrap();
            fs::write(&drc, json!({"violations":[finding]}).to_string()).unwrap();
        };
        fixture(&hatch, issue.clone());
        let advice = action(&facts, &[&finding]).unwrap();
        assert!(advice.contains(
            "Zone [GND] on F.Cu and B.Cu (UUID hatch) alone in 1 new connection-width error"
        ));
        assert!(advice.contains("fraction is 0.4"));
        assert!(advice.contains("'test board' --retain-clipped-hatch 0.6"));
        assert!(action(&facts, &[]).is_none());
        for zones in [
            zone("hatch", ""),
            zone("hatch", "(mode hatch) (hatch_smoothing_level 1)"),
            zone("unrelated", "(mode hatch)"),
            format!("{hatch} {hatch}"),
        ] {
            fixture(&zones, issue.clone());
            assert!(action(&facts, &[&finding]).is_none());
        }
        let mut excluded = issue;
        excluded["excluded"] = json!(true);
        fixture(&hatch, excluded);
        assert!(action(&facts, &[&finding]).is_none());
        fs::remove_dir_all(path).unwrap();
    }
}
