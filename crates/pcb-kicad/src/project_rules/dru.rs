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

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

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

// ---------------------------------------------------------------------------
// The condition language (KiCad's libeval as pcbnew uses it).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    A,
    B,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Compare {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

#[derive(Clone, Debug, PartialEq)]
enum Ex {
    Or(Box<Ex>, Box<Ex>),
    And(Box<Ex>, Box<Ex>),
    Not(Box<Ex>),
    Compare(Compare, Box<Ex>, Box<Ex>),
    /// `A.Type`: the property's name normalised (lower case, without `_`
    /// and spaces), as KiCad matches property names.
    Property(Side, String),
    Call(Side, String, Vec<Ex>),
    Text(String),
    Number(f64),
    Null,
    /// Arithmetic, a bare word: nothing the lowering can decide.
    Opaque(String),
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Word(String),
    Text(String),
    Number(f64),
    Symbol(&'static str),
}

fn lex(text: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    const SYMBOLS: [&str; 17] = [
        "||", "&&", "==", "!=", "<=", ">=", "<", ">", "!", "(", ")", ",", ".", "+", "-", "*", "/",
    ];
    while index < chars.len() {
        let c = chars[index];
        if c.is_whitespace() {
            index += 1;
        } else if c == '\'' || c == '"' {
            let mut out = String::new();
            index += 1;
            while index < chars.len() && chars[index] != c {
                if chars[index] == '\\' && index + 1 < chars.len() {
                    index += 1;
                }
                out.push(chars[index]);
                index += 1;
            }
            if index >= chars.len() {
                return Err("unterminated string in condition".into());
            }
            index += 1;
            tokens.push(Token::Text(out));
        } else if c.is_ascii_digit()
            || (c == '.' && chars.get(index + 1).is_some_and(char::is_ascii_digit))
        {
            let start = index;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric() || chars[index] == '.')
            {
                index += 1;
            }
            let word: String = chars[start..index].iter().collect();
            tokens.push(Token::Number(
                length_mm(&word).ok_or_else(|| format!("unreadable number {word:?}"))?,
            ));
        } else if c.is_alphabetic() || c == '_' {
            let start = index;
            while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
                index += 1;
            }
            tokens.push(Token::Word(chars[start..index].iter().collect()));
        } else {
            let rest: String = chars[index..chars.len().min(index + 2)].iter().collect();
            let Some(symbol) = SYMBOLS.iter().find(|symbol| rest.starts_with(**symbol)) else {
                return Err(format!("unexpected {c:?} in condition"));
            };
            index += symbol.chars().count();
            tokens.push(Token::Symbol(symbol));
        }
    }
    Ok(tokens)
}

fn normalise(name: &str) -> String {
    name.chars()
        .filter(|c| *c != '_' && !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// How deep a condition's expression may be: its tree (chains of `||` and
/// `&&` balanced, `balanced`) and the parser's own nesting (parentheses,
/// `!`, calls). KiCad's conditions are a few levels
/// deep; deeper is refused as the input's error, before the tree (whose
/// evaluation, walks and drop recurse) grows past it: until 2026-10-21
/// `((((` or `!!!!` by the hundred thousand overflowed the stack.
pub(crate) const MAX_CONDITION_DEPTH: usize = 200;

struct ExParser {
    tokens: Vec<Token>,
    at: usize,
    /// The parser's nesting now (parentheses, `!`, `-`, calls).
    nesting: usize,
}

/// An expression and its tree's depth.
type Parsed = Result<(Ex, usize), String>;

fn too_deep() -> String {
    format!("the condition nests deeper than {MAX_CONDITION_DEPTH} levels")
}

/// The depth of a node over children of `depths`, refused past the limit.
fn node_depth(depths: &[usize]) -> Result<usize, String> {
    let depth = depths.iter().copied().max().unwrap_or(0) + 1;
    if depth > MAX_CONDITION_DEPTH {
        return Err(too_deep());
    }
    Ok(depth)
}

/// A chain of `||` or `&&` as a tree of `join`s as deep as the logarithm
/// of its length, in its order (`a || b || c` is `(a || b) || c`, as
/// read left to right): a condition listing a thousand nets nests ten
/// levels, not a thousand. Both are associative, so the tree's shape
/// changes no truth.
fn balanced(mut operands: Vec<(Ex, usize)>, join: fn(Box<Ex>, Box<Ex>) -> Ex) -> Parsed {
    if operands.len() == 1 {
        return Ok(operands.pop().expect("one operand"));
    }
    let right = operands.split_off(operands.len().div_ceil(2));
    let (left, left_depth) = balanced(operands, join)?;
    let (right, right_depth) = balanced(right, join)?;
    Ok((
        join(Box::new(left), Box::new(right)),
        node_depth(&[left_depth, right_depth])?,
    ))
}

impl ExParser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn eat(&mut self, symbol: &str) -> bool {
        if matches!(self.peek(), Some(Token::Symbol(s)) if *s == symbol) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    /// Runs `parse` one nesting level deeper, refused past the limit.
    fn nested(&mut self, parse: impl FnOnce(&mut Self) -> Parsed) -> Parsed {
        if self.nesting >= MAX_CONDITION_DEPTH {
            return Err(too_deep());
        }
        self.nesting += 1;
        let parsed = parse(self);
        self.nesting -= 1;
        parsed
    }

    fn or(&mut self) -> Parsed {
        let mut operands = vec![self.and()?];
        while self.eat("||") {
            operands.push(self.and()?);
        }
        balanced(operands, Ex::Or)
    }

    fn and(&mut self) -> Parsed {
        let mut operands = vec![self.comparison()?];
        while self.eat("&&") {
            operands.push(self.comparison()?);
        }
        balanced(operands, Ex::And)
    }

    fn comparison(&mut self) -> Parsed {
        let (left, left_depth) = self.additive()?;
        let operator = match self.peek() {
            Some(Token::Symbol("==")) => Compare::Equal,
            Some(Token::Symbol("!=")) => Compare::NotEqual,
            Some(Token::Symbol("<")) => Compare::Less,
            Some(Token::Symbol("<=")) => Compare::LessEqual,
            Some(Token::Symbol(">")) => Compare::Greater,
            Some(Token::Symbol(">=")) => Compare::GreaterEqual,
            _ => return Ok((left, left_depth)),
        };
        self.at += 1;
        let (right, right_depth) = self.additive()?;
        let depth = node_depth(&[left_depth, right_depth])?;
        Ok((
            Ex::Compare(operator, Box::new(left), Box::new(right)),
            depth,
        ))
    }

    fn additive(&mut self) -> Parsed {
        let (left, depth) = self.unary()?;
        if matches!(self.peek(), Some(Token::Symbol("+" | "-" | "*" | "/"))) {
            // Arithmetic (`A.Width * 2`): read it, decide nothing with it.
            while self.eat("+") || self.eat("-") || self.eat("*") || self.eat("/") {
                self.unary()?;
            }
            return Ok((Ex::Opaque("arithmetic".into()), 1));
        }
        Ok((left, depth))
    }

    fn unary(&mut self) -> Parsed {
        if self.eat("!") {
            return self.nested(|parser| {
                let (inner, depth) = parser.unary()?;
                Ok((Ex::Not(Box::new(inner)), node_depth(&[depth])?))
            });
        }
        if self.eat("-") {
            return self.nested(|parser| {
                Ok(match parser.unary()?.0 {
                    Ex::Number(value) => (Ex::Number(-value), 1),
                    _ => (Ex::Opaque("negation".into()), 1),
                })
            });
        }
        self.primary()
    }

    /// A call's arguments and the deepest of them.
    fn arguments(&mut self) -> Result<(Vec<Ex>, usize), String> {
        let mut arguments = Vec::new();
        let mut deepest = 0;
        if self.eat(")") {
            return Ok((arguments, deepest));
        }
        loop {
            let (argument, depth) = self.or()?;
            arguments.push(argument);
            deepest = deepest.max(depth);
            if self.eat(")") {
                return Ok((arguments, deepest));
            }
            if !self.eat(",") {
                return Err("expected ',' or ')' in a call".into());
            }
        }
    }

    fn primary(&mut self) -> Parsed {
        let token = self.peek().cloned().ok_or("unexpected end of condition")?;
        self.at += 1;
        match token {
            Token::Symbol("(") => self.nested(|parser| {
                let inner = parser.or()?;
                if !parser.eat(")") {
                    return Err("expected ')'".into());
                }
                Ok(inner)
            }),
            Token::Text(text) => Ok((Ex::Text(text), 1)),
            Token::Number(value) => Ok((Ex::Number(value), 1)),
            Token::Word(word) => {
                let side = match word.as_str() {
                    "A" => Some(Side::A),
                    "B" => Some(Side::B),
                    "AB" => Some(Side::Both),
                    _ => None,
                };
                if let Some(side) = side
                    && self.eat(".")
                {
                    let Some(Token::Word(name)) = self.peek().cloned() else {
                        return Err(format!("expected a property after {word}."));
                    };
                    self.at += 1;
                    if self.eat("(") {
                        return self.nested(|parser| {
                            let (arguments, depth) = parser.arguments()?;
                            Ok((
                                Ex::Call(side, normalise(&name), arguments),
                                node_depth(&[depth])?,
                            ))
                        });
                    }
                    return Ok((Ex::Property(side, normalise(&name)), 1));
                }
                if self.eat("(") {
                    self.nested(|parser| parser.arguments().map(|_| (Ex::Null, 1)))?;
                    return Ok((Ex::Opaque(format!("{word}()")), 1));
                }
                Ok((
                    match word.to_ascii_lowercase().as_str() {
                        "null" | "undefined" => Ex::Null,
                        _ => Ex::Opaque(word),
                    },
                    1,
                ))
            }
            Token::Symbol(symbol) => Err(format!("unexpected {symbol:?} in condition")),
        }
    }
}

fn parse_condition(text: &str) -> Result<Ex, String> {
    let mut parser = ExParser {
        tokens: lex(text)?,
        at: 0,
        nesting: 0,
    };
    if parser.tokens.is_empty() {
        return Ok(Ex::Number(1.0));
    }
    let (ex, _) = parser.or()?;
    if parser.at != parser.tokens.len() {
        return Err("trailing input in condition".into());
    }
    Ok(ex)
}

/// What a condition says, as far as the router can tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum T3 {
    True,
    False,
    Unknown,
}

impl T3 {
    fn of(value: bool) -> Self {
        if value { Self::True } else { Self::False }
    }

    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::False, _) | (_, Self::False) => Self::False,
            (Self::True, Self::True) => Self::True,
            _ => Self::Unknown,
        }
    }

    fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::True, _) | (_, Self::True) => Self::True,
            (Self::False, Self::False) => Self::False,
            _ => Self::Unknown,
        }
    }

    fn not(self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            Self::Unknown => Self::Unknown,
        }
    }
}

