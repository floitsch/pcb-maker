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
    finite_number(
        node.children()
            .get(index)
            .and_then(Expr::atom)
            .ok_or_else(|| format!("{label} is missing"))?,
        label,
    )
}

/// A number of a file (a coordinate, a size, an angle), refused as the
/// input's error, naming `label`, when it is not finite: Rust reads `NaN`,
/// `inf` and `1e999` as numbers, and until 2026-10-21 they reached the
/// lattice's sizing (`usize::MAX + 1`) and a rule area's sort (a panic).
/// Every coordinate the readers take from a form goes through here
/// (`form_f64`, `expression_coordinate`).
pub(crate) fn finite_number(text: &str, label: &str) -> Result<f64, String> {
    let value = text
        .parse::<f64>()
        .map_err(|error| format!("invalid {label}: {error}"))?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(crate::verdict::input_error(format!(
            "{label} is {text}: coordinates and sizes must be finite numbers"
        )))
    }
}

pub(crate) fn form_atom<'a>(node: &'a Expr, head: &str, index: usize) -> Option<&'a str> {
    node.child(head)?.children().get(index)?.atom()
}

pub(crate) fn form_f64(node: &Expr, head: &str, index: usize) -> Result<f64, String> {
    let text = form_atom(node, head, index)
        .ok_or_else(|| format!("{head} form is missing coordinate {index}"))?;
    match text.parse::<f64>() {
        Ok(value) if value.is_finite() => Ok(value),
        Ok(_) => Err(crate::verdict::input_error(format!(
            "{} {head} coordinate {index} is {text}: coordinates and sizes must be finite numbers",
            item_name(node)
        ))),
        Err(error) => Err(format!("invalid {head} coordinate: {error}")),
    }
}

