// Copyright (C) 2026 Toit contributors.

//! KiCad's custom design rules (`<project>.kicad_dru`): the file, the
//! condition language for what the router can tell apart, and the
//! lowering of each rule to the router's rules.
//!
//! KiCad (pcbnew/drc/drc_engine.cpp, `EvalRules`, 10.0.7) takes for an
//! item pair the last rule in the file whose layer and condition match
//! (conditions are commutative: A and B are tried both ways); a matching
//! custom rule replaces the net classes' and the board setup's value.
//! The router has one clearance per net (a pair keeps the larger of the
//! two), one width per net, one via per net and one edge, hole and
//! hole-to-hole clearance for the board, so the lowering keeps, for each,
//! the largest value of every rule that can match a pair it routes
//! (track or via against track, via or pad): never less than KiCad asks,
//! more where a later rule loosens an earlier one or where a rule is
//! narrower than what the router can tell apart (an item type, a layer,
//! a pair of classes).
//!
//! The module: the rule file and what came of each constraint (here),
//! the condition language (`condition`), the lowering of each constraint
//! (`lower`, with the clearances in `clearance` and the sizes of tracks
//! and vias in `sizes`), and the rule KiCad applies to one track
//! (`tracks`).

use serde::Serialize;

mod clearance;
mod condition;
pub mod lower;
mod sizes;
mod tracks;

pub(crate) use condition::{Kind, NetFacts};
pub(crate) use lower::{
    BoardFacts, Lowered, NamedArea, finding_severity, lower, rule_copper_layers,
};
pub(crate) use tracks::{Picked, track_constraint, track_disallowed, track_rule};

/// One `(rule ...)` of a `.kicad_dru`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rule {
    pub name: String,
    pub layer: Option<String>,
    pub condition: Option<String>,
    pub severity: Option<String>,
    pub constraints: Vec<Constraint>,
}

/// One `(constraint ...)` of a rule; lengths in millimetres.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Constraint {
    pub kind: String,
    pub min: Option<f64>,
    pub opt: Option<f64>,
    pub max: Option<f64>,
    /// Bare words after the kind (`disallow via track`, `zone_connection
    /// solid`) and the text of an `assertion`.
    pub words: Vec<String>,
}

/// What a rule did, for `board-router.json`'s `rules`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KiCadCustomRuleReport {
    /// The rule's name in the .kicad_dru.
    pub rule: String,
    /// The constraint (`clearance`, `track_width`, ...).
    pub constraint: String,
    /// `applied` (the router keeps it as KiCad does), `approximated` (kept
    /// more widely than KiCad asks: every layer, every item type, both
    /// nets of a pair), `no_effect` (the router's rules already keep at
    /// least as much, or nothing it routes matches), `not_routing` (not a
    /// rule on the copper the router lays: silk, text, courtyards, pads),
    /// `unmodelled` (a routing rule the router cannot keep; KiCad still
    /// checks it), `skipped` (its severity or the project's makes its
    /// findings no errors, or no layer of the board has it).
    pub status: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq)]
enum Sexp {
    Atom(String),
    Text(String),
    List(Vec<Sexp>),
}

impl Sexp {
    fn word(&self) -> Option<&str> {
        match self {
            Self::Atom(text) | Self::Text(text) => Some(text),
            Self::List(_) => None,
        }
    }

    fn head(&self) -> Option<&str> {
        match self {
            Self::List(items) => items.first().and_then(Self::word),
            _ => None,
        }
    }
}