/// The kinds of item the lowering tells apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Kind {
    Track,
    Via,
    SmdPad,
    PlatedPad,
    NpthPad,
    /// An edge-cut graphic (the other item of an edge clearance).
    Edge,
}

/// A net as the conditions see it: its name and its net classes (the
/// constituents of its effective class, which KiCad names by joining
/// them with commas).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NetFacts {
    pub name: String,
    pub classes: Vec<String>,
    /// The net's differential pair as `inDiffPair()` sees it: `None` when
    /// unknown (the custom rules' lowering: a condition on it is not
    /// lowered), `Some(None)` for a net in no pair, `Some(Some(base))` for
    /// a pair's net (`base`: the name less its polarity suffix, KiCad's
    /// `MatchDpSuffix`).
    pub diff_pair: Option<Option<String>>,
}

impl NetFacts {
    fn effective(&self) -> String {
        self.classes.join(",")
    }
}

#[derive(Clone, Copy)]
struct Item<'a> {
    kind: Kind,
    net: Option<&'a NetFacts>,
    layer: &'a str,
}

#[derive(Clone, Debug)]
enum Value {
    Text(String),
    Number(f64),
    Bool(T3),
    /// A net class: equal to a name that is one of its constituents or the
    /// effective class's name.
    NetClass(Vec<String>),
    /// The net of an item, for `A.Net == B.Net`.
    Net(Option<String>),
    Undefined,
    Unknown,
}

/// KiCad's string comparison: case-insensitive, with `*` and `?` as
/// wildcards when the pattern has them (`WildCompareString`).
fn text_matches(pattern: &str, text: &str) -> bool {
    let pattern = pattern.to_lowercase();
    let text = text.to_lowercase();
    if pattern.contains(['*', '?']) {
        super::wildcard_match(&pattern, &text)
    } else {
        pattern == text
    }
}

struct Context<'a> {
    a: Item<'a>,
    b: Option<Item<'a>>,
    /// Whether A and B are of one net (`A.Net == B.Net`).
    same_net: bool,
}

impl Context<'_> {
    fn item(&self, side: Side) -> Option<Item<'_>> {
        match side {
            Side::A | Side::Both => Some(self.a),
            Side::B => self.b,
        }
    }

    fn property(&self, side: Side, name: &str) -> Value {
        let Some(item) = self.item(side) else {
            return Value::Undefined;
        };
        match name {
            "type" => Value::Text(
                match item.kind {
                    Kind::Track => "Track",
                    Kind::Via => "Via",
                    Kind::SmdPad | Kind::PlatedPad | Kind::NpthPad => "Pad",
                    Kind::Edge => "Graphic",
                }
                .into(),
            ),
            "netclass" => match item.net {
                Some(net) => Value::NetClass(net.classes.clone()),
                None => Value::Undefined,
            },
            "netname" | "net" => match (item.net, name) {
                (Some(net), "net") => Value::Net(Some(net.name.clone())),
                (Some(net), _) => Value::Text(net.name.clone()),
                (None, "net") => Value::Net(None),
                (None, _) => Value::Text(String::new()),
            },
            "layer" => Value::Text(item.layer.into()),
            "padtype" => match item.kind {
                Kind::SmdPad => Value::Text("SMD".into()),
                Kind::PlatedPad => Value::Text("Through-hole".into()),
                Kind::NpthPad => Value::Text("NPTH, mechanical".into()),
                _ => Value::Undefined,
            },
            "viatype" => match item.kind {
                Kind::Via => Value::Text("Through".into()),
                _ => Value::Undefined,
            },
            _ => Value::Unknown,
        }
    }

    fn call(&self, side: Side, name: &str, arguments: &[Ex]) -> Value {
        let text_argument = || match arguments.first() {
            Some(Ex::Text(text)) => Some(text.clone()),
            _ => None,
        };
        let Some(item) = self.item(side) else {
            return Value::Undefined;
        };
        let via = item.kind == Kind::Via;
        match name {
            "isplated" => Value::Bool(T3::of(matches!(item.kind, Kind::Via | Kind::PlatedPad))),
            // The router's vias are through vias.
            "isblindburiedvia" | "ismicrovia" | "isblindvia" | "isburiedvia" => {
                Value::Bool(T3::False)
            }
            "isthroughvia" => Value::Bool(T3::of(via)),
            "hasnetclass" | "innetclass" => match (text_argument(), item.net) {
                (Some(pattern), Some(net)) => Value::Bool(T3::of(
                    net.classes
                        .iter()
                        .any(|class| text_matches(&pattern, class)),
                )),
                (Some(_), None) => Value::Bool(T3::False),
                _ => Value::Unknown,
            },
            "hasexactnetclass" => match (text_argument(), item.net) {
                (Some(pattern), Some(net)) => Value::Bool(T3::of(net.effective() == pattern)),
                (Some(_), None) => Value::Bool(T3::False),
                _ => Value::Unknown,
            },
            // KiCad's `inDiffPair(pattern)`: the net's pair base name (or
            // that less a trailing `_`) matches the pattern.
            "indiffpair" => match (text_argument(), item.net.map(|net| net.diff_pair.as_ref())) {
                (_, None) => Value::Bool(T3::False),
                (_, Some(None)) => Value::Unknown,
                (_, Some(Some(None))) => Value::Bool(T3::False),
                (Some(pattern), Some(Some(Some(base)))) => Value::Bool(T3::of(
                    super::wildcard_match(&pattern, base)
                        || base
                            .strip_suffix('_')
                            .is_some_and(|stem| super::wildcard_match(&pattern, stem)),
                )),
                (None, _) => Value::Unknown,
            },
            "existsonlayer" => match text_argument() {
                Some(pattern) => Value::Bool(T3::of(text_matches(&pattern, item.layer))),
                None => Value::Unknown,
            },
            _ => Value::Unknown,
        }
    }

    fn value(&self, ex: &Ex) -> Value {
        match ex {
            Ex::Property(side, name) => self.property(*side, name),
            Ex::Call(side, name, arguments) => self.call(*side, name, arguments),
            Ex::Text(text) => Value::Text(text.clone()),
            Ex::Number(value) => Value::Number(*value),
            Ex::Null => Value::Undefined,
            Ex::Opaque(_) => Value::Unknown,
            other => Value::Bool(self.truth(other)),
        }
    }

    fn equal(&self, left: &Value, right: &Value) -> T3 {
        match (left, right) {
            (Value::Unknown, _) | (_, Value::Unknown) => T3::Unknown,
            (Value::Undefined, _) | (_, Value::Undefined) => T3::False,
            (Value::Net(a), Value::Net(b)) => T3::of(self.same_net && a.is_some() && b.is_some()),
            (Value::NetClass(a), Value::NetClass(b)) => T3::of(a == b),
            (Value::NetClass(classes), Value::Text(pattern))
            | (Value::Text(pattern), Value::NetClass(classes)) => T3::of(
                classes.iter().any(|class| text_matches(pattern, class))
                    || text_matches(pattern, &classes.join(",")),
            ),
            (Value::Net(Some(name)), Value::Text(pattern))
            | (Value::Text(pattern), Value::Net(Some(name))) => T3::of(text_matches(pattern, name)),
            (Value::Net(None), Value::Text(pattern)) | (Value::Text(pattern), Value::Net(None)) => {
                T3::of(pattern.is_empty())
            }
            (Value::Text(a), Value::Text(b)) => T3::of(text_matches(b, a) || text_matches(a, b)),
            (Value::Number(a), Value::Number(b)) => T3::of((a - b).abs() < 1.0e-9),
            (Value::Bool(a), Value::Bool(b)) => match (a, b) {
                (T3::Unknown, _) | (_, T3::Unknown) => T3::Unknown,
                _ => T3::of(a == b),
            },
            (Value::Bool(t), Value::Number(n)) | (Value::Number(n), Value::Bool(t)) => {
                if *n == 0.0 { t.not() } else { *t }
            }
            _ => T3::False,
        }
    }

    fn truth(&self, ex: &Ex) -> T3 {
        match ex {
            Ex::Or(left, right) => self.truth(left).or(self.truth(right)),
            Ex::And(left, right) => self.truth(left).and(self.truth(right)),
            Ex::Not(inner) => self.truth(inner).not(),
            Ex::Compare(operator, left, right) => {
                // `A.NetName == B.NetName`: whether the items share a net.
                if let (Ex::Property(_, x), Ex::Property(_, y)) = (left.as_ref(), right.as_ref())
                    && matches!(x.as_str(), "net" | "netname")
                    && matches!(y.as_str(), "net" | "netname")
                    && self.b.is_some()
                {
                    match operator {
                        Compare::Equal => return T3::of(self.same_net),
                        Compare::NotEqual => return T3::of(!self.same_net),
                        _ => return T3::Unknown,
                    }
                }
                let (left, right) = (self.value(left), self.value(right));
                match operator {
                    Compare::Equal => self.equal(&left, &right),
                    Compare::NotEqual => match (&left, &right) {
                        (Value::Undefined, _) | (_, Value::Undefined) => T3::False,
                        _ => self.equal(&left, &right).not(),
                    },
                    _ => match (&left, &right) {
                        (Value::Number(a), Value::Number(b)) => T3::of(match operator {
                            Compare::Less => a < b,
                            Compare::LessEqual => a <= b,
                            Compare::Greater => a > b,
                            _ => a >= b,
                        }),
                        (Value::Undefined, _) | (_, Value::Undefined) => T3::False,
                        _ => T3::Unknown,
                    },
                }
            }
            Ex::Number(value) => T3::of(*value != 0.0),
            Ex::Text(_) => T3::Unknown,
            Ex::Null => T3::False,
            other => match self.value(other) {
                Value::Bool(value) => value,
                Value::Number(value) => T3::of(value != 0.0),
                Value::Undefined => T3::False,
                _ => T3::Unknown,
            },
        }
    }
}

/// A condition for an item pair, both ways round, as KiCad evaluates it
/// (`DRC_RULE_CONDITION::EvaluateFor`).
fn pair_truth(ex: &Ex, a: Item, b: Option<Item>, same_net: bool) -> T3 {
    let first = Context { a, b, same_net }.truth(ex);
    let Some(b) = b else {
        return first;
    };
    if first == T3::True {
        return first;
    }
    first.or(Context {
        a: b,
        b: Some(a),
        same_net,
    }
    .truth(ex))
}

