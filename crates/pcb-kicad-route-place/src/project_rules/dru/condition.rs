// Copyright (C) 2026 Toit contributors.

//! The condition language of the custom rules (KiCad's libeval as pcbnew
//! uses it): the parser, and the evaluation for what the router can tell
//! apart (a net, its classes, an item's kind and layer).

use std::collections::BTreeSet;

use super::length_mm;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    A,
    B,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Compare {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Ex {
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

pub(super) fn parse_condition(text: &str) -> Result<Ex, String> {
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
pub(super) enum T3 {
    True,
    False,
    Unknown,
}

impl T3 {
    pub(super) fn of(value: bool) -> Self {
        if value { Self::True } else { Self::False }
    }

    pub(super) fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::False, _) | (_, Self::False) => Self::False,
            (Self::True, Self::True) => Self::True,
            _ => Self::Unknown,
        }
    }

    pub(super) fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::True, _) | (_, Self::True) => Self::True,
            (Self::False, Self::False) => Self::False,
            _ => Self::Unknown,
        }
    }

    pub(super) fn not(self) -> Self {
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
    pub(super) fn effective(&self) -> String {
        self.classes.join(",")
    }
}

#[derive(Clone, Copy)]
pub(super) struct Item<'a> {
    pub kind: Kind,
    pub net: Option<&'a NetFacts>,
    pub layer: &'a str,
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
pub(super) fn text_matches(pattern: &str, text: &str) -> bool {
    let pattern = pattern.to_lowercase();
    let text = text.to_lowercase();
    if pattern.contains(['*', '?']) {
        crate::project_rules::wildcard_match(&pattern, &text)
    } else {
        pattern == text
    }
}

pub(super) struct Context<'a> {
    pub a: Item<'a>,
    pub b: Option<Item<'a>>,
    /// Whether A and B are of one net (`A.Net == B.Net`).
    pub same_net: bool,
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
                    crate::project_rules::wildcard_match(&pattern, base)
                        || base.strip_suffix('_').is_some_and(|stem| {
                            crate::project_rules::wildcard_match(&pattern, stem)
                        }),
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

    pub(super) fn truth(&self, ex: &Ex) -> T3 {
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
pub(super) fn pair_truth(ex: &Ex, a: Item, b: Option<Item>, same_net: bool) -> T3 {
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
pub(super) fn opaque_atoms(ex: &Ex, out: &mut BTreeSet<String>) {
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

pub(super) fn side_name(side: Side) -> &'static str {
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
pub(super) fn net_leaves<'e>(ex: &'e Ex, out: &mut Vec<&'e Ex>, compares_classes: &mut bool) {
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
pub(super) fn leaf_on(leaf: &Ex, net: &NetFacts) -> T3 {
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