/// An item for a message: its head and, when it has them, its reference
/// or position (`a gr_line`, `the footprint R1`).
fn item_name(node: &Expr) -> String {
    let head = node.head().unwrap_or("form");
    match footprint_reference(node) {
        Some(reference) if !reference.is_empty() => format!("the {head} {reference}'s"),
        _ => format!("a {head}'s"),
    }
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
            Some(angle) => finite_number(angle, "an at form's rotation")?,
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

/// How deep lists may nest in a file read: KiCad's own files nest a dozen
/// levels (a footprint's pad's primitive's points), and every reader that
/// walks a tree recursively (the writer's indentation aside, which walks
/// with its own stack) stays within a few hundred kilobytes of stack at
/// this depth, on any thread. A file nested deeper is refused as the
/// input's error: until 2026-10-21 a few hundred thousand "(" overflowed
/// the stack (SIGSEGV, no verdict).
pub(crate) const MAX_NESTING: usize = 1000;

/// Parses a KiCad S-expression file. Its errors are the input's
/// (`verdict::input_error`): a file that does not parse or nests deeper
/// than `MAX_NESTING`. Numbers are read where they are used, and refused
/// there when they are not finite (`finite_number`): a word such as `inf` is a
/// KiCad 5 name or a tstamp as often as a number.
pub(crate) fn parse(source: &str) -> Result<Expr, String> {
    let mut parser = Parser { source, offset: 0 };
    let expression = parser.expression().map_err(crate::verdict::input_error)?;
    parser.whitespace();
    if parser.offset != source.len() {
        return Err(crate::verdict::input_error(format!(
            "trailing input at byte {}",
            parser.offset
        )));
    }
    Ok(expression)
}

/// An open list while parsing: its items so far and whether its "(" was
/// missing (a teardrop setting's, `Parser::list`).
struct Open {
    items: Vec<Expr>,
    without_opening: bool,
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

    /// The line of the parser's place, for messages.
    fn line(&self) -> usize {
        self.source[..self.offset].matches('\n').count() + 1
    }

    /// A list and the lists in it, with a stack of the open ones rather
    /// than recursion (`MAX_NESTING`).
    fn list(&mut self) -> Result<Expr, String> {
        self.bump();
        let mut open = vec![Open {
            items: Vec::new(),
            without_opening: false,
        }];
        loop {
            self.whitespace();
            let Some(character) = self.peek() else {
                return Err("unterminated list".into());
            };
            if character == ')' {
                self.bump();
                let list = Expr::List(open.pop().expect("a list is open").items);
                match open.last_mut() {
                    Some(parent) => parent.items.push(list),
                    None => return Ok(list),
                }
                continue;
            }
            let top = open.last().expect("a list is open");
            // KiCad reads a teardrop setting's opening parenthesis as
            // optional (PCB_IO_KICAD_SEXPR_PARSER's
            // parseTEARDROP_PARAMETERS: a key after a missing "(" ends
            // at its ")" as usual), and a pad written as
            // "(curved_edges no)filter_ratio 0.9)" loads; read as a
            // list, the stray ")" closed the pad and then the board
            // (RoyalBlue54L-Feather: "trailing input at byte 14111").
            let missing_opening = character != '('
                && !top.without_opening
                && matches!(top.items.first(), Some(Expr::Atom(head)) if head == "teardrops");
            if character == '(' || missing_opening {
                if open.len() >= MAX_NESTING {
                    return Err(format!(
                        "lists nested deeper than {MAX_NESTING} levels at line {} (byte {}): no KiCad file nests so deep",
                        self.line(),
                        self.offset
                    ));
                }
                if character == '(' {
                    self.bump();
                }
                open.push(Open {
                    items: Vec::new(),
                    without_opening: missing_opening,
                });
                continue;
            }
            let atom = self.atom()?;
            open.last_mut().expect("a list is open").items.push(atom);
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

/// A string's text as KiCad reads it: a backslash takes the character
/// after it as it is (`\"` a quote, `\\` a backslash), as the custom
/// rules' reader (`project_rules::dru`) and KiCad's DSN lexer read them.
/// `Expr::atom` keeps the escapes (the writer writes atoms back as read);
/// compare names from other files (the project's JSON, the custom rules)
/// with a board's through this.
pub(crate) fn unescape(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains('\\') {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            if let Some(next) = characters.next() {
                out.push(next);
            }
        } else {
            out.push(character);
        }
    }
    std::borrow::Cow::Owned(out)
}

/// Whether `form` is a footprint (or KiCad 5 module) with its name: a
/// list of at least the head and a name atom. A file's `(footprint)`
/// (a damaged library file, a hostile board) is refused where it is
/// read, before code that writes the name (`items[1]`) or reads the
/// items after it.
pub(crate) fn footprint_form_error(form: &Expr) -> Option<String> {
    match form.children() {
        [_, Expr::Atom(_), ..] => None,
        [] | [_] => Some("a footprint form without its name".into()),
        _ => Some("a footprint form whose name is not a string".into()),
    }
}

fn unquote(atom: &str) -> &str {
    atom.strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(atom)
}

/// The text of `expression` as written to a file: each footprint's
/// courtyard graphics in KiCad's save order, which KiCad chains them in
/// ([`canonical_order::source_order`]); the same whatever order the board
/// was read in.
pub(crate) fn encode(expression: &Expr) -> String {
    let mut output = String::new();
    encode_into(expression, &mut output, true);
    output
}

/// The text of `expression` as it is held, footprint graphics in the
/// canonical order (for keys and digests).
pub(crate) fn encode_canonical(expression: &Expr) -> String {
    let mut output = String::new();
    encode_into(expression, &mut output, false);
    output
}

/// Writes `expression`, with a stack of the open lists rather than
/// recursion (a tree built in code is not bound by `MAX_NESTING`).
fn encode_into<'a>(expression: &'a Expr, output: &mut String, source_order: bool) {
    struct Frame<'a> {
        items: &'a [Expr],
        order: Option<Vec<usize>>,
        next: usize,
        depth: usize,
        data: bool,
    }
    let open = |list: &'a Expr, depth: usize, output: &mut String| -> Frame<'a> {
        let order = if source_order && list.head() == Some("footprint") {
            canonical_order::source_order(list)
        } else {
            None
        };
        output.push('(');
        Frame {
            items: list.children(),
            order,
            next: 0,
            depth,
            // Embedded data (an image's base64) one string a line, as
            // KiCad writes it: KiCad reads lines of at most 1 MB, and an
            // image written on one 24 MB line made the board unreadable
            // (stonedDiscord's EMP).
            data: list.head() == Some("data"),
        }
    };
    let Expr::List(_) = expression else {
        if let Expr::Atom(atom) = expression {
            output.push_str(atom);
        }
        return;
    };
    let mut stack = vec![open(expression, 0, output)];
    while let Some(frame) = stack.last_mut() {
        let index = frame.next;
        if index == frame.items.len() {
            output.push(')');
            stack.pop();
            continue;
        }
        frame.next += 1;
        let item = &frame.items[frame.order.as_ref().map_or(index, |order| order[index])];
        if index > 0 {
            if matches!(item, Expr::List(_)) || (frame.data && index > 1) {
                output.push('\n');
                for _ in 0..=frame.depth {
                    output.push_str("  ");
                }
            } else {
                output.push(' ');
            }
        }
        match item {
            Expr::Atom(atom) => output.push_str(atom.as_str()),
            Expr::List(_) => {
                let depth = frame.depth + 1;
                stack.push(open(item, depth, output));
            }
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

    /// `(a (a (a ...)))`, `depth` lists deep, inside a board's footprint.
    fn nested_board(depth: usize) -> String {
        format!(
            "(kicad_pcb (footprint \"L:F\" (layer \"B.Cu\") (at 1 2) (net 1 \"GND\") {}{}))",
            "(a ".repeat(depth - 2),
            ")".repeat(depth - 2)
        )
    }

    #[test]
    fn nesting_up_to_the_limit_parses_and_every_walk_keeps_to_the_stack() {
        // On a test thread's 2 MiB stack: the parser, the writer, the
        // counts, the flip's mirror and the drop at the deepest nesting
        // read, in a debug build too.
        let text = nested_board(MAX_NESTING);
        let mut board = parse(&text).unwrap();
        assert_eq!(parse(&encode(&board)).unwrap(), board);
        assert_eq!(
            crate::board_router::expression_count(&board),
            16 + 2 * (MAX_NESTING as u64 - 2)
        );
        let mut nets = BTreeSet::new();
        collect_named_nets(&board, &mut nets);
        assert_eq!(nets.into_iter().collect::<Vec<_>>(), ["GND"]);
        if let Expr::List(items) = &mut board {
            crate::flip::flip_footprint(&mut items[1]).unwrap();
        }
        drop(board);
        // One level more is the input's error, with its line.
        let error = parse(&nested_board(MAX_NESTING + 1)).unwrap_err();
        assert!(
            error.contains("nested deeper than 1000 levels at line 1"),
            "{error}"
        );
        assert!(crate::verdict::is_input_error(&error));
        // Far deeper: no recursion to overflow.
        assert!(parse(&"(".repeat(200_000)).is_err());
    }

    #[test]
    fn numbers_that_are_not_finite_are_refused_where_they_are_read() {
        // Rust reads NaN, inf and 1e999 as numbers; a coordinate or size
        // that is one is the input's error, naming the item.
        for (text, message) in [
            (
                "(gr_line (start NaN 0))",
                "a gr_line's start coordinate 1 is NaN",
            ),
            (
                "(gr_line (start 0 -inf))",
                "a gr_line's start coordinate 2 is -inf",
            ),
            (
                "(via (at +Infinity 0))",
                "a via's at coordinate 1 is +Infinity",
            ),
            ("(via (at 1e999 0))", "a via's at coordinate 1 is 1e999"),
            (r#"(via (at "NaN" 0))"#, "a via's at coordinate 1 is NaN"),
        ] {
            let item = parse(text).unwrap();
            let error = form_xy(&item, item.children()[1].head().unwrap()).unwrap_err();
            assert!(error.starts_with(message), "{error}");
            assert!(crate::verdict::is_input_error(&error));
        }
        let footprint =
            parse(r#"(footprint "L:R" (at 1 2 nan) (property "Reference" "R7"))"#).unwrap();
        assert!(form_at(&footprint).unwrap_err().contains("rotation is nan"));
        let footprint =
            parse(r#"(footprint "L:R" (at inf 2) (property "Reference" "R7"))"#).unwrap();
        assert!(
            form_at(&footprint)
                .unwrap_err()
                .starts_with("the footprint R7's at coordinate 1 is inf")
        );
        let point = parse("(xy 1e400 0)").unwrap();
        assert!(
            expression_coordinate(&point, 1, "polygon x")
                .unwrap_err()
                .contains("1e400")
        );
        // A word that reads as a number is a name where a name is read: a
        // KiCad 5 board's unquoted net "inf", a tstamp "inf".
        let board = parse("(kicad_pcb (net 3 inf) (tstamp inf) (x 1e5 -2.5E-3 .5 1e308))").unwrap();
        assert_eq!(form_atom(&board, "net", 2), Some("inf"));
        assert_eq!(form_f64(&board, "x", 4).unwrap(), 1e308);
    }

    #[test]
    fn escapes_are_undone_as_kicad_reads_them() {
        assert_eq!(unescape("GND"), "GND");
        assert!(matches!(unescape("GND"), std::borrow::Cow::Borrowed(_)));
        assert_eq!(unescape(r#"A\"B"#), r#"A"B"#);
        assert_eq!(unescape(r"A\\B"), r"A\B");
        assert_eq!(unescape(r"A\"), "A");
        // As the writer quotes them.
        let name = r#"x"y\z"#;
        assert_eq!(unescape(unquote(&quote(name))), name);
    }

    #[test]
    fn a_footprint_form_needs_its_name() {
        for (text, refused) in [
            ("(footprint)", true),
            ("(module)", true),
            ("(footprint (at 0 0))", true),
            (r#"(footprint "L:R")"#, false),
            ("(footprint R (layer F.Cu))", false),
        ] {
            assert_eq!(
                footprint_form_error(&parse(text).unwrap()).is_some(),
                refused,
                "{text}"
            );
        }
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