/// The atoms of a condition the router cannot evaluate, for the report.
fn opaque_atoms(ex: &Ex, out: &mut BTreeSet<String>) {
    match ex {
        Ex::Or(a, b) | Ex::And(a, b) | Ex::Compare(_, a, b) => {
            opaque_atoms(a, out);
            opaque_atoms(b, out);
        }
        Ex::Not(a) => opaque_atoms(a, out),
        Ex::Property(side, name) => {
            if !matches!(
                name.as_str(),
                "type" | "netclass" | "netname" | "net" | "layer" | "padtype" | "viatype"
            ) {
                out.insert(format!("{}.{name}", side_name(*side)));
            }
        }
        Ex::Call(side, name, _) => {
            if !matches!(
                name.as_str(),
                "isplated"
                    | "isblindburiedvia"
                    | "ismicrovia"
                    | "isblindvia"
                    | "isburiedvia"
                    | "isthroughvia"
                    | "hasnetclass"
                    | "innetclass"
                    | "hasexactnetclass"
                    | "existsonlayer"
            ) {
                out.insert(format!("{}.{name}()", side_name(*side)));
            }
        }
        Ex::Opaque(what) => {
            out.insert(what.clone());
        }
        Ex::Text(_) | Ex::Number(_) | Ex::Null => {}
    }
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::A => "A",
        Side::B => "B",
        Side::Both => "AB",
    }
}

/// The net-dependent leaves of a condition (a net class, name or net
/// compared with a literal, a net class function), whose values on a
/// net decide everything the condition asks of it; and whether it
/// compares the classes of A and B.
fn net_leaves<'e>(ex: &'e Ex, out: &mut Vec<&'e Ex>, compares_classes: &mut bool) {
    match ex {
        Ex::Or(a, b) | Ex::And(a, b) => {
            net_leaves(a, out, compares_classes);
            net_leaves(b, out, compares_classes);
        }
        Ex::Not(a) => net_leaves(a, out, compares_classes),
        Ex::Compare(_, a, b) => {
            let net_property = |ex: &Ex| matches!(ex, Ex::Property(_, name) if matches!(name.as_str(), "netclass" | "netname" | "net"));
            match (a.as_ref(), b.as_ref()) {
                (Ex::Property(_, x), Ex::Property(_, y)) if x == "netclass" && y == "netclass" => {
                    *compares_classes = true
                }
                (x, Ex::Text(_)) | (Ex::Text(_), x) if net_property(x) => out.push(ex),
                _ => {}
            }
        }
        Ex::Call(_, name, _)
            if matches!(
                name.as_str(),
                "hasnetclass" | "innetclass" | "hasexactnetclass"
            ) =>
        {
            out.push(ex)
        }
        _ => {}
    }
}

/// A leaf's value on `net`, the same on either side.
fn leaf_on(leaf: &Ex, net: &NetFacts) -> T3 {
    let facts = Item {
        kind: Kind::Track,
        net: Some(net),
        layer: "",
    };
    let context = Context {
        a: facts,
        b: Some(facts),
        same_net: false,
    };
    let retarget = |ex: &Ex| -> Ex {
        match ex {
            Ex::Property(_, name) => Ex::Property(Side::A, name.clone()),
            Ex::Call(_, name, arguments) => Ex::Call(Side::A, name.clone(), arguments.clone()),
            other => other.clone(),
        }
    };
    match leaf {
        Ex::Compare(operator, a, b) => context.truth(&Ex::Compare(
            *operator,
            Box::new(retarget(a)),
            Box::new(retarget(b)),
        )),
        other => context.truth(&retarget(other)),
    }
}

// ---------------------------------------------------------------------------
// The lowering.

/// What the lowering needs of the board.
#[derive(Clone)]
pub(crate) struct BoardFacts {
    /// The nets with pads.
    pub nets: Vec<NetFacts>,
    /// In the router's order (F.Cu, the inner layers, B.Cu): a layer's
    /// index is its bit in `PairClearance::layers`.
    pub copper_layers: Vec<String>,
    /// The kinds of pad the board has.
    pub pad_kinds: BTreeSet<Kind>,
    /// Each net's rules before the custom rules (in `nets`' order), and
    /// the board's edge, hole and hole-to-hole clearances: a rule that
    /// asks no more than these changes nothing (`no_effect`).
    pub current: Vec<crate::KiCadConnectionRoutingRules>,
    pub board_clearances: [f64; 3],
    /// The neck-down width before the custom rules.
    pub neck_width: f64,
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

/// A clearance between the items of two sets of nets (router
/// `ClearanceRule`): an item of kind A (`ROUTED_KINDS`) of a net of `first`
/// against one of kind B (`OTHER_KINDS`) of a net of `second`, on
/// `layers`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PairClearance {
    /// Indices into `BoardFacts::nets`; `None`: every item, of no net too.
    pub first: Option<Vec<usize>>,
    pub second: Option<Vec<usize>>,
    /// Bit `3 * a + b` for kinds `ROUTED_KINDS[a]` and `OTHER_KINDS[b]`
    /// (pcb_router::rules::kind_bit).
    pub kinds: u16,
    /// Bit per index of `BoardFacts::copper_layers`; all of them for a
    /// rule on every layer.
    pub layers: u32,
    pub clearance: f64,
}

/// The kinds the router lays (A of a clearance) and the kinds they meet
/// (B), in the router's order (pcb_router::rules::TRACK, VIA, PAD).
const ROUTED_KINDS: [Kind; 2] = [Kind::Track, Kind::Via];

/// The rules a .kicad_dru gives the router.
#[derive(Clone, Debug, Default)]
pub(crate) struct Lowered {
    /// Clearance floors per net (the index of `BoardFacts::nets`).
    pub clearance: Vec<f64>,
    /// A clearance floor for every net (and nets without pads).
    pub clearance_all: f64,
    pub width_min: Vec<f64>,
    pub width_min_all: f64,
    pub width_max: Vec<f64>,
    pub width_opt: Vec<Option<f64>>,
    pub via_diameter_min: Vec<f64>,
    pub via_diameter_min_all: f64,
    pub via_diameter_max: Vec<f64>,
    pub via_drill_min: Vec<f64>,
    pub via_drill_min_all: f64,
    pub via_drill_max: Vec<f64>,
    pub annulus_min: Vec<f64>,
    pub annulus_min_all: f64,
    pub edge_clearance: f64,
    pub hole_clearance: f64,
    pub hole_to_hole: f64,
    /// Clearances the per-net values cannot keep exactly: between two
    /// sets of nets, from some item types only (pads to tracks), on some
    /// layers only (router `ClearanceRule`s).
    pub pair_clearances: Vec<PairClearance>,
    pub report: Vec<KiCadCustomRuleReport>,
    /// Routing rules the router does not keep, one line each, for the
    /// verdict.
    pub unmodelled: Vec<String>,
}

/// The project's severity of the findings a constraint makes, with
/// KiCad's defaults (board_design_settings.cpp: hole_to_hole is a
/// warning, the others errors).
pub(crate) fn finding_severity(kind: &str, severities: &BTreeMap<String, String>) -> String {
    let (key, default) = match kind {
        "clearance" => ("clearance", "error"),
        "track_width" => ("track_width", "error"),
        "edge_clearance" => ("copper_edge_clearance", "error"),
        "hole_clearance" => ("hole_clearance", "error"),
        "hole_to_hole" => ("hole_to_hole", "warning"),
        "via_diameter" => ("via_diameter", "error"),
        "hole_size" => ("drill_out_of_range", "error"),
        "annular_width" => ("annular_width", "error"),
        _ => return "error".into(),
    };
    severities
        .get(key)
        // KiCad 8's name for hole_to_hole, which KiCad 10 still reads.
        .or_else(|| {
            (key == "hole_to_hole")
                .then(|| severities.get("hole_near_hole"))
                .flatten()
        })
        .cloned()
        .unwrap_or_else(|| default.into())
}

/// Constraints that do not concern the copper the router lays.
const NOT_ROUTING: [&str; 9] = [
    "silk_clearance",
    "text_height",
    "text_thickness",
    "courtyard_clearance",
    "solder_paste_abs_margin",
    "solder_paste_rel_margin",
    "solder_mask_expansion",
    "track_angle",
    "track_segment_length",
];

fn mm(value: f64) -> String {
    let text = format!("{value:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text} mm")
}

impl Lowered {
    fn new(nets: usize) -> Self {
        Self {
            clearance: vec![0.0; nets],
            width_min: vec![0.0; nets],
            width_max: vec![f64::INFINITY; nets],
            width_opt: vec![None; nets],
            via_diameter_min: vec![0.0; nets],
            via_diameter_max: vec![f64::INFINITY; nets],
            via_drill_min: vec![0.0; nets],
            via_drill_max: vec![f64::INFINITY; nets],
            annulus_min: vec![0.0; nets],
            ..Self::default()
        }
    }

    fn note(&mut self, rule: &Rule, constraint: &Constraint, status: &str, detail: String) {
        if status == "unmodelled" {
            self.unmodelled.push(format!(
                "{} ('{}'{}): {detail}",
                constraint.kind,
                rule.name,
                constraint
                    .min
                    .map(|value| format!(", min {}", mm(value)))
                    .or_else(|| constraint.max.map(|value| format!(", max {}", mm(value))))
                    .unwrap_or_default()
            ));
        }
        self.report.push(KiCadCustomRuleReport {
            rule: rule.name.clone(),
            constraint: constraint.kind.clone(),
            status: status.into(),
            detail,
        });
    }
}

/// The groups of nets a condition cannot tell apart, with their members.
fn net_groups(ex: &Ex, nets: &[NetFacts]) -> Vec<Vec<usize>> {
    let mut leaves = Vec::new();
    let mut compares_classes = false;
    net_leaves(ex, &mut leaves, &mut compares_classes);
    let mut groups: BTreeMap<(Vec<u8>, String), Vec<usize>> = BTreeMap::new();
    for (index, net) in nets.iter().enumerate() {
        let signature = leaves
            .iter()
            .map(|leaf| match leaf_on(leaf, net) {
                T3::True => 1,
                T3::False => 0,
                T3::Unknown => 2,
            })
            .collect();
        let class = if compares_classes {
            net.effective()
        } else {
            String::new()
        };
        groups.entry((signature, class)).or_default().push(index);
    }
    groups.into_values().collect()
}

