// Copyright (C) 2026 Toit contributors.

//! KiCad's S-expression files: the expression tree, the parser and the
//! writer (in source order or canonical), and the forms read from them.

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Expr {
    Atom(String),
    List(Vec<Expr>),
}

impl Expr {
    pub(crate) fn head(&self) -> Option<&str> {
        match self {
            Self::List(items) => items.first().and_then(Self::atom),
            Self::Atom(_) => None,
        }
    }

    pub(crate) fn atom(&self) -> Option<&str> {
        match self {
            Self::Atom(atom) => Some(unquote(atom)),
            Self::List(_) => None,
        }
    }

    pub(crate) fn child(&self, head: &str) -> Option<&Expr> {
        match self {
            Self::List(items) => items.iter().find(|item| item.head() == Some(head)),
            Self::Atom(_) => None,
        }
    }

    pub(crate) fn children(&self) -> &[Expr] {
        match self {
            Self::List(items) => items,
            Self::Atom(_) => &[],
        }
    }
}

pub(crate) fn set_form_at(node: &mut Expr, at: [f64; 3]) -> Result<(), String> {
    let Expr::List(items) = node else {
        return Err("cannot set at form on an atom".into());
    };
    let Some(Expr::List(values)) = items.iter_mut().find(|item| item.head() == Some("at")) else {
        return Err("node has no at form".into());
    };
    values.truncate(4);
    while values.len() < 4 {
        values.push(Expr::Atom("0".into()));
    }
    values[1] = Expr::Atom(at[0].to_string());
    values[2] = Expr::Atom(at[1].to_string());
    values[3] = Expr::Atom(at[2].to_string());
    Ok(())
}

pub(crate) fn expression_coordinate(node: &Expr, index: usize, label: &str) -> Result<f64, String> {
    node.children()
        .get(index)
        .and_then(Expr::atom)
        .ok_or_else(|| format!("{label} is missing"))?
        .parse::<f64>()
        .map_err(|error| format!("invalid {label}: {error}"))
}

pub(crate) fn form_atom<'a>(node: &'a Expr, head: &str, index: usize) -> Option<&'a str> {
    node.child(head)?.children().get(index)?.atom()
}

pub(crate) fn form_f64(node: &Expr, head: &str, index: usize) -> Result<f64, String> {
    form_atom(node, head, index)
        .ok_or_else(|| format!("{head} form is missing coordinate {index}"))?
        .parse::<f64>()
        .map_err(|error| format!("invalid {head} coordinate: {error}"))
}

pub(crate) fn form_xy(node: &Expr, head: &str) -> Result<[f64; 2], String> {
    Ok([form_f64(node, head, 1)?, form_f64(node, head, 2)?])
}

pub(crate) fn form_at(node: &Expr) -> Result<[f64; 3], String> {
    let at = node
        .child("at")
        .ok_or_else(|| "copper item has no at form".to_string())?;
    Ok([
        form_f64(node, "at", 1)?,
        form_f64(node, "at", 2)?,
        // KiCad 6 writes `(at x y unlocked)` and `(at x y angle unlocked)`
        // for texts: the flag is not a rotation.
        match at.children().get(3).and_then(Expr::atom) {
            None | Some("unlocked" | "locked") => 0.0,
            Some(angle) => angle
                .parse::<f64>()
                .map_err(|error| format!("invalid at rotation: {error}"))?,
        },
    ])
}

pub(crate) fn coordinate_form(head: &str, coordinate: [f64; 2]) -> Expr {
    Expr::List(vec![
        Expr::Atom(head.into()),
        Expr::Atom(coordinate[0].to_string()),
        Expr::Atom(coordinate[1].to_string()),
    ])
}

pub(crate) fn string_form(head: &str, value: &str) -> Expr {
    Expr::List(vec![Expr::Atom(head.into()), Expr::Atom(quote(value))])
}

pub(crate) fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(crate) fn deterministic_uuid_for(identity: &str) -> String {
    let digest = Sha256::digest(identity);
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

pub(crate) fn count_head(root: &Expr, head: &str) -> usize {
    root.children()
        .iter()
        .filter(|item| item.head() == Some(head))
        .count()
}

pub(crate) fn root_head(items: &[Expr]) -> Option<&str> {
    items.first()?.atom()
}

pub(crate) fn parse(source: &str) -> Result<Expr, String> {
    let mut parser = Parser { source, offset: 0 };
    let expression = parser.expression()?;
    parser.whitespace();
    if parser.offset != source.len() {
        return Err(format!("trailing input at byte {}", parser.offset));
    }
    Ok(expression)
}

struct Parser<'a> {
    source: &'a str,
    offset: usize,
}