/// The forms of a rule file: comment lines (`#` first) are skipped,
/// strings take backslash escapes, as KiCad's DSN lexer reads them.
fn sexps(text: &str) -> Result<Vec<Sexp>, String> {
    let mut stack: Vec<Vec<Sexp>> = vec![Vec::new()];
    let mut chars = text.chars().peekable();
    let mut line_start = true;
    while let Some(c) = chars.next() {
        match c {
            '\n' => line_start = true,
            c if c.is_whitespace() => {}
            '#' if line_start => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
                line_start = true;
                continue;
            }
            '(' => {
                // Deeper than any rule file nests: the tree's drop and walks
                // recurse (`kicad_sexpr::MAX_NESTING`).
                if stack.len() > crate::kicad_sexpr::MAX_NESTING {
                    return Err(format!(
                        "lists nested deeper than {} levels",
                        crate::kicad_sexpr::MAX_NESTING
                    ));
                }
                stack.push(Vec::new());
                line_start = false;
            }
            ')' => {
                let list = stack.pop().filter(|_| !stack.is_empty());
                let Some(list) = list else {
                    return Err("unbalanced ')'".into());
                };
                stack
                    .last_mut()
                    .ok_or("unbalanced ')'")?
                    .push(Sexp::List(list));
                line_start = false;
            }
            '"' => {
                let mut out = String::new();
                let mut closed = false;
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(next) = chars.next() {
                                out.push(next);
                            }
                        }
                        '"' => {
                            closed = true;
                            break;
                        }
                        c => out.push(c),
                    }
                }
                if !closed {
                    return Err("unterminated string".into());
                }
                stack.last_mut().unwrap().push(Sexp::Text(out));
                line_start = false;
            }
            c => {
                let mut out = String::from(c);
                while let Some(&next) = chars.peek() {
                    if next.is_whitespace() || matches!(next, '(' | ')' | '"') {
                        break;
                    }
                    out.push(next);
                    chars.next();
                }
                stack.last_mut().unwrap().push(Sexp::Atom(out));
                line_start = false;
            }
        }
        if c != '\n' && !c.is_whitespace() {
            line_start = false;
        }
    }
    if stack.len() != 1 {
        return Err("unbalanced '('".into());
    }
    Ok(stack.pop().unwrap())
}

/// A length as KiCad's rule parser reads it, in millimetres (`0.2`,
/// `0.2mm`, `8mil`, `0.01in`, `200um`; a bare number is millimetres).
pub(crate) fn length_mm(text: &str) -> Option<f64> {
    let text = text.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')))
        .unwrap_or(text.len());
    // An exponent's `e` must not eat a unit (`1em` is not a length anyway).
    let (number, unit) = text.split_at(split);
    let value: f64 = number
        .parse()
        .ok()
        .filter(|value: &f64| value.is_finite())?;
    let scale = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "mm" => 1.0,
        "mil" | "mils" | "thou" => 0.0254,
        "in" | "inch" | "\"" => 25.4,
        "um" | "µm" => 0.001,
        "cm" => 10.0,
        _ => return None,
    };
    Some(value * scale)
}

/// The rules of a `.kicad_dru`.
pub(crate) fn parse_rules(text: &str) -> Result<Vec<Rule>, String> {
    let mut rules = Vec::new();
    for form in sexps(text)? {
        if form.head() != Some("rule") {
            continue;
        }
        let Sexp::List(items) = form else { continue };
        let name = items
            .get(1)
            .and_then(Sexp::word)
            .unwrap_or("(unnamed)")
            .to_string();
        let mut rule = Rule {
            name,
            layer: None,
            condition: None,
            severity: None,
            constraints: Vec::new(),
        };
        for item in items.iter().skip(2) {
            let Sexp::List(parts) = item else { continue };
            let word = |index: usize| parts.get(index).and_then(Sexp::word).map(str::to_string);
            match item.head() {
                Some("layer") => rule.layer = word(1),
                Some("condition") => rule.condition = word(1),
                Some("severity") => rule.severity = word(1),
                Some("constraint") => {
                    let mut constraint = Constraint {
                        kind: word(1).unwrap_or_default(),
                        min: None,
                        opt: None,
                        max: None,
                        words: Vec::new(),
                    };
                    for part in parts.iter().skip(2) {
                        match part {
                            Sexp::List(limit) => {
                                let value = || {
                                    let mut text: String = limit
                                        .iter()
                                        .skip(1)
                                        .filter_map(Sexp::word)
                                        .collect::<Vec<_>>()
                                        .join("");
                                    text.retain(|c| !c.is_whitespace());
                                    length_mm(&text)
                                };
                                match part.head() {
                                    Some("min") => constraint.min = value(),
                                    Some("opt") => constraint.opt = value(),
                                    Some("max") => constraint.max = value(),
                                    Some(other) => constraint.words.push(other.to_string()),
                                    None => {}
                                }
                            }
                            other => {
                                if let Some(text) = other.word() {
                                    constraint.words.push(text.to_string());
                                }
                            }
                        }
                    }
                    rule.constraints.push(constraint);
                }
                _ => {}
            }
        }
        rules.push(rule);
    }
    Ok(rules)
}

#[cfg(test)]
mod tests;