/// The layers of the board a rule's `(layer ...)` names (`outer`,
/// `inner`, a name, a pattern); all of them without one.
fn rule_layers<'a>(rule: &Rule, copper: &'a [String]) -> Vec<&'a str> {
    copper
        .iter()
        .map(String::as_str)
        .filter(|layer| match rule.layer.as_deref() {
            None => true,
            Some("outer") => matches!(*layer, "F.Cu" | "B.Cu"),
            Some("inner") => !matches!(*layer, "F.Cu" | "B.Cu"),
            Some(pattern) => text_matches(pattern, layer),
        })
        .collect()
}

/// Lowers the rules of a .kicad_dru for a board.
pub(crate) fn lower(
    rules: &[Rule],
    board: &BoardFacts,
    severities: &BTreeMap<String, String>,
) -> Lowered {
    let mut lowered = Lowered::new(board.nets.len());
    let no_net = NetFacts {
        name: String::new(),
        classes: vec!["Default".into()],
        diff_pair: None,
    };
    let conditions: Vec<Option<Ex>> = rules
        .iter()
        .map(|rule| match rule.condition.as_deref() {
            None => Some(Ex::Number(1.0)),
            Some(text) => parse_condition(text).ok(),
        })
        .collect();
    for (index, rule) in rules.iter().enumerate() {
        for constraint in &rule.constraints {
            // KiCad applies the last matching rule of a constraint: a later
            // rule with a minimum of the same constraint takes the pairs it
            // surely matches (JLCPCB's template: "Pad to Track" 0.2 mm and,
            // after it, "Min. Spacing" 0.127 on everything; AMLing2's drone
            // carrier, where KiCad finds no track 0.13 mm from a pad).
            let later: Vec<Later> = if SHADOWED.contains(&constraint.kind.as_str()) {
                rules[index + 1..]
                    .iter()
                    .zip(&conditions[index + 1..])
                    .filter(|(other, _)| {
                        other
                            .constraints
                            .iter()
                            .any(|own| own.kind == constraint.kind && own.min.is_some())
                    })
                    .filter_map(|(other, condition)| {
                        Some((
                            condition.as_ref()?,
                            rule_layers(other, &board.copper_layers),
                        ))
                    })
                    .collect()
            } else {
                Vec::new()
            };
            lower_constraint(
                &mut lowered,
                rule,
                constraint,
                board,
                severities,
                &no_net,
                &later,
            );
        }
    }
    lowered
}

/// Constraints a later rule takes from an earlier one where it surely
/// matches (`lower`).
const SHADOWED: [&str; 4] = [
    "clearance",
    "hole_clearance",
    "edge_clearance",
    "hole_to_hole",
];

/// A later rule's condition and layers (`lower`).
type Later<'a> = (&'a Ex, Vec<&'a str>);

/// The routed kinds (A) and the kinds they meet (B) a constraint is
/// checked between, `None` for B when KiCad checks the item alone.
fn kinds_of(kind: &str, pads: &BTreeSet<Kind>) -> (Vec<Kind>, Option<Vec<Kind>>) {
    let holes: Vec<Kind> = [Kind::Via, Kind::PlatedPad, Kind::NpthPad]
        .into_iter()
        .filter(|kind| *kind == Kind::Via || pads.contains(kind))
        .collect();
    match kind {
        "clearance" => {
            let mut others = vec![Kind::Track, Kind::Via];
            others.extend(pads.iter().copied().filter(|kind| *kind != Kind::NpthPad));
            (vec![Kind::Track, Kind::Via], Some(others))
        }
        "track_width" => (vec![Kind::Track], None),
        "via_diameter" | "hole_size" | "annular_width" => (vec![Kind::Via], None),
        "edge_clearance" => (vec![Kind::Track, Kind::Via], Some(vec![Kind::Edge])),
        "hole_clearance" => (vec![Kind::Track, Kind::Via], Some(holes)),
        "hole_to_hole" => (vec![Kind::Via], Some(holes)),
        _ => (Vec::new(), None),
    }
}