impl Parser<'_> {
    fn expression(&mut self) -> Result<Expr, String> {
        self.whitespace();
        match self.peek() {
            Some('(') => self.list(),
            Some(_) => self.atom(),
            None => Err("unexpected end of input".into()),
        }
    }

    fn list(&mut self) -> Result<Expr, String> {
        self.bump();
        let mut items = Vec::new();
        loop {
            self.whitespace();
            match self.peek() {
                Some(')') => {
                    self.bump();
                    return Ok(Expr::List(items));
                }
                // KiCad reads a teardrop setting's opening parenthesis as
                // optional (PCB_IO_KICAD_SEXPR_PARSER's
                // parseTEARDROP_PARAMETERS: a key after a missing "(" ends
                // at its ")" as usual), and a pad written as
                // "(curved_edges no)filter_ratio 0.9)" loads; read as a
                // list, the stray ")" closed the pad and then the board
                // (RoyalBlue54L-Feather: "trailing input at byte 14111").
                Some(character)
                    if character != '('
                        && matches!(items.first(), Some(Expr::Atom(head)) if head == "teardrops") =>
                {
                    items.push(self.list_without_opening()?)
                }
                Some(_) => items.push(self.expression()?),
                None => return Err("unterminated list".into()),
            }
        }
    }

    /// A list whose "(" is missing: its items up to and with its ")".
    fn list_without_opening(&mut self) -> Result<Expr, String> {
        let mut items = Vec::new();
        loop {
            self.whitespace();
            match self.peek() {
                Some(')') => {
                    self.bump();
                    return Ok(Expr::List(items));
                }
                Some(_) => items.push(self.expression()?),
                None => return Err("unterminated list".into()),
            }
        }
    }

    fn atom(&mut self) -> Result<Expr, String> {
        let start = self.offset;
        if self.peek() == Some('"') {
            self.bump();
            let mut escaped = false;
            while let Some(character) = self.bump() {
                if character == '"' && !escaped {
                    return Ok(Expr::Atom(self.source[start..self.offset].to_string()));
                }
                escaped = character == '\\' && !escaped;
                if character != '\\' {
                    escaped = false;
                }
            }
            return Err("unterminated string".into());
        }
        while let Some(character) = self.peek() {
            if character.is_whitespace() || matches!(character, '(' | ')') {
                break;
            }
            self.bump();
        }
        if self.offset == start {
            Err(format!("expected atom at byte {}", self.offset))
        } else {
            Ok(Expr::Atom(self.source[start..self.offset].to_string()))
        }
    }

    fn whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    fn peek(&self) -> Option<char> {
        self.source[self.offset..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.offset += character.len_utf8();
        Some(character)
    }
}

fn unquote(atom: &str) -> &str {
    atom.strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(atom)
}

/// The text of `expression` as written to a file: a footprint's graphics
/// in the order of the file they were read from when only the canonical
/// order moved them ([`canonical_order::source_order`]).
pub(crate) fn encode(expression: &Expr) -> String {
    let mut output = String::new();
    encode_into(expression, 0, &mut output, true);
    output
}

/// The text of `expression` as it is held, footprint graphics in the
/// canonical order (for keys and digests).
pub(crate) fn encode_canonical(expression: &Expr) -> String {
    let mut output = String::new();
    encode_into(expression, 0, &mut output, false);
    output
}

fn encode_into(expression: &Expr, depth: usize, output: &mut String, source_order: bool) {
    match expression {
        Expr::Atom(atom) => output.push_str(atom),
        Expr::List(items) => {
            let order = if source_order && expression.head() == Some("footprint") {
                canonical_order::source_order(expression)
            } else {
                None
            };
            // Embedded data (an image's base64) one string a line, as
            // KiCad writes it: KiCad reads lines of at most 1 MB, and an
            // image written on one 24 MB line made the board unreadable
            // (stonedDiscord's EMP).
            let data = expression.head() == Some("data");
            output.push('(');
            for index in 0..items.len() {
                let item = &items[order.as_ref().map_or(index, |order| order[index])];
                if index > 0 {
                    if matches!(item, Expr::List(_)) || (data && index > 1) {
                        output.push('\n');
                        output.push_str(&"  ".repeat(depth + 1));
                    } else {
                        output.push(' ');
                    }
                }
                encode_into(item, depth + 1, output, source_order);
            }
            output.push(')');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_teardrop_setting_without_its_opening_parenthesis_reads_as_kicad_reads_it() {
        // RoyalBlue54L-Feather's pads: "(curved_edges no)filter_ratio 0.9)".
        // KiCad takes the "(" of a teardrop setting as optional; read as
        // a list, the stray ")" closed the pad and then the board.
        let board = parse(
            r#"(kicad_pcb (footprint "C" (pad "1" smd roundrect (at -0.48 0)
              (teardrops (best_length_ratio 0.5) (max_length 1) (best_width_ratio 1)
                (max_width 2) (curved_edges no)filter_ratio 0.9)
                (enabled yes) (allow_two_segments yes) (prefer_zone_connections yes))
              (uuid "a")) (uuid "b")) (gr_line (start 0 0) (end 1 0)))"#,
        )
        .unwrap();
        let footprint = board.child("footprint").unwrap();
        let teardrops = footprint.child("pad").unwrap().child("teardrops").unwrap();
        assert_eq!(form_atom(teardrops, "filter_ratio", 1), Some("0.9"));
        assert_eq!(form_atom(teardrops, "enabled", 1), Some("yes"));
        assert!(footprint.child("uuid").is_some());
        assert!(board.child("gr_line").is_some());
        // Written back with its parenthesis.
        assert!(encode(teardrops).contains("(filter_ratio 0.9)"));
        // Elsewhere a stray atom is still an atom.
        assert!(parse("(kicad_pcb (x 1) y)").is_ok());
    }

    #[test]
    fn embedded_data_is_written_one_string_a_line() {
        // An image's base64 (stonedDiscord's EMP: 24 MB on one line, which
        // KiCad cannot read back), as KiCad writes it.
        let image = parse(
            r#"(image (at 84.7 106.25) (layer "F.Paste")
              (data "/9j/4AAQ" "CwsNDhIQ"
                "FBQUFBQU") (uuid "u"))"#,
        )
        .unwrap();
        let text = encode(&image);
        assert!(
            text.contains("(data \"/9j/4AAQ\"\n    \"CwsNDhIQ\"\n    \"FBQUFBQU\")"),
            "{text}"
        );
        assert_eq!(parse(&text).unwrap(), image);
    }

    #[test]
    fn parser_round_trips_strings_and_nested_lists() {
        let source = "(root (name \"a b\\\"c\") (xy 1.25 -2))";
        assert_eq!(
            parse(&encode(&parse(source).unwrap())).unwrap(),
            parse(source).unwrap()
        );
    }
}
