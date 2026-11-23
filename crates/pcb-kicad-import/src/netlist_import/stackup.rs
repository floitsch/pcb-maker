// Copyright (C) 2026 Florian Loitsch.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Explicit data from a fabrication stack. The rest of the donor stays out of the import.

use super::*;
use pcb_kicad_route::board_router::LayerTable;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize)]
pub struct KiCadImportedStackup {
    pub source: PathBuf,
    pub source_sha256: String,
    pub thickness_mm: f64,
    pub copper_layers: Vec<String>,
}

pub(super) struct Stackup {
    pub report: KiCadImportedStackup,
    thickness: Expr,
    layers: Expr,
}

impl Stackup {
    pub(super) fn read(
        source: &Path,
        output: &Path,
        requested: Option<usize>,
    ) -> Result<Self, String> {
        let fail =
            |why: String| exit::input_error(format!("--stackup-from {}: {why}", source.display()));
        let input = fs::canonicalize(source).map_err(|error| fail(error.to_string()))?;
        if fs::canonicalize(output).is_ok_and(|path| path == input) {
            return Err(fail(
                "the stackup source is also the PCB being written; give a separate output project"
                    .into(),
            ));
        }
        let text = fs::read_to_string(source).map_err(|error| fail(error.to_string()))?;
        let pcb = parse(&text).map_err(|error| fail(error.to_string()))?;
        let (thickness, thickness_mm, layers, copper_layers) =
            forms(&pcb, requested).map_err(fail)?;
        Ok(Self {
            report: KiCadImportedStackup {
                source: source.to_path_buf(),
                source_sha256: format!("{:x}", Sha256::digest(text.as_bytes())),
                thickness_mm,
                copper_layers,
            },
            thickness: thickness.clone(),
            layers: layers.clone(),
        })
    }

    pub(super) fn apply(&self, pcb: &mut Expr) {
        let Expr::List(items) = pcb else {
            unreachable!("generated board is a list");
        };
        for item in items {
            match item.head() {
                Some("general") => {
                    let Expr::List(fields) = item else {
                        unreachable!()
                    };
                    let thickness = fields
                        .iter_mut()
                        .find(|field| field.head() == Some("thickness"));
                    *thickness.expect("generated board thickness") = self.thickness.clone();
                }
                Some("setup") => {
                    let Expr::List(fields) = item else {
                        unreachable!()
                    };
                    fields.insert(1, self.layers.clone());
                }
                _ => {}
            }
        }
    }
}

fn forms(pcb: &Expr, requested: Option<usize>) -> Result<(&Expr, f64, &Expr, Vec<String>), String> {
    if pcb.head() != Some("kicad_pcb") {
        return Err("expected a KiCad PCB".into());
    }
    let thickness = sole(sole(pcb, "general")?, "thickness")?;
    let thickness_mm = thickness
        .children()
        .get(1)
        .and_then(Expr::atom)
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0 && thickness.children().len() == 2)
        .ok_or("general.thickness needs a positive thickness in millimetres")?;
    let stackup = sole(sole(pcb, "setup")?, "stackup")?;
    sole(pcb, "layers")?;
    let actual = LayerTable::from_pcb(pcb)?.names;
    let count = actual.len();
    let expected: Vec<_> = std::iter::once("F.Cu".to_string())
        .chain((1..count - 1).map(|i| format!("In{i}.Cu")))
        .chain(std::iter::once("B.Cu".to_string()))
        .collect();
    if count % 2 != 0 || actual != expected {
        return Err(format!(
            "the donor's active copper stack {actual:?} cannot be imported on standard KiCad copper layers"
        ));
    }
    if let Some(requested) = requested.filter(|&layers| layers != count) {
        return Err(format!(
            "the donor has {count} copper layers but --layers asks for {requested}"
        ));
    }
    let mut authored = Vec::new();
    for layer in stackup
        .children()
        .iter()
        .filter(|layer| layer.head() == Some("layer"))
    {
        let name = layer
            .children()
            .get(1)
            .and_then(Expr::atom)
            .ok_or("unnamed stackup layer")?;
        let kind = sole(layer, "type")?;
        let kind = match kind.children() {
            [_, atom] => atom.atom().ok_or("invalid stackup layer type")?,
            _ => return Err("invalid stackup layer type".into()),
        };
        if kind == "copper" {
            authored.push(name);
        } else if name.ends_with(".Cu") {
            return Err(format!("stackup layer {name} has no copper type"));
        }
    }
    if authored != actual {
        return Err(format!(
            "the stackup copper sequence {authored:?} differs from the donor's active layers {actual:?}"
        ));
    }
    Ok((thickness, thickness_mm, stackup, actual))
}