fn lower_constraint(
    lowered: &mut Lowered,
    rule: &Rule,
    constraint: &Constraint,
    board: &BoardFacts,
    severities: &BTreeMap<String, String>,
    no_net: &NetFacts,
    later: &[Later],
) {
    // Whether a later rule surely takes the pair (`lower`); remembered for
    // the report.
    let shadowed_any = std::cell::Cell::new(false);
    let shadowed = |truth: T3, a: Item, b: Option<Item>, same: bool| -> T3 {
        if truth != T3::False
            && later.iter().any(|(condition, layers)| {
                layers.contains(&a.layer) && pair_truth(condition, a, b, same) == T3::True
            })
        {
            shadowed_any.set(true);
            T3::False
        } else {
            truth
        }
    };
    let kind = constraint.kind.as_str();
    if NOT_ROUTING.contains(&kind) {
        lowered.note(
            rule,
            constraint,
            "not_routing",
            "not a rule on routed copper".into(),
        );
        return;
    }
    if let Some(severity) = rule.severity.as_deref().filter(|s| *s != "error") {
        lowered.note(
            rule,
            constraint,
            "skipped",
            format!("the rule's severity is {severity}: its findings are no errors"),
        );
        return;
    }
    let modelled = matches!(
        kind,
        "clearance"
            | "track_width"
            | "via_diameter"
            | "hole_size"
            | "annular_width"
            | "edge_clearance"
            | "hole_clearance"
            | "hole_to_hole"
    );
    let layers = rule_layers(rule, &board.copper_layers);
    if layers.is_empty() {
        lowered.note(
            rule,
            constraint,
            "skipped",
            format!(
                "layer {} is not on this board",
                rule.layer.as_deref().unwrap_or("?")
            ),
        );
        return;
    }
    let condition = match rule.condition.as_deref().map(parse_condition).transpose() {
        Ok(condition) => condition.unwrap_or(Ex::Number(1.0)),
        Err(error) => {
            lowered.note(
                rule,
                constraint,
                if modelled {
                    "unmodelled"
                } else {
                    "not_routing"
                },
                format!("the condition does not parse: {error}"),
            );
            return;
        }
    };
    if kind == "disallow" {
        let words: Vec<&str> = constraint.words.iter().map(String::as_str).collect();
        if words
            .iter()
            .all(|word| matches!(*word, "buried_via" | "micro_via" | "blind_via"))
        {
            lowered.note(
                rule,
                constraint,
                "no_effect",
                format!("{}: the router lays through vias only", words.join(", ")),
            );
        } else if words
            .iter()
            .all(|word| matches!(*word, "footprint" | "text" | "graphic"))
        {
            lowered.note(
                rule,
                constraint,
                "not_routing",
                "placement or graphics".into(),
            );
        } else {
            lowered.note(
                rule,
                constraint,
                "unmodelled",
                format!(
                    "disallow {} where {} (rule areas from rules are not lowered)",
                    words.join(", "),
                    rule.condition.as_deref().unwrap_or("anywhere")
                ),
            );
        }
        return;
    }
    if !modelled {
        let detail = match kind {
            "creepage" => "creepage (surface distance) is not modelled".to_string(),
            "length" | "skew" | "max_uncoupled" | "via_count" => {
                "no length tuning or via counts in the router".into()
            }
            "diff_pair_gap" | "diff_pair_uncoupled" => {
                "differential pairs route coupled only with `diff_pairs` (off)".into()
            }
            "physical_clearance" | "physical_hole_clearance" => {
                "physical clearances (also within a net) are not modelled".into()
            }
            "connection_width" => "a custom connection width is not modelled".into(),
            "thermal_relief_gap"
            | "thermal_spoke_width"
            | "zone_connection"
            | "min_resolved_spokes" => "zone connection rules are KiCad's fill's".into(),
            "assertion" => "assertions are not evaluated".into(),
            "bridged_mask" => "mask bridging rules are not modelled".into(),
            other => format!("{other} is not modelled"),
        };
        // Pour rules and assertions do not stop a route.
        let status = if matches!(
            kind,
            "thermal_relief_gap"
                | "thermal_spoke_width"
                | "zone_connection"
                | "min_resolved_spokes"
        ) || kind == "assertion"
        {
            "not_routing"
        } else {
            "unmodelled"
        };
        lowered.note(rule, constraint, status, detail);
        return;
    }
    let severity = finding_severity(kind, severities);
    if severity != "error" {
        lowered.note(
            rule,
            constraint,
            "skipped",
            format!("the project rates these findings {severity}"),
        );
        return;
    }
    let (a_kinds, b_kinds) = kinds_of(kind, &board.pad_kinds);
    // The groups tell apart what the later rules' conditions do too (a
    // later rule may take some of the nets of one group of this rule).
    let grouping = later.iter().fold(condition.clone(), |all, (other, _)| {
        Ex::And(Box::new(all), Box::new((*other).clone()))
    });
    let groups = net_groups(&grouping, &board.nets);
    let representative = |group: &[usize]| &board.nets[group[0]];
    let mut unknown = false;
    // Pairs, for clearances: which group pairs the rule matches.
    // `tracks`: a track against a track only (a clearance rule that
    // matches no such pair is one on pads or vias alone).
    let truth_kinds =
        |a_net: Option<&NetFacts>, b_net: Option<&NetFacts>, same: bool, tracks: bool| -> T3 {
            let track_only = [Kind::Track];
            let a_kinds: &[Kind] = if tracks { &track_only } else { &a_kinds };
            let b_kinds: Option<&[Kind]> = if tracks {
                Some(&track_only)
            } else {
                b_kinds.as_deref()
            };
            let mut result = T3::False;
            for &layer in &layers {
                for a_kind in a_kinds {
                    let a = Item {
                        kind: *a_kind,
                        net: a_net,
                        layer,
                    };
                    match b_kinds {
                        None => {
                            result = result.or(shadowed(
                                pair_truth(&condition, a, None, false),
                                a,
                                None,
                                false,
                            ))
                        }
                        Some(b_kinds) => {
                            for b_kind in b_kinds {
                                let b_net = match b_kind {
                                    Kind::NpthPad => Some(no_net),
                                    Kind::Edge => None,
                                    _ => b_net,
                                };
                                let b = Item {
                                    kind: *b_kind,
                                    net: b_net,
                                    layer: if *b_kind == Kind::Edge {
                                        "Edge.Cuts"
                                    } else {
                                        layer
                                    },
                                };
                                result = result.or(shadowed(
                                    pair_truth(&condition, a, Some(b), same),
                                    a,
                                    Some(b),
                                    same,
                                ));
                            }
                        }
                    }
                    if result == T3::True {
                        return result;
                    }
                }
            }
            result
        };
    let truth_for = |a_net: Option<&NetFacts>, b_net: Option<&NetFacts>, same: bool| {
        truth_kinds(a_net, b_net, same, false)
    };
    let opaque = || {
        let mut atoms = BTreeSet::new();
        opaque_atoms(&condition, &mut atoms);
        atoms.into_iter().collect::<Vec<_>>().join(", ")
    };
    let narrower = rule.layer.is_some() || rule.condition.is_some();
    match kind {
        "clearance" | "hole_clearance" | "edge_clearance" | "hole_to_hole" => {
            let Some(value) = constraint.min else {
                lowered.note(rule, constraint, "no_effect", "no minimum".into());
                return;
            };
            if kind != "clearance" {
                // One value for the board: does the rule match anything
                // the router lays?
                let mut matches = T3::False;
                for group in &groups {
                    let net = representative(group);
                    let pairs: Vec<(Option<&NetFacts>, bool)> = match kind {
                        // Holes of the same net count for hole to hole.
                        "hole_to_hole" => groups
                            .iter()
                            .map(|other| (Some(representative(other)), false))
                            .chain([(Some(net), true)])
                            .collect(),
                        "edge_clearance" => vec![(None, false)],
                        _ => groups
                            .iter()
                            .map(|other| (Some(representative(other)), false))
                            .collect(),
                    };
                    for (other, same) in pairs {
                        matches = matches.or(truth_for(Some(net), other, same));
                    }
                }
                match matches {
                    T3::Unknown => lowered.note(
                        rule,
                        constraint,
                        "unmodelled",
                        format!("the condition depends on {}", opaque()),
                    ),
                    T3::False => lowered.note(
                        rule,
                        constraint,
                        "no_effect",
                        if shadowed_any.get() {
                            "a later rule takes everything it matches (KiCad applies the last matching rule)".into()
                        } else {
                            "matches nothing the router lays on this board".into()
                        },
                    ),
                    T3::True => {
                        let (target, before) = match kind {
                            "edge_clearance" => {
                                (&mut lowered.edge_clearance, board.board_clearances[0])
                            }
                            "hole_clearance" => {
                                (&mut lowered.hole_clearance, board.board_clearances[1])
                            }
                            _ => (&mut lowered.hole_to_hole, board.board_clearances[2]),
                        };
                        *target = target.max(value);
                        if value <= before + 1.0e-9 {
                            lowered.note(
                                rule,
                                constraint,
                                "no_effect",
                                format!(
                                    "{} {}: the board setup keeps {}",
                                    kind.replace('_', " "),
                                    mm(value),
                                    mm(before)
                                ),
                            );
                            return;
                        }
                        lowered.note(
                            rule,
                            constraint,
                            if narrower { "approximated" } else { "applied" },
                            format!(
                                "{} {} for the whole board{}",
                                kind.replace('_', " "),
                                mm(value),
                                if narrower {
                                    " (the router has one value; the rule's layer or condition is narrower)"
                                } else {
                                    ""
                                }
                            ),
                        );
                    }
                }
                return;
            }
            // Clearances: per group pair. A group of one net has no pair
            // of its own, but whether it would match itself tells a rule
            // on one net (`A.NetClass == 'HV'`) from one on a pair.
            let eligible = |i: usize, j: usize| i != j || groups[i].len() >= 2;
            let mut truth = BTreeMap::new();
            let mut matched = BTreeSet::new();
            for (i, gi) in groups.iter().enumerate() {
                for (j, gj) in groups.iter().enumerate().skip(i) {
                    let value =
                        truth_for(Some(representative(gi)), Some(representative(gj)), false);
                    truth.insert((i, j), value);
                    if !eligible(i, j) {
                        continue;
                    }
                    match value {
                        T3::True => {
                            matched.insert((i, j));
                        }
                        T3::Unknown => unknown = true,
                        T3::False => {}
                    }
                }
            }
            if unknown {
                lowered.note(
                    rule,
                    constraint,
                    "unmodelled",
                    format!("the condition depends on {}", opaque()),
                );
                return;
            }
            if matched.is_empty() {
                lowered.note(
                    rule,
                    constraint,
                    "no_effect",
                    if shadowed_any.get() {
                        "a later rule takes every pair it matches (KiCad applies the last matching rule)".into()
                    } else {
                        "matches no pair of routed items of two nets on this board".into()
                    },
                );
                return;
            }
            let binds = |pairs: &BTreeSet<(usize, usize)>| {
                pairs.iter().any(|&(i, j)| {
                    groups[i]
                        .iter()
                        .chain(groups[j].iter())
                        .any(|&net| board.current[net].clearance_mm < value - 1.0e-9)
                })
            };
            // Per pair of groups (A's, B's): per kind of A (a track, a via)
            // and of B (a track, a via, a pad), the layers the rule holds on.
            let all_layers: u32 = if board.copper_layers.len() >= 32 {
                u32::MAX
            } else {
                (1u32 << board.copper_layers.len()) - 1
            };
            let pad_kinds: Vec<Kind> = [Kind::SmdPad, Kind::PlatedPad]
                .into_iter()
                .filter(|kind| board.pad_kinds.contains(kind))
                .collect();
            let other_kinds: [Vec<Kind>; 3] = [vec![Kind::Track], vec![Kind::Via], pad_kinds];
            // Whether the rule tells SMD from plated pads apart somewhere
            // (the router's pads are one kind: it keeps the rule from both).
            let mut pad_types_differ = false;
            let mut masks = |a: usize, b: usize| -> [[u32; 3]; 2] {
                let mut result = [[0u32; 3]; 2];
                for (ai, a_kind) in ROUTED_KINDS.iter().enumerate() {
                    for (bi, b_kinds) in other_kinds.iter().enumerate() {
                        let mut per_kind = Vec::new();
                        for b_kind in b_kinds {
                            let mut mask = 0u32;
                            for (index, layer) in board.copper_layers.iter().enumerate() {
                                if !layers.contains(&layer.as_str()) {
                                    continue;
                                }
                                let a_item = Item {
                                    kind: *a_kind,
                                    net: Some(representative(&groups[a])),
                                    layer,
                                };
                                let b_item = Item {
                                    kind: *b_kind,
                                    net: Some(representative(&groups[b])),
                                    layer,
                                };
                                let truth = shadowed(
                                    pair_truth(&condition, a_item, Some(b_item), false),
                                    a_item,
                                    Some(b_item),
                                    false,
                                );
                                // (Unknown where the whole was known: kept.)
                                if truth != T3::False {
                                    mask |= 1u32.checked_shl(index as u32).unwrap_or(0);
                                }
                            }
                            per_kind.push(mask);
                        }
                        if per_kind.windows(2).any(|pair| pair[0] != pair[1]) {
                            pad_types_differ = true;
                        }
                        result[ai][bi] = per_kind.into_iter().fold(0, |all, mask| all | mask);
                    }
                }
                result
            };
            let mut tables = BTreeMap::new();
            for &(i, j) in &matched {
                tables.insert((i, j), masks(i, j));
                if i != j {
                    tables.insert((j, i), masks(j, i));
                }
            }
            let uniform = tables.values().all(|table| {
                table.iter().all(|row| {
                    row.iter()
                        .zip(&other_kinds)
                        .all(|(mask, kinds)| kinds.is_empty() || *mask == all_layers)
                })
            });
            // Groups that match every group, themselves included: the
            // rule is theirs against anything, and raising them keeps
            // every matched pair, as KiCad's larger-of-two does. Failing
            // that, groups that match every group they have a pair with
            // on this board (a single net needs no clearance to itself).
            let covers = |set: &BTreeSet<usize>| {
                matched
                    .iter()
                    .all(|(i, j)| set.contains(i) || set.contains(j))
            };
            let star: BTreeSet<usize> = (0..groups.len())
                .filter(|&i| (0..groups.len()).all(|j| truth[&(i.min(j), i.max(j))] == T3::True))
                .collect();
            let present: BTreeSet<usize> = (0..groups.len())
                .filter(|&i| {
                    (0..groups.len())
                        .filter(|&j| eligible(i, j))
                        .all(|j| matched.contains(&(i.min(j), i.max(j))))
                })
                .collect();
            let raise = if !uniform {
                None
            } else if covers(&star) {
                Some(star)
            } else if covers(&present) {
                Some(present)
            } else {
                None
            };
            let Some(raise) = raise else {
                // A rule between two sets of nets (ChengminLi's flyback:
                // 2.5 mm primary to secondary), from some item types only
                // (JLCPCB's "Pad to Track": 0.2 mm from pads to tracks over
                // classes of 0.127 on AMLing2's drone carrier), or on some
                // layers only: one clearance a net would keep it from
                // every net, every item, on every layer, so it is a pair
                // rule of the router (`pcb_router::ClearanceRule`).
                let sides: BTreeSet<usize> = matched.iter().flat_map(|(i, j)| [*i, *j]).collect();
                let nets: usize = sides.iter().map(|g| groups[*g].len()).sum();
                if !binds(&matched) {
                    lowered.note(
                        rule,
                        constraint,
                        "no_effect",
                        format!(
                            "clearance {} {}: the net classes keep at least as much",
                            mm(value),
                            if sides.len() > 1 {
                                "between two sets of nets"
                            } else {
                                "on some item kinds or layers"
                            }
                        ),
                    );
                    return;
                }
                let side = |group: usize| (groups.len() > 1).then(|| groups[group].clone());
                let mut kinds_seen = 0u16;
                let mut layers_seen = 0u32;
                for (&(a, b), table) in &tables {
                    let mut by_layers: BTreeMap<u32, u16> = BTreeMap::new();
                    for (ai, row) in table.iter().enumerate() {
                        for (bi, mask) in row.iter().enumerate() {
                            if *mask != 0 {
                                *by_layers.entry(*mask).or_default() |= 1 << (ai * 3 + bi);
                            }
                        }
                    }
                    for (mask, kinds) in by_layers {
                        kinds_seen |= kinds;
                        layers_seen |= mask;
                        let pair = PairClearance {
                            first: side(a),
                            second: side(b),
                            kinds,
                            layers: mask,
                            clearance: value,
                        };
                        if !lowered.pair_clearances.contains(&pair) {
                            lowered.pair_clearances.push(pair);
                        }
                    }
                }
                let kind_names = ["track", "via", "pad"];
                let mut kinds_text: Vec<String> = Vec::new();
                for ai in 0..2 {
                    for bi in 0..3 {
                        if kinds_seen & (1 << (ai * 3 + bi)) != 0 {
                            kinds_text.push(format!("{} to {}", kind_names[ai], kind_names[bi]));
                        }
                    }
                }
                let layers_text = if layers_seen == all_layers {
                    "every layer".to_string()
                } else {
                    board
                        .copper_layers
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| {
                            layers_seen & 1u32.checked_shl(*index as u32).unwrap_or(0) != 0
                        })
                        .map(|(_, name)| name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let mut detail = format!(
                    "clearance {} as a pair rule: {} ({}; {})",
                    mm(value),
                    if groups.len() == 1 {
                        "every net".to_string()
                    } else if sides.len() > 1 {
                        format!("between sets of nets, {nets} nets")
                    } else {
                        format!("within a set of {nets} nets")
                    },
                    kinds_text.join(", "),
                    layers_text
                );
                if pad_types_differ {
                    detail.push_str(" (SMD and plated pads alike: the router has one pad kind)");
                }
                lowered.note(
                    rule,
                    constraint,
                    if pad_types_differ {
                        "approximated"
                    } else {
                        "applied"
                    },
                    detail,
                );
                return;
            };
            let every = raise.len() == groups.len();
            let raised_nets: usize = raise.iter().map(|g| groups[*g].len()).sum();
            let raises = raise
                .iter()
                .flat_map(|g| groups[*g].iter())
                .any(|&net| board.current[net].clearance_mm < value - 1.0e-9);
            if every {
                lowered.clearance_all = lowered.clearance_all.max(value);
            } else {
                for g in &raise {
                    for &net in &groups[*g] {
                        lowered.clearance[net] = lowered.clearance[net].max(value);
                    }
                }
            }
            let status = if !narrower_than_nets(&condition, rule) {
                "applied"
            } else {
                "approximated"
            };
            let mut detail = format!(
                "clearance {} on {}",
                mm(value),
                if every {
                    "every net".to_string()
                } else {
                    format!("{raised_nets} of {} nets", board.nets.len())
                }
            );
            if !raises {
                detail.push_str(": the net classes keep at least as much");
                lowered.note(rule, constraint, "no_effect", detail);
                return;
            }
            if status == "approximated" {
                detail.push_str(" (on every layer and against every item type: the router has one clearance a net)");
            }
            lowered.note(rule, constraint, status, detail);
        }
        _ => {
            // Unary: tracks or vias of a net.
            let mut applies = Vec::new();
            for group in &groups {
                match truth_for(Some(representative(group)), None, false) {
                    T3::True => applies.extend(group.iter().copied()),
                    T3::Unknown => unknown = true,
                    T3::False => {}
                }
            }
            if unknown {
                lowered.note(
                    rule,
                    constraint,
                    "unmodelled",
                    format!("the condition depends on {}", opaque()),
                );
                return;
            }
            if applies.is_empty() {
                lowered.note(
                    rule,
                    constraint,
                    "no_effect",
                    format!(
                        "matches no routed {} on this board",
                        if kind == "track_width" {
                            "track"
                        } else {
                            "via"
                        }
                    ),
                );
                return;
            }
            let every = applies.len() == board.nets.len();
            let mut parts = Vec::new();
            let set = |values: &mut Vec<f64>, all: Option<&mut f64>, value: f64, max: bool| {
                if every && let Some(all) = all {
                    *all = all.max(value);
                }
                for &net in &applies {
                    values[net] = if max {
                        values[net].min(value)
                    } else {
                        values[net].max(value)
                    };
                }
            };
            match kind {
                "track_width" => {
                    if let Some(min) = constraint.min {
                        set(
                            &mut lowered.width_min,
                            Some(&mut lowered.width_min_all),
                            min,
                            false,
                        );
                        parts.push(format!("width at least {}", mm(min)));
                    }
                    if let Some(max) = constraint.max {
                        set(&mut lowered.width_max, None, max, true);
                        parts.push(format!("at most {}", mm(max)));
                    }
                    if let Some(opt) = constraint.opt {
                        for &net in &applies {
                            lowered.width_opt[net] = Some(opt);
                        }
                        parts.push(format!("routed at {}", mm(opt)));
                    }
                }
                "via_diameter" => {
                    if let Some(min) = constraint.min {
                        set(
                            &mut lowered.via_diameter_min,
                            Some(&mut lowered.via_diameter_min_all),
                            min,
                            false,
                        );
                        parts.push(format!("via diameter at least {}", mm(min)));
                    }
                    if let Some(max) = constraint.max {
                        set(&mut lowered.via_diameter_max, None, max, true);
                        parts.push(format!("at most {}", mm(max)));
                    }
                }
                "hole_size" => {
                    if let Some(min) = constraint.min {
                        set(
                            &mut lowered.via_drill_min,
                            Some(&mut lowered.via_drill_min_all),
                            min,
                            false,
                        );
                        parts.push(format!("via drill at least {}", mm(min)));
                    }
                    if let Some(max) = constraint.max {
                        set(&mut lowered.via_drill_max, None, max, true);
                        parts.push(format!("at most {}", mm(max)));
                    }
                }
                _ => {
                    if let Some(min) = constraint.min {
                        set(
                            &mut lowered.annulus_min,
                            Some(&mut lowered.annulus_min_all),
                            min,
                            false,
                        );
                        parts.push(format!("via annular ring at least {}", mm(min)));
                    }
                }
            }
            if parts.is_empty() {
                lowered.note(rule, constraint, "no_effect", "no limit".into());
                return;
            }
            let changes = applies.iter().any(|&net| {
                let now = &board.current[net];
                let (value, drill) = match kind {
                    "track_width" => (now.trace_width_mm, 0.0),
                    "via_diameter" => (now.via_size_mm, 0.0),
                    "hole_size" => (now.via_drill_mm, 0.0),
                    _ => (now.via_size_mm, now.via_drill_mm),
                };
                let min = constraint.min.map_or(0.0, |min| {
                    if kind == "annular_width" {
                        drill + 2.0 * min
                    } else {
                        min
                    }
                });
                value < min - 1.0e-9
                    || constraint.max.is_some_and(|max| value > max + 1.0e-9)
                    || (kind == "track_width"
                        && constraint
                            .opt
                            .is_some_and(|opt| (value - opt).abs() > 1.0e-9))
            });
            let mut detail = format!(
                "{} on {}",
                parts.join(", "),
                if every {
                    "every net".to_string()
                } else {
                    format!("{} of {} nets", applies.len(), board.nets.len())
                }
            );
            let approximated = rule.layer.is_some() && layers.len() < board.copper_layers.len();
            if approximated {
                detail.push_str(" (on every layer: the router has one width a net)");
            }
            // A minimum on some nets floors their neck-downs and their
            // narrow-signal width as well (router `width_floor`).
            let floors_net_necks = kind == "track_width"
                && !every
                && constraint
                    .min
                    .is_some_and(|min| min > board.neck_width + 1.0e-9);
            if floors_net_necks {
                detail.push_str(&format!(
                    "; their neck-downs rise from {} to it",
                    mm(board.neck_width)
                ));
            }
            // A board-wide minimum also floors the neck-downs.
            let floors_necks = kind == "track_width"
                && every
                && constraint
                    .min
                    .is_some_and(|min| min > board.neck_width + 1.0e-9);
            if floors_necks {
                detail.push_str(&format!(
                    "; the neck-down width {} rises to it",
                    mm(board.neck_width)
                ));
            }
            if !changes && !floors_necks && !floors_net_necks {
                detail.push_str(": the net classes and the board setup keep it");
                lowered.note(rule, constraint, "no_effect", detail);
                return;
            }
            lowered.note(
                rule,
                constraint,
                if approximated {
                    "approximated"
                } else {
                    "applied"
                },
                detail,
            );
        }
    }
}

/// Whether a clearance rule asks less than every layer and every routed
/// item type, which the router's per-net clearance cannot tell apart.
fn narrower_than_nets(condition: &Ex, rule: &Rule) -> bool {
    fn item_atoms(ex: &Ex) -> bool {
        match ex {
            Ex::Or(a, b) | Ex::And(a, b) | Ex::Compare(_, a, b) => item_atoms(a) || item_atoms(b),
            Ex::Not(a) => item_atoms(a),
            Ex::Property(_, name) => {
                matches!(name.as_str(), "type" | "layer" | "padtype" | "viatype")
            }
            Ex::Call(_, name, _) => !matches!(
                name.as_str(),
                "hasnetclass" | "innetclass" | "hasexactnetclass"
            ),
            _ => false,
        }
    }
    rule.layer.is_some() || item_atoms(condition)
}

// ---------------------------------------------------------------------------
// Differential pairs.

/// The values of the one rule KiCad applies to an item for a constraint
/// (`track_constraint`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Picked {
    pub rule: String,
    pub min: Option<f64>,
    pub opt: Option<f64>,
    pub max: Option<f64>,
}

