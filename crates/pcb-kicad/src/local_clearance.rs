// Copyright (C) 2026 Toit contributors.

//! Native pad/footprint clearance overrides, shared by routing and repair.
//! The configured router clearance remains a conservative floor. This does
//! not resolve custom design rules, net classes, or pair-specific exceptions.

use super::{Expr, form_f64};

pub(super) fn pad_clearance(pad: &Expr, footprint: &Expr) -> Result<f64, String> {
    // KiCad 9+: an explicit pad zero overrides the footprint; an absent pad
    // value inherits it. See the native manual's pad clearance overrides:
    // https://docs.kicad.org/10.0/en/pcbnew/pcbnew.html
    let owner = if pad.child("clearance").is_some() {
        pad
    } else if footprint.child("clearance").is_some() {
        footprint
    } else {
        return Ok(0.0);
    };
    let clearance = form_f64(owner, "clearance", 1)?;
    if !clearance.is_finite() {
        return Err("unsupported non-finite pad/footprint clearance".into());
    }
    // Old (KiCad 4) boards carry negative overrides; they never lower the
    // clearance here, the class clearance applies.
    Ok(clearance.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn pad_override_precedes_footprint_including_explicit_zero() {
        let footprint = parse("(footprint (clearance 2.5))").unwrap();
        for (source, expected) in [
            ("(pad)", 2.5),
            ("(pad (clearance 1.85))", 1.85),
            ("(pad (clearance 0))", 0.0),
        ] {
            assert_eq!(
                pad_clearance(&parse(source).unwrap(), &footprint).unwrap(),
                expected
            );
        }
        assert!(pad_clearance(&parse("(pad (clearance NaN))").unwrap(), &footprint).is_err());
        // KiCad 4 boards carry negative overrides; they mean no override.
        assert_eq!(
            pad_clearance(&parse("(pad (clearance -1))").unwrap(), &footprint).unwrap(),
            0.0
        );
    }
}