fn sole<'a>(parent: &'a Expr, head: &str) -> Result<&'a Expr, String> {
    let mut children = parent
        .children()
        .iter()
        .filter(|form| form.head() == Some(head));
    match (children.next(), children.next()) {
        (Some(form), None) => Ok(form),
        _ => Err(format!(
            "expected exactly one {head} form in {}",
            parent.head().unwrap_or("PCB")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"(kicad_pcb
        (general (thickness 1.12345) (legacy_teardrops yes))
        (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (2 "B.Cu" signal))
        (setup (stackup
            (layer "F.Cu" (type "copper") (thickness 0.035))
            (layer "dielectric 1" (type "prepreg") (epsilon_r 4.15) (material "FR4"))
            (layer "In1.Cu" (type "copper") (thickness 0.018))
            (layer "In2.Cu" (type "copper") (thickness 0.018))
            (layer "B.Cu" (type "copper") (thickness 0.035))
            (copper_finish "ENIG") (dielectric_constraints yes))
          (pad_to_mask_clearance 0.1))
        (gr_rect (start 0 0) (end 10 10) (layer "Edge.Cuts")))"#;

    #[test]
    fn import_carries_only_the_explicit_fabrication_stack() {
        let scratch = std::env::temp_dir().join(format!(
            "pcb-maker-stackup-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&scratch).unwrap();
        let netlist = scratch.join("board.net");
        let source = scratch.join("donor.kicad_pcb");
        fs::write(&netlist, "(export (version E) (components) (nets))").unwrap();
        fs::write(&source, SOURCE).unwrap();
        let opts = KiCadNetlistImportOptions {
            stackup_from: Some(&source),
            ..Default::default()
        };
        let output = scratch.join("import");
        let report = import_kicad_netlist_with_options(&netlist, &output, "board", opts).unwrap();
        assert_eq!(report.copper_layers, 4);
        assert_eq!(report.copper_layers_from, "--stackup-from");
        assert_eq!(report.stackup_from.as_ref().unwrap().thickness_mm, 1.12345);
        let imported = parse(&fs::read_to_string(report.board).unwrap()).unwrap();
        let donor = parse(SOURCE).unwrap();
        assert_eq!(
            forms(&imported, Some(4)).unwrap(),
            forms(&donor, Some(4)).unwrap()
        );
        assert!(imported.child("gr_rect").is_none());
        assert_eq!(
            form_atom(imported.child("setup").unwrap(), "pad_to_mask_clearance", 1),
            Some("0")
        );
        let mismatch = KiCadNetlistImportOptions {
            layers: Some(2),
            ..opts
        };
        let refused = scratch.join("refused");
        let error =
            import_kicad_netlist_with_options(&netlist, &refused, "board", mismatch).unwrap_err();
        assert!(error.contains("donor has 4 copper layers"), "{error}");
        assert!(!refused.exists());
        assert!(import_kicad_netlist_with_options(&netlist, &scratch, "donor", opts).is_err());
        assert_eq!(fs::read_to_string(&source).unwrap(), SOURCE);
        let ordinary =
            import_kicad_netlist(&netlist, &scratch.join("ordinary"), "board", 4).unwrap();
        assert!(ordinary.stackup_from.is_none());
        let ordinary = parse(&fs::read_to_string(ordinary.board).unwrap()).unwrap();
        assert_eq!(
            form_atom(ordinary.child("general").unwrap(), "thickness", 1),
            Some("1.6")
        );
        assert!(ordinary.child("setup").unwrap().child("stackup").is_none());
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn active_and_authored_copper_must_match_in_physical_order() {
        let malformed = SOURCE.replace("(layer \"In1.Cu\"", "(layer \"In2.Cu\"");
        let error = forms(&parse(&malformed).unwrap(), None).unwrap_err();
        assert!(error.contains("stackup copper sequence"), "{error}");
        let malformed = SOURCE.replace("(thickness 1.12345)", "(thickness 0)");
        assert!(forms(&parse(&malformed).unwrap(), None).is_err());
        assert!(forms(&empty_board(4).unwrap(), None).is_err());
    }
}