/// Whether a custom rule surely disallows a track of `net` on `layer`
/// (`(constraint disallow track)` with a condition that surely holds;
/// ducktop2's "solid ground layers": no track but GND's on In1.Cu).
pub(crate) fn track_disallowed(
    rules: &[Rule],
    net: &NetFacts,
    layer: &str,
    copper: &[String],
) -> bool {
    let item = Item {
        kind: Kind::Track,
        net: Some(net),
        layer,
    };
    rules.iter().any(|rule| {
        rule.severity.as_deref() != Some("ignore")
            && rule_layers(rule, copper).contains(&layer)
            && rule.constraints.iter().any(|constraint| {
                constraint.kind == "disallow" && constraint.words.iter().any(|word| word == "track")
            })
            && match rule.condition.as_deref() {
                None => true,
                Some(text) => parse_condition(text).is_ok_and(|condition| {
                    Context {
                        a: item,
                        b: None,
                        same_net: false,
                    }
                    .truth(&condition)
                        == T3::True
                }),
            }
    })
}

/// The rule KiCad applies to a track of `net` on `layer` for a constraint
/// `kind` checked on the item alone (track width, diff pair gap, uncoupled
/// length): the last rule whose `(layer ...)` holds there and whose
/// condition surely holds for it (`EvalRules`: the last match wins).
/// Rules whose condition is unknown for the track, that do not parse or
/// are rated `ignore` are passed over.
pub(crate) fn track_constraint(
    rules: &[Rule],
    net: &NetFacts,
    layer: &str,
    copper: &[String],
    kind: &str,
) -> Option<Picked> {
    let item = Item {
        kind: Kind::Track,
        net: Some(net),
        layer,
    };
    rules.iter().rev().find_map(|rule| {
        if rule.severity.as_deref() == Some("ignore") || !rule_layers(rule, copper).contains(&layer)
        {
            return None;
        }
        let constraint = rule
            .constraints
            .iter()
            .rev()
            .find(|constraint| constraint.kind == kind)?;
        let holds = match rule.condition.as_deref() {
            None => T3::True,
            Some(text) => {
                let condition = parse_condition(text).ok()?;
                Context {
                    a: item,
                    b: None,
                    same_net: false,
                }
                .truth(&condition)
            }
        };
        (holds == T3::True).then(|| Picked {
            rule: rule.name.clone(),
            min: constraint.min,
            opt: constraint.opt,
            max: constraint.max,
        })
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn conditions_past_the_depth_limit_are_refused_and_chains_stay_shallow() {
        // `((((` and `!!!!` by the thousand overflowed the stack until
        // 2026-10-21.
        for text in [
            format!(
                "{}A.NetName == 'X'{}",
                "(".repeat(100_000),
                ")".repeat(100_000)
            ),
            format!("{}(A.NetName == 'X')", "!".repeat(100_000)),
            format!("{}1", "-".repeat(100_000)),
            format!(
                "A.insideArea({}'X'{})",
                "(".repeat(10_000),
                ")".repeat(10_000)
            ),
        ] {
            let error = parse_condition(&text).unwrap_err();
            assert!(error.contains("nests deeper than 200 levels"), "{error}");
        }
        // Within the limit, as before.
        let nested = format!("{}A.NetName == 'X'{}", "(".repeat(150), ")".repeat(150));
        assert!(parse_condition(&nested).is_ok());
        // Ten thousand alternatives: a tree of depth 15, the same truth as
        // read left to right.
        let chain = (0..10_000)
            .map(|index| format!("A.NetName == 'N{index}'"))
            .collect::<Vec<_>>()
            .join(" || ");
        let ex = parse_condition(&chain).unwrap();
        fn depth(ex: &Ex) -> usize {
            match ex {
                Ex::Or(a, b) | Ex::And(a, b) => 1 + depth(a).max(depth(b)),
                _ => 1,
            }
        }
        assert_eq!(depth(&ex), 15);
        let mut leaves = Vec::new();
        net_leaves(&ex, &mut leaves, &mut false);
        assert_eq!(leaves.len(), 10_000);
        // Three operands nest as before: `(a || b) || c`.
        assert_eq!(
            parse_condition("A.Type == 'via' || A.Type == 'pad' || A.Type == 'track'").unwrap(),
            Ex::Or(
                Box::new(parse_condition("A.Type == 'via' || A.Type == 'pad'").unwrap()),
                Box::new(parse_condition("A.Type == 'track'").unwrap())
            )
        );
        // A rule file nested past the reader's limit.
        let deep = format!("(version 1){}{}", "(".repeat(5_000), ")".repeat(5_000));
        assert!(sexps(&deep).unwrap_err().contains("nested deeper than"));
        // A length past f64's range is no length.
        assert_eq!(length_mm("1e999mm"), None);
        assert_eq!(length_mm("0.2mm"), Some(0.2));
    }

    use super::*;

    fn nets(entries: &[(&str, &[&str])]) -> Vec<NetFacts> {
        entries
            .iter()
            .map(|(name, classes)| NetFacts {
                name: name.to_string(),
                classes: classes.iter().map(|class| class.to_string()).collect(),
                diff_pair: None,
            })
            .collect()
    }

    fn board(entries: &[(&str, &[&str])]) -> BoardFacts {
        BoardFacts {
            nets: nets(entries),
            copper_layers: vec!["F.Cu".into(), "B.Cu".into()],
            pad_kinds: [Kind::SmdPad, Kind::PlatedPad, Kind::NpthPad].into(),
            current: vec![
                crate::KiCadConnectionRoutingRules {
                    trace_width_mm: 0.1,
                    clearance_mm: 0.1,
                    via_size_mm: 0.4,
                    via_drill_mm: 0.2,
                };
                entries.len()
            ],
            board_clearances: [0.0; 3],
            neck_width: 0.0,
        }
    }

    fn lowered(text: &str, board: &BoardFacts) -> Lowered {
        lower(&parse_rules(text).unwrap(), board, &BTreeMap::new())
    }

    #[test]
    fn rule_files_parse_with_comments_units_and_escapes() {
        let rules = parse_rules(
            r#"(version 1)
# a comment (with a parenthesis
(rule "Track width, outer"
	(layer outer)
	(condition "A.Type == 'track' && A.NetName == \"/USB_D+\"")
	(constraint track_width (min 0.127mm) (opt 8mil) (max 0.01in)))
(rule HV (severity warning)
  (constraint clearance (min 2)) (constraint disallow via track))"#,
        )
        .unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].name, "Track width, outer");
        assert_eq!(rules[0].layer.as_deref(), Some("outer"));
        assert_eq!(
            rules[0].condition.as_deref(),
            Some("A.Type == 'track' && A.NetName == \"/USB_D+\"")
        );
        let width = &rules[0].constraints[0];
        assert_eq!(width.kind, "track_width");
        assert_eq!(width.min, Some(0.127));
        assert!((width.opt.unwrap() - 0.2032).abs() < 1e-9);
        assert!((width.max.unwrap() - 0.254).abs() < 1e-9);
        assert_eq!(rules[1].severity.as_deref(), Some("warning"));
        assert_eq!(rules[1].constraints[0].min, Some(2.0));
        assert_eq!(rules[1].constraints[1].words, ["via", "track"]);
    }

    #[test]
    fn a_net_class_clearance_raises_that_class_only() {
        // KiCad's netclass comparison is case-insensitive.
        let board = board(&[
            ("HV+", &["HIGH_V"]),
            ("HV-", &["HIGH_V"]),
            ("SDA", &["Default"]),
        ]);
        let rules = lowered(
            "(rule hv (condition \"A.NetClass == 'high_v'\") (constraint clearance (min 1.5mm)))",
            &board,
        );
        assert_eq!(rules.clearance, [1.5, 1.5, 0.0]);
        assert_eq!(rules.clearance_all, 0.0);
        assert_eq!(rules.report[0].status, "applied");
    }

    #[test]
    fn a_rule_on_every_pair_raises_every_net() {
        let board = board(&[("A", &["Default"]), ("B", &["Default"])]);
        let rules = lowered(
            "(rule all (condition \"A.NetClass == 'default' && B.NetClass == 'Default'\") (constraint clearance (min 0.205mm)))",
            &board,
        );
        assert_eq!(rules.clearance_all, 0.205);
        assert_eq!(rules.report[0].status, "applied");
    }

    #[test]
    fn a_rule_between_two_classes_is_a_pair_rule_unless_one_is_every_other_net() {
        let board = board(&[
            ("HV", &["HV"]),
            ("GND_PRE", &["PRE"]),
            ("SDA", &["Default"]),
            ("SCL", &["Default"]),
        ]);
        let rules = lowered(
            "(rule iso (condition \"A.NetClass == 'HV' && B.NetClass == 'PRE'\") (constraint clearance (min 4mm)))",
            &board,
        );
        // One clearance a net would keep 4 mm from every net: a pair rule
        // between the two sets instead, every item kind, every layer,
        // both ways round.
        assert_eq!(rules.report[0].status, "applied");
        assert!(
            rules.report[0]
                .detail
                .contains("between sets of nets, 2 nets")
        );
        assert_eq!(rules.clearance, [0.0; 4]);
        assert_eq!(rules.clearance_all, 0.0);
        assert!(rules.unmodelled.is_empty());
        assert_eq!(rules.pair_clearances.len(), 2);
        for pair in &rules.pair_clearances {
            assert_eq!(pair.kinds, 0b111_111);
            assert_eq!(pair.layers, 0b11);
            assert_eq!(pair.clearance, 4.0);
        }
        type Sides = (Option<Vec<usize>>, Option<Vec<usize>>);
        let sides: BTreeSet<Sides> = rules
            .pair_clearances
            .iter()
            .map(|pair| (pair.first.clone(), pair.second.clone()))
            .collect();
        assert_eq!(
            sides,
            [
                (Some(vec![0]), Some(vec![1])),
                (Some(vec![1]), Some(vec![0]))
            ]
            .into()
        );
        // Against everything else, a class's rule is the class's.
        let rules = lowered(
            "(rule iso (condition \"A.NetClass == 'HV' && B.NetClass != 'HV'\") (constraint clearance (min 4mm)))",
            &board,
        );
        assert_eq!(rules.clearance, [4.0, 0.0, 0.0, 0.0]);
        assert!(rules.pair_clearances.is_empty());
        // Looser than the classes: nothing to keep.
        let rules = lowered(
            "(rule iso (condition \"A.NetClass == 'HV' && B.NetClass == 'PRE'\") (constraint clearance (min 0.05mm)))",
            &board,
        );
        assert_eq!(rules.report[0].status, "no_effect");
        assert!(rules.pair_clearances.is_empty());
    }

    #[test]
    fn net_names_match_with_wildcards_and_net_identity() {
        let board = board(&[
            ("/USB_D+", &["Default"]),
            ("/USB_D-", &["Default"]),
            ("GND", &["Default"]),
        ]);
        let rules = lowered(
            "(rule usb (condition \"A.NetName == '/USB*' && A.Net != B.Net\") (constraint clearance (min 0.3)))",
            &board,
        );
        assert_eq!(rules.clearance, [0.3, 0.3, 0.0]);
        // A rule between items of one net never binds a clearance.
        let same = lowered(
            "(rule same (condition \"A.Net == B.Net\") (constraint clearance (min 0.3)))",
            &board,
        );
        assert_eq!(same.report[0].status, "no_effect");
        assert_eq!(same.clearance_all, 0.0);
    }

    #[test]
    fn item_types_and_layers_are_pair_rules() {
        let board = board(&[("A", &["Default"]), ("B", &["Default"])]);
        let rules = lowered(
            "(rule tp (layer outer) (condition \"A.Type == 'pad' && B.Type == 'track' && A.Net != B.Net\") (constraint clearance (min 0.2)))
             (rule pp (condition \"A.Type == 'Pad' && B.Type == 'Pad'\") (constraint clearance (min 0.5)))
             (rule in (layer inner) (constraint clearance (min 0.9)))",
            &board,
        );
        // Pad to track alone (JLCPCB's): a pair rule of tracks against
        // pads, on every net (no side named), not a clearance a net would
        // also hold between tracks.
        assert_eq!(rules.clearance_all, 0.0);
        assert_eq!(rules.report[0].status, "applied");
        assert_eq!(
            rules.pair_clearances,
            [PairClearance {
                first: None,
                second: None,
                kinds: 1 << 2,
                layers: 0b11,
                clearance: 0.2,
            }]
        );
        let tracks = lowered(
            "(rule tt (layer outer) (condition \"A.Type == 'track' && B.Type == A.Type\") (constraint clearance (min 0.2)))",
            &board,
        );
        assert_eq!(tracks.clearance_all, 0.0);
        assert_eq!(tracks.report[0].status, "applied");
        assert_eq!(tracks.pair_clearances[0].kinds, 1);
        // Pad to pad is the placement's, not the router's.
        assert_eq!(rules.report[1].status, "no_effect");
        // A two-layer board has no inner layer.
        assert_eq!(rules.report[2].status, "skipped");
        // On four layers: the inner layers only.
        let mut four = board.clone();
        four.copper_layers = vec![
            "F.Cu".into(),
            "In1.Cu".into(),
            "In2.Cu".into(),
            "B.Cu".into(),
        ];
        let inner = lowered(
            "(rule in (layer inner) (constraint clearance (min 0.9)))",
            &four,
        );
        assert_eq!(inner.clearance_all, 0.0);
        assert_eq!(inner.report[0].status, "applied");
        assert_eq!(inner.pair_clearances[0].layers, 0b0110);
        assert_eq!(inner.pair_clearances[0].kinds, 0b111_111);
        // An SMD pad rule holds for plated pads too (one pad kind).
        let smd = lowered(
            "(rule smd (condition \"A.Pad_Type == 'SMD' && B.Type == 'track'\") (constraint clearance (min 0.3)))",
            &board,
        );
        assert_eq!(smd.report[0].status, "approximated");
        assert_eq!(smd.pair_clearances[0].kinds, 1 << 2);
    }

    #[test]
    fn a_later_rule_takes_the_pairs_it_surely_matches() {
        let board = board(&[("A", &["Default"]), ("B", &["Default"]), ("HV", &["HV"])]);
        // JLCPCB's template: "Pad to Track" 0.2, then 0.127 on everything,
        // which KiCad applies (the last matching rule).
        let rules = lowered(
            "(rule pt (condition \"A.Type == 'Pad' && B.Type == 'Track'\") (constraint clearance (min 0.2)))
             (rule all (constraint clearance (min 0.127)))",
            &board,
        );
        assert_eq!(rules.report[0].status, "no_effect");
        assert!(rules.report[0].detail.contains("later rule"));
        assert!(rules.pair_clearances.is_empty());
        // The other way round, the pad rule is the last one for pads.
        let rules = lowered(
            "(rule all (constraint clearance (min 0.127)))
             (rule pt (condition \"A.Type == 'Pad' && B.Type == 'Track'\") (constraint clearance (min 0.2)))",
            &board,
        );
        assert_eq!(rules.report[1].status, "applied");
        // (The general rule then holds for every pair but tracks to pads.)
        assert_eq!(rules.pair_clearances.len(), 2);
        assert!(
            rules
                .pair_clearances
                .iter()
                .any(|pair| pair.clearance == 0.2 && pair.kinds == 1 << 2)
        );
        assert!(
            rules
                .pair_clearances
                .iter()
                .any(|pair| pair.clearance == 0.127 && pair.kinds == 0b111_111 & !(1 << 2))
        );
        // A later rule on some nets takes their pairs only: the general
        // 0.3 holds between A and B, not with HV.
        let rules = lowered(
            "(rule all (constraint clearance (min 0.3)))
             (rule hv (condition \"A.NetClass == 'HV'\") (constraint clearance (min 0.05)))",
            &board,
        );
        assert_eq!(rules.clearance_all, 0.0);
        assert_eq!(rules.report[0].status, "applied");
        let pair = &rules.pair_clearances[0];
        assert_eq!(pair.clearance, 0.3);
        assert_eq!(pair.first, Some(vec![0, 1]));
        assert_eq!(pair.second, Some(vec![0, 1]));
    }

    #[test]
    fn conditions_on_areas_or_parts_are_unmodelled_and_named() {
        let board = board(&[("L1", &["Default"]), ("FB", &["Default"])]);
        let rules = lowered(
            "(rule keep (condition \"A.insideCourtyard('L1') && B.Type == 'track'\") (constraint clearance (min 20mm)))
             (rule creep (constraint creepage (min 4mm)))
             (rule len (condition \"A.NetClass == 'DDR'\") (constraint length (min 10mm) (max 20mm)))",
            &board,
        );
        assert_eq!(rules.clearance_all, 0.0);
        assert_eq!(rules.clearance, [0.0, 0.0]);
        let statuses: Vec<&str> = rules.report.iter().map(|r| r.status.as_str()).collect();
        assert_eq!(statuses, ["unmodelled", "unmodelled", "unmodelled"]);
        assert!(rules.report[0].detail.contains("A.insidecourtyard()"));
        assert_eq!(rules.unmodelled.len(), 3);
        assert!(rules.unmodelled[1].starts_with("creepage ('creep', min 4 mm)"));
    }

    #[test]
    fn widths_vias_and_holes_lower_per_net_or_for_the_board() {
        let board = board(&[("VBUS", &["Power"]), ("SDA", &["Default"])]);
        let rules = lowered(
            "(rule pw (condition \"A.NetClass == 'Power'\") (constraint track_width (min 0.5mm) (opt 0.8mm)))
             (rule all (condition \"A.Type == 'track'\") (constraint track_width (min 0.15mm)))
             (rule via (condition \"A.Type == 'via'\") (constraint via_diameter (min 0.6)) (constraint hole_size (min 0.3) (max 6.3)))
             (rule ring (condition \"A.isPlated()\") (constraint annular_width (min 0.15)))
             (rule pads (condition \"A.Type == 'pad' && !A.isPlated()\") (constraint hole_size (min 0.5)))
             (rule edge (condition \"A.Type == 'track'\") (constraint edge_clearance (min 0.3mm)))
             (rule npth (condition \"!A.isPlated() && B.Type == 'track' && A.Net != B.Net\") (constraint hole_clearance (min 0.254)))
             (rule h2h (condition \"A.Net != B.Net\") (constraint hole_to_hole (min 0.5)))",
            &board,
        );
        assert_eq!(rules.width_min, [0.5, 0.15]);
        assert_eq!(rules.width_min_all, 0.15);
        assert_eq!(rules.width_opt, [Some(0.8), None]);
        assert_eq!(rules.via_diameter_min_all, 0.6);
        assert_eq!(rules.via_drill_min_all, 0.3);
        assert_eq!(rules.via_drill_max, [6.3, 6.3]);
        assert_eq!(rules.annulus_min_all, 0.15);
        assert_eq!(rules.edge_clearance, 0.3);
        assert_eq!(rules.hole_clearance, 0.254);
        // hole_to_hole is a warning by KiCad's default.
        assert_eq!(rules.hole_to_hole, 0.0);
        let status: BTreeMap<&str, &str> = rules
            .report
            .iter()
            .map(|r| (r.rule.as_str(), r.status.as_str()))
            .collect();
        assert_eq!(status["pads"], "no_effect");
        assert_eq!(status["h2h"], "skipped");
        let severities = BTreeMap::from([("hole_to_hole".to_string(), "error".to_string())]);
        let strict = lower(
            &parse_rules(
                "(rule h2h (condition \"A.Net != B.Net\") (constraint hole_to_hole (min 0.5)))",
            )
            .unwrap(),
            &board,
            &severities,
        );
        assert_eq!(strict.hole_to_hole, 0.5);
    }

    #[test]
    fn a_rule_the_classes_already_keep_has_no_effect() {
        let board = board(&[("A", &["Default"]), ("B", &["Default"])]);
        let rules = lowered(
            "(rule loose (condition \"A.Type == 'track' && B.Type == A.Type\") (constraint clearance (min 0.09mm)))
             (rule narrow (constraint track_width (min 0.09mm)))",
            &board,
        );
        assert_eq!(rules.report[0].status, "no_effect");
        // A board-wide minimum width still floors the neck-downs.
        assert_eq!(rules.report[1].status, "applied");
        assert_eq!(rules.width_min_all, 0.09);
    }

    #[test]
    fn not_routing_rules_and_through_via_disallows_need_nothing() {
        let board = board(&[("A", &["Default"])]);
        let rules = lowered(
            "(rule silk (constraint silk_clearance (min 0.15)))
             (rule text (constraint text_height (min 1mm)))
             (rule vias (constraint disallow buried_via micro_via))
             (rule zone (constraint thermal_spoke_width (min 0.3)))",
            &board,
        );
        let statuses: Vec<&str> = rules.report.iter().map(|r| r.status.as_str()).collect();
        assert_eq!(
            statuses,
            ["not_routing", "not_routing", "no_effect", "not_routing"]
        );
        assert!(rules.unmodelled.is_empty());
    }

    #[test]
    fn conditions_are_evaluated_both_ways_and_with_kleene_logic() {
        let ex = parse_condition("A.Type == 'pad' && B.Type == 'track'").unwrap();
        let net = NetFacts {
            name: "N".into(),
            classes: vec!["Default".into()],
            diff_pair: None,
        };
        let track = Item {
            kind: Kind::Track,
            net: Some(&net),
            layer: "F.Cu",
        };
        let pad = Item {
            kind: Kind::SmdPad,
            net: Some(&net),
            layer: "F.Cu",
        };
        assert_eq!(pair_truth(&ex, track, Some(pad), false), T3::True);
        assert_eq!(pair_truth(&ex, track, Some(track), false), T3::False);
        let unknown = parse_condition("A.Type == 'via' || A.insideArea('X')").unwrap();
        assert_eq!(pair_truth(&unknown, track, None, false), T3::Unknown);
        let decided = parse_condition("A.Type == 'pad' && A.insideArea('X')").unwrap();
        assert_eq!(pair_truth(&decided, track, None, false), T3::False);
        assert_eq!(
            parse_condition("A.Pad_Type == 'NPTH, mechanical' && B.Type == 'track'").unwrap(),
            Ex::And(
                Box::new(Ex::Compare(
                    Compare::Equal,
                    Box::new(Ex::Property(Side::A, "padtype".into())),
                    Box::new(Ex::Text("NPTH, mechanical".into()))
                )),
                Box::new(Ex::Compare(
                    Compare::Equal,
                    Box::new(Ex::Property(Side::B, "type".into())),
                    Box::new(Ex::Text("track".into()))
                ))
            )
        );
    }
}
