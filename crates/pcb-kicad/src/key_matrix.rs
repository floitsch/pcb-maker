// Copyright (C) 2026 Toit contributors.

//! Key matrices recognised from a netlist alone, and the starter
//! constraints `import-kicad-netlist` writes for them
//! (`constraints.starter.json`), so that an agent does not write a
//! keyboard's hundred constraints by hand.
//!
//! A key of a matrix is a switch between two shared nets: one a matrix
//! row's, one a column's. Either side may run through the key's own
//! diode: the switch's pin is then on a net only that diode (and the
//! switch) has, and the diode's other pin is on the line. Rows and columns
//! are told apart by their names (`ROW0`, `col_3`, `r2`, `C4`), else the
//! side with fewer nets is the rows (keyboards are wider than tall).
//! Halves are the matrix's separate parts (no switch joins them: each
//! half its own controller), or, in one matrix, columns named for a hand
//! (`col_L3`, `col_R3`).

use super::*;
use crate::placement_constraints::natural;

/// A part of the netlist as the inference sees it.
#[derive(Clone, Debug, Default)]
pub(crate) struct MatrixPart {
    pub reference: String,
    /// `Library:Name`.
    pub footprint: String,
    /// The symbol's name in its library (`SW_Push`, `D_Small`).
    pub symbol: String,
    pub value: String,
    /// Pin number, net and the pin's electrical type.
    pub pins: Vec<(String, String, Option<String>)>,
}

/// A key: its switch, its column's net and its own diode; `alternates`
/// are other switch footprints on the same key (the same row and column,
/// a 2u and a 1.25u footprint drawn over each other).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MatrixKey {
    pub switch: String,
    pub column: String,
    pub diode: Option<String>,
    pub alternates: Vec<String>,
}

/// A matrix row: its net and its keys in column order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MatrixLine {
    pub net: String,
    pub keys: Vec<MatrixKey>,
}

/// One matrix, or one half of a split keyboard's.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MatrixGroup {
    /// `left` or `right` for the halves of a split keyboard.
    pub half: Option<&'static str>,
    /// Rows in row order (by the number in their names, else by their
    /// first switch's reference).
    pub rows: Vec<MatrixLine>,
    /// Column nets in column order.
    pub columns: Vec<String>,
    /// The part with the most pins on this group's row and column nets.
    pub controller: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct KeyMatrix {
    pub groups: Vec<MatrixGroup>,
    /// How rows were told from columns: `names` or `fewer nets`.
    pub rows_by: &'static str,
    /// How the halves were found: `separate matrices` or `column names`.
    pub halves_by: Option<&'static str>,
    /// Switch-like parts outside the matrix (reset and power switches, an
    /// encoder, keys wired directly to a pin).
    pub unassigned: Vec<String>,
}

#[cfg(test)]
impl KeyMatrix {
    pub fn keys(&self) -> impl Iterator<Item = &MatrixKey> {
        self.groups.iter().flat_map(|group| group.rows.iter().flat_map(|row| row.keys.iter()))
    }
}

/// The letters a reference starts with (`SW` of `SW12`, `SW_POWER` of
/// `SW_POWER0`).
fn prefix(reference: &str) -> String {
    reference.chars().take_while(|c| !c.is_ascii_digit()).collect::<String>().trim_end_matches('_').to_ascii_uppercase()
}

/// Reference prefixes that are never a key switch, whatever their
/// footprint is called (a USB receptacle `USB-C-SMD_G-SWITCH`, a key's LED
/// `LED_MX_6028R`).
const NOT_SWITCHES: &[&str] = &[
    "D", "LED", "U", "IC", "J", "P", "CN", "Q", "R", "RN", "C", "L", "F", "FB", "Y", "X", "BT", "AE", "ANT", "TP", "H", "MH", "JP", "FID", "TH",
];

fn switch_named(part: &MatrixPart) -> bool {
    if NOT_SWITCHES.contains(&prefix(&part.reference).as_str()) {
        return false;
    }
    [part.footprint.as_str(), part.symbol.as_str()].iter().any(|name| {
        let name = name.to_ascii_uppercase();
        ["SW", "KEY", "MX", "CHOC", "HOTSWAP"].iter().any(|word| name.contains(word))
    })
}

fn is_led(part: &MatrixPart) -> bool {
    [part.footprint.as_str(), part.symbol.as_str(), part.value.as_str()].iter().any(|name| name.to_ascii_uppercase().contains("LED"))
}

fn last_segment(net: &str) -> &str {
    net.rsplit('/').next().unwrap_or(net)
}

/// Supply and ground nets by name; nets with a power pin count too.
fn power_name(net: &str) -> bool {
    let name = last_segment(net).to_ascii_uppercase();
    name.starts_with('+')
        || (name.starts_with('-') && name.len() > 1)
        || name.contains("GND")
        || ["VCC", "VDD", "VBUS", "VBAT", "VSYS", "VIN", "EARTH"].iter().any(|word| name.starts_with(word))
        || ["3V3", "5V", "3.3V", "1V8"].contains(&name.as_str())
}

/// The number a line's name carries (`COL3`, `R_ROW2`, `col0_r`: its last
/// run of digits); KiCad's own `Net-(...)` names carry none.
fn number_of(net: &str) -> Option<u64> {
    let name = last_segment(net);
    if name.starts_with("Net-(") {
        return None;
    }
    let end = name.rfind(|c: char| c.is_ascii_digit())? + 1;
    let start = name[..end].rfind(|c: char| !c.is_ascii_digit()).map_or(0, |start| start + 1);
    name[start..end].parse().ok()
}

/// +1 for a row's name, -1 for a column's, 0 for neither.
fn line_kind(net: &str) -> i32 {
    let name = last_segment(net).to_ascii_lowercase();
    if name.starts_with("net-(") {
        return 0;
    }
    if name.contains("row") {
        return 1;
    }
    if name.contains("col") {
        return -1;
    }
    let bare = name.trim_start_matches(|c: char| c == '_' || c == '-');
    let mut chars = bare.chars();
    match (chars.next(), chars.next()) {
        (Some('r'), Some(c)) if c.is_ascii_digit() => 1,
        (Some('c'), Some(c)) if c.is_ascii_digit() => -1,
        _ => 0,
    }
}

/// The hand a column's name gives (`col_L3`, `colR2`, `col_left_1`):
/// what follows the word column starts with an upper-case L or R, or with
/// left or right.
fn hand_of_column(net: &str) -> Option<&'static str> {
    let name = last_segment(net);
    let lower = name.to_ascii_lowercase();
    let at = lower.find("column").map(|at| at + 6).or_else(|| lower.find("col").map(|at| at + 3))?;
    let rest = name[at..].trim_start_matches(|c: char| c == '_' || c == '-');
    let rest_lower = rest.to_ascii_lowercase();
    if rest.starts_with('L') || rest_lower.starts_with("left") {
        Some("left")
    } else if rest.starts_with('R') || rest_lower.starts_with("right") {
        Some("right")
    } else {
        None
    }
}

/// Whether a matrix's nets are named for the right hand (`R_COL0`,
/// `row0_r`, a sheet `/...-right/`).
fn right_named(net: &str) -> bool {
    let lower = net.to_ascii_lowercase();
    let name = last_segment(&lower);
    lower.contains("right") || name.starts_with("r_") || name.ends_with("_r") || name.starts_with("right")
}

/// A line a switch reaches: directly, or through a diode (`diode`) whose
/// pin shares a net with these switches only (`private`).
#[derive(Clone, Debug)]
struct Terminal<'a> {
    line: &'a str,
    diode: Option<&'a str>,
    private: BTreeSet<&'a str>,
}

/// Recognises the key matrix of a netlist. None when no part looks like a
/// switch; a matrix without groups when switches share no row and column
/// nets (keys wired directly to pins).
pub(crate) fn infer_key_matrix<'a>(parts: &'a [MatrixPart]) -> Option<KeyMatrix> {
    let by_reference: BTreeMap<&str, &MatrixPart> = parts.iter().map(|part| (part.reference.as_str(), part)).collect();
    // Nets: their parts (distinct references) and whether a pin on them is
    // a power pin.
    let mut members: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut powered: BTreeSet<&str> = BTreeSet::new();
    for part in parts {
        for (_, net, kind) in &part.pins {
            members.entry(net.as_str()).or_default().insert(part.reference.as_str());
            if matches!(kind.as_deref(), Some("power_in" | "power_out")) {
                powered.insert(net.as_str());
            }
        }
    }
    let power = |net: &str| powered.contains(net) || power_name(net);
    let connected = |part: &'a MatrixPart| -> BTreeSet<&'a str> {
        part.pins.iter().map(|(_, net, _)| net.as_str()).filter(|net| !net.is_empty() && !net.starts_with("unconnected-")).collect()
    };
    let diode = |reference: &str| -> bool {
        by_reference.get(reference).is_some_and(|part| prefix(reference) == "D" && connected(part).len() == 2 && !is_led(part))
    };
    let mut named: Vec<&MatrixPart> = parts.iter().filter(|part| switch_named(part)).collect();
    if named.is_empty() {
        return None;
    }
    named.sort_by_key(|part| natural(&part.reference));
    let named_set: BTreeSet<&str> = named.iter().map(|part| part.reference.as_str()).collect();

    // A switch's terminals: the lines it reaches, directly or through a
    // diode whose other pin is on no other switch's line (the key's own
    // diode, or the column diode several keys share).
    // A diode net several switches share is followed only where `shared`
    // names it: their other lines are the same (alternate footprints of
    // one key), not a column's diode (CyberKeeb's column diodes, five
    // keys on each).
    let terminals = |part: &'a MatrixPart, shared: &BTreeSet<&'a str>| -> Vec<Terminal<'a>> {
        let mut found: Vec<Terminal> = Vec::new();
        for net in connected(part) {
            if power(net) {
                continue;
            }
            let others: Vec<&str> =
                members.get(net).map(|set| set.iter().copied().filter(|other| *other != part.reference).collect()).unwrap_or_default();
            let diodes: Vec<&str> = others.iter().copied().filter(|other| diode(other)).collect();
            let only_switches = others.iter().all(|other| *other == diodes.first().copied().unwrap_or("") || named_set.contains(other));
            let terminal = if diodes.len() == 1 && only_switches && (others.len() == 1 || shared.contains(net)) {
                let through = diodes[0];
                let Some(line) = connected(by_reference[through]).into_iter().find(|other| *other != net) else {
                    continue;
                };
                if power(line) {
                    continue;
                }
                let mut private: BTreeSet<&str> = others.iter().copied().filter(|other| *other != through).collect();
                private.insert(part.reference.as_str());
                Terminal { line, diode: Some(through), private }
            } else {
                Terminal { line: net, diode: None, private: BTreeSet::new() }
            };
            if !found.iter().any(|seen| seen.line == terminal.line) {
                found.push(terminal);
            }
        }
        found
    };
    let none = BTreeSet::new();
    let first: BTreeMap<&str, Vec<Terminal>> = named.iter().map(|part| (part.reference.as_str(), terminals(part, &none))).collect();
    let mut shared: BTreeSet<&str> = BTreeSet::new();
    for (net, on) in &members {
        let switches: Vec<&str> = on.iter().copied().filter(|other| named_set.contains(other)).collect();
        if switches.len() < 2 || on.iter().filter(|other| diode(other)).count() != 1 || switches.len() + 1 != on.len() {
            continue;
        }
        let rest = |switch: &str| -> BTreeSet<&str> { first[switch].iter().map(|terminal| terminal.line).filter(|line| line != net).collect() };
        if switches.iter().all(|switch| rest(switch) == rest(switches[0])) {
            shared.insert(net);
        }
    }
    let mut all_terminals: BTreeMap<&str, Vec<Terminal>> =
        named.iter().map(|part| (part.reference.as_str(), terminals(part, &shared))).collect();
    // Lines: nets two switches or more reach.
    let mut reach: BTreeMap<&str, usize> = BTreeMap::new();
    for found in all_terminals.values() {
        for terminal in found {
            *reach.entry(terminal.line).or_default() += 1;
        }
    }
    let mut cells: Vec<(&str, [Terminal; 2])> = Vec::new();
    let mut unassigned: Vec<String> = Vec::new();
    for part in &named {
        let lines: Vec<Terminal> =
            all_terminals[part.reference.as_str()].iter().filter(|terminal| reach[terminal.line] >= 2).cloned().collect();
        match <[Terminal; 2]>::try_from(lines) {
            Ok(pair) => cells.push((part.reference.as_str(), pair)),
            Err(_) => unassigned.push(part.reference.clone()),
        }
    }
    // A key alone on its column (or row): a switch with two terminals, one
    // on a line, the other on a net the controller has too (a part with
    // pins on two lines or more).
    let mut line_nets: BTreeSet<&str> = cells.iter().flat_map(|(_, pair)| pair.iter().map(|terminal| terminal.line)).collect();
    let controllers: BTreeSet<&str> = parts
        .iter()
        .filter(|part| !named_set.contains(part.reference.as_str()) && !diode(&part.reference))
        .filter(|part| part.pins.iter().filter(|(_, net, _)| line_nets.contains(net.as_str())).count() >= 2)
        .map(|part| part.reference.as_str())
        .collect();
    let mut rescued = Vec::new();
    for reference in &unassigned {
        let found = &all_terminals[reference.as_str()];
        if let Ok(pair) = <[Terminal; 2]>::try_from(found.clone()) {
            let on_line = pair.iter().filter(|terminal| line_nets.contains(terminal.line)).count();
            let lone = pair.iter().find(|terminal| !line_nets.contains(terminal.line));
            if on_line == 1
                && let Some(lone) = lone
                && members.get(lone.line).is_some_and(|on| on.iter().any(|other| controllers.contains(other)))
            {
                rescued.push(reference.clone());
                cells.push((named.iter().find(|part| part.reference == *reference).unwrap().reference.as_str(), pair));
            }
        }
    }
    unassigned.retain(|reference| !rescued.contains(reference));
    line_nets.extend(cells.iter().flat_map(|(_, pair)| pair.iter().map(|terminal| terminal.line)));
    // Parts named otherwise with exactly two pins, one on a line of each
    // kind the named switches use (a capacitive pad, a `B1` button).
    let mut others: Vec<&MatrixPart> = parts
        .iter()
        .filter(|part| {
            !named_set.contains(part.reference.as_str())
                && !NOT_SWITCHES.contains(&prefix(&part.reference).as_str())
                && part.pins.len() == 2
                && connected(part).len() == 2
        })
        .collect();
    others.sort_by_key(|part| natural(&part.reference));
    for part in others {
        let found = terminals(part, &none);
        if let Ok(pair) = <[Terminal; 2]>::try_from(found)
            && pair.iter().all(|terminal| line_nets.contains(terminal.line))
        {
            all_terminals.insert(part.reference.as_str(), pair.to_vec());
            cells.push((part.reference.as_str(), pair));
        }
    }

    // Two-colour the lines (a switch joins a row to a column), component
    // by component; a switch that would join two lines of one colour is
    // left out.
    let mut colour: BTreeMap<&str, (usize, u8)> = BTreeMap::new();
    let mut adjacency: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (_, pair) in &cells {
        adjacency.entry(pair[0].line).or_default().push(pair[1].line);
        adjacency.entry(pair[1].line).or_default().push(pair[0].line);
    }
    let mut components = 0;
    let mut order: Vec<&str> = Vec::new();
    for (_, pair) in &cells {
        for terminal in pair {
            if !order.contains(&terminal.line) {
                order.push(terminal.line);
            }
        }
    }
    for start in order {
        if colour.contains_key(start) {
            continue;
        }
        colour.insert(start, (components, 0));
        let mut queue = VecDeque::from([start]);
        while let Some(line) = queue.pop_front() {
            let (component, own) = colour[line];
            for next in &adjacency[line] {
                if !colour.contains_key(next) {
                    colour.insert(next, (component, 1 - own));
                    queue.push_back(next);
                }
            }
        }
        components += 1;
    }
    let mut keyed: Vec<(&str, &Terminal, &Terminal, usize)> = Vec::new();
    for (switch, pair) in &cells {
        let (a, b) = (colour[pair[0].line], colour[pair[1].line]);
        if a.1 == b.1 {
            unassigned.push(switch.to_string());
            continue;
        }
        let (zero, one) = if a.1 == 0 { (&pair[0], &pair[1]) } else { (&pair[1], &pair[0]) };
        keyed.push((switch, zero, one, a.0));
    }

    let mut groups: Vec<(MatrixGroup, Vec<String>)> = Vec::new();
    let mut rows_by = "names";
    for component in 0..components {
        let cells: Vec<&(&str, &Terminal, &Terminal, usize)> = keyed.iter().filter(|cell| cell.3 == component).collect();
        let lines = |side: usize| -> BTreeSet<&str> { cells.iter().map(|cell| if side == 0 { cell.1.line } else { cell.2.line }).collect() };
        let (zero, one) = (lines(0), lines(1));
        // A matrix of four keys or more, on two lines or more each way
        // (a row of keys on one line is a matrix of one row).
        if cells.len() < 4 || zero.len() + one.len() < 3 {
            unassigned.extend(cells.iter().map(|cell| cell.0.to_string()));
            continue;
        }
        let score = |set: &BTreeSet<&str>| -> i32 { set.iter().map(|net| line_kind(net)).sum() };
        let through = |side: usize| cells.iter().filter(|cell| (if side == 0 { cell.1 } else { cell.2 }).diode.is_some()).count();
        let rows_side = if score(&zero) != score(&one) {
            if score(&zero) > score(&one) { 0 } else { 1 }
        } else {
            rows_by = "fewer nets";
            if zero.len() != one.len() {
                if zero.len() < one.len() { 0 } else { 1 }
            } else if through(0) >= through(1) {
                0
            } else {
                1
            }
        };
        // (switch, row line, column line, row terminal, column terminal)
        let oriented: Vec<(&str, &Terminal, &Terminal)> =
            cells.iter().map(|cell| if rows_side == 0 { (cell.0, cell.1, cell.2) } else { (cell.0, cell.2, cell.1) }).collect();
        groups.push((build_group(&oriented), Vec::new()));
    }
    // Halves: two separate matrices, or one whose columns are named for a
    // hand.
    let mut halves_by = None;
    if groups.len() == 2 {
        halves_by = Some("separate matrices");
        let right = |group: &MatrixGroup| group.rows.iter().all(|row| right_named(&row.net)) || group.columns.iter().all(|net| right_named(net));
        let (first, second) = (right(&groups[0].0), right(&groups[1].0));
        let swap = first && !second;
        if swap {
            groups.swap(0, 1);
        }
        groups[0].0.half = Some("left");
        groups[1].0.half = Some("right");
    } else if groups.len() == 1 {
        let group = &groups[0].0;
        let hands: Vec<Option<&str>> = group.columns.iter().map(|net| hand_of_column(net)).collect();
        if hands.iter().all(Option::is_some) && hands.contains(&Some("left")) && hands.contains(&Some("right")) {
            halves_by = Some("column names");
            let mut split = Vec::new();
            for hand in ["left", "right"] {
                let columns: Vec<String> =
                    group.columns.iter().zip(&hands).filter(|(_, own)| **own == Some(hand)).map(|(net, _)| net.clone()).collect();
                let rows: Vec<MatrixLine> = group
                    .rows
                    .iter()
                    .map(|row| MatrixLine { net: row.net.clone(), keys: row.keys.iter().filter(|key| columns.contains(&key.column)).cloned().collect() })
                    .filter(|row| !row.keys.is_empty())
                    .collect();
                split.push((MatrixGroup { half: Some(hand), rows, columns, controller: None }, Vec::new()));
            }
            groups = split;
        }
    }
    // Each group's controller: the part (not a switch or diode) with the
    // most pins on its lines; one shared by both halves is no half's.
    let switches: BTreeSet<&str> = keyed.iter().map(|cell| cell.0).collect();
    for (group, _) in groups.iter_mut() {
        let lines: BTreeSet<&str> =
            group.rows.iter().map(|row| row.net.as_str()).chain(group.columns.iter().map(String::as_str)).collect();
        let mut best: Option<(usize, &str)> = None;
        for part in parts {
            if switches.contains(part.reference.as_str()) || named_set.contains(part.reference.as_str()) || diode(&part.reference) {
                continue;
            }
            let count = part.pins.iter().filter(|(_, net, _)| lines.contains(net.as_str())).count();
            if count >= 2 && best.is_none_or(|(most, _)| count > most) {
                best = Some((count, part.reference.as_str()));
            }
        }
        group.controller = best.map(|(_, reference)| reference.to_string());
    }
    if groups.len() == 2 && groups[0].0.controller.is_some() && groups[0].0.controller == groups[1].0.controller {
        for (group, _) in groups.iter_mut() {
            group.controller = None;
        }
    }
    unassigned.sort_by_key(|reference| natural(reference));
    unassigned.dedup();
    Some(KeyMatrix { groups: groups.into_iter().map(|(group, _)| group).collect(), rows_by, halves_by, unassigned })
}

/// Lines in order: by the numbers in their names where every line has its
/// own, else by the first switch on each.
fn ordered(lines: &BTreeMap<&str, Vec<&str>>) -> Vec<String> {
    let numbers: Vec<Option<u64>> = lines.keys().map(|net| number_of(net)).collect();
    let distinct: BTreeSet<u64> = numbers.iter().flatten().copied().collect();
    let mut nets: Vec<&str> = lines.keys().copied().collect();
    // Only lines named as rows or columns: a controller's pin numbers
    // (`/P0.28`) say nothing of where the keys are.
    let named = lines.keys().all(|net| line_kind(net) != 0);
    if named && numbers.iter().all(Option::is_some) && distinct.len() == numbers.len() {
        nets.sort_by_key(|net| number_of(net));
    } else {
        nets.sort_by_key(|net| lines[net].iter().map(|switch| natural(switch)).min());
    }
    nets.into_iter().map(String::from).collect()
}

fn build_group(cells: &[(&str, &Terminal, &Terminal)]) -> MatrixGroup {
    let mut rows: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut columns: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (switch, row, column) in cells {
        rows.entry(row.line).or_default().push(switch);
        columns.entry(column.line).or_default().push(switch);
    }
    let row_order = ordered(&rows);
    let column_order = ordered(&columns);
    // Keys: one per (row, column); further switches there are alternates.
    let mut lines = Vec::new();
    for row in &row_order {
        let mut keys: Vec<MatrixKey> = Vec::new();
        let mut here: Vec<&(&str, &Terminal, &Terminal)> = cells.iter().filter(|cell| cell.1.line == row).collect();
        here.sort_by_key(|cell| natural(cell.0));
        for column in &column_order {
            let at: Vec<&&(&str, &Terminal, &Terminal)> = here.iter().filter(|cell| cell.2.line == column).collect();
            let Some(first) = at.first() else {
                continue;
            };
            // The key's own diode: on a net between it and the diode that
            // only this key's switches share.
            let own: BTreeSet<&str> = at.iter().map(|cell| cell.0).collect();
            let diode = [first.1, first.2]
                .into_iter()
                .find_map(|terminal| terminal.diode.filter(|_| terminal.private.iter().all(|switch| own.contains(switch))))
                .map(String::from);
            keys.push(MatrixKey {
                switch: first.0.to_string(),
                column: column.clone(),
                diode,
                alternates: at[1..].iter().map(|cell| cell.0.to_string()).collect(),
            });
        }
        lines.push(MatrixLine { net: row.clone(), keys });
    }
    MatrixGroup { half: None, rows: lines, columns: column_order, controller: None }
}

/// What the import reports of the matrix (`starter` in its report).
#[derive(Clone, Debug, Default, Serialize)]
pub struct KiCadStarterReport {
    /// `constraints.starter.json`, written when a matrix was found.
    pub file: Option<PathBuf>,
    /// Row and column nets of the whole matrix (both halves).
    pub matrix: [usize; 2],
    /// Switches in the matrix (alternate footprints of a key included).
    pub switches: usize,
    /// Keys with their own diode found.
    pub diodes: usize,
    /// The matrices: the two halves of a split keyboard (`at` the half of
    /// the board their region suggests), else one.
    pub halves: Vec<KiCadStarterHalf>,
    /// How the halves were told apart: `separate matrices` (no switch
    /// joins them) or `column names` (`col_L1`, `col_R1`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub halves_by: Option<String>,
    /// How rows were told from columns: their `names`, or `fewer nets`.
    pub rows_by: String,
    /// The key pitch the rows use (along a row, between rows).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pitch_mm: Option<[f64; 2]>,
    /// Switch-like parts outside the matrix: place them yourself.
    pub unassigned: Vec<String>,
    /// Keys wider or taller than the pitch (1.5u, 2u, an ISO enter), kept
    /// out of the rows and near their neighbour instead.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub wide: Vec<String>,
    /// Second footprints of a key (the same row, column and diode), near
    /// the key's first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub alternates: Vec<String>,
    /// Why no file was written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KiCadStarterHalf {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    pub rows: Vec<String>,
    pub columns: Vec<String>,
    pub switches: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controller: Option<String>,
}

/// The pitch keys of these footprints stand at, along a row and between
/// rows: Kailh Choc's 18 x 17 mm, else the Cherry MX unit, 19.05 mm.
fn standard_pitch(footprints: &[&str]) -> [f64; 2] {
    let choc = footprints
        .iter()
        .filter(|id| {
            let id = id.to_ascii_uppercase();
            id.contains("CHOC") || id.contains("PG1350") || id.contains("PG1353")
        })
        .count();
    if 2 * choc > footprints.len() { [18.0, 17.0] } else { [19.05, 19.05] }
}

/// A diode's gap to its key, as designers keep it (under or beside the
/// switch).
const DIODE_GAP_MM: f64 = 3.0;

/// The starter constraints for a key matrix: `base` (the import's own
/// starter: `move_all`, the automatic outline, the plug-in connectors)
/// with a `row` per matrix row (and half), its keys in column order at the
/// key pitch, each row below the one before (`relative`), each key's diode
/// `near` it, and a `region` per half of a split keyboard. `footprints`:
/// the board's parts, their footprint id and body size (unturned); keys
/// the board lacks are left out.
pub(crate) fn starter_constraints(
    matrix: &KeyMatrix,
    footprints: &BTreeMap<String, (String, [f64; 2])>,
    mut base: serde_json::Value,
) -> (serde_json::Value, KiCadStarterReport) {
    use serde_json::json;
    let mut report = KiCadStarterReport {
        rows_by: matrix.rows_by.to_string(),
        halves_by: matrix.halves_by.map(String::from),
        unassigned: matrix.unassigned.clone(),
        ..Default::default()
    };
    let on_board = |reference: &str| footprints.contains_key(reference);
    let (mut rows, mut near, mut relative, mut regions) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut row_nets = BTreeSet::new();
    let mut column_nets = BTreeSet::new();
    let mut pitches = Vec::new();
    let mut thumbs = Vec::new();
    let mut grids = 0;
    for group in &matrix.groups {
        let keys: Vec<&MatrixKey> = group.rows.iter().flat_map(|row| row.keys.iter()).filter(|key| on_board(&key.switch)).collect();
        if keys.is_empty() {
            continue;
        }
        let ids: Vec<&str> = keys.iter().map(|key| footprints[&key.switch].0.as_str()).collect();
        let standard = standard_pitch(&ids);
        let size = |reference: &str| footprints[reference].1;
        // Keys a unit wide set the pitch (a body wider than the standard
        // one, the rows' pitch grows to it); wider keys stay out of the rows.
        let unit: Vec<[f64; 2]> =
            keys.iter().map(|key| size(&key.switch)).filter(|size| size[0] <= standard[0] * 1.1 && size[1] <= standard[1] * 1.1).collect();
        let pitch = [
            unit.iter().map(|size| size[0]).fold(standard[0], f64::max),
            unit.iter().map(|size| size[1]).fold(standard[1], f64::max),
        ];
        let round = |value: f64| (value * 100.0).round() / 100.0;
        let pitch = [round(pitch[0]), round(pitch[1])];
        pitches.push(pitch);
        let longest = group.rows.iter().map(|row| row.keys.len()).max().unwrap_or(0);
        // Each row in runs of keys a unit wide on neighbouring columns (a
        // wide key or a column without a key here ends a run), first pass;
        // then whole rows over the same columns, one after the other,
        // become one grid.
        struct Line {
            members: Vec<String>,
            columns: Vec<String>,
            leader: String,
            height: f64,
            thumb: bool,
            /// The row's first run (the others follow it across a wide key).
            first: bool,
            /// The row is this one run.
            whole: bool,
            /// After a wide key: right of the run before, within this gap.
            right_of: Option<(String, f64)>,
        }
        let column_index = |column: &str| group.columns.iter().position(|other| other == column).unwrap_or(0);
        let mut lines: Vec<Line> = Vec::new();
        for row in &group.rows {
            row_nets.insert(row.net.clone());
            let present: Vec<&MatrixKey> = row.keys.iter().filter(|key| on_board(&key.switch)).collect();
            if present.is_empty() {
                continue;
            }
            column_nets.extend(present.iter().map(|key| key.column.clone()));
            // Wider or taller than a unit (1.5u, a vertical 2u, an ISO enter).
            let is_wide = |key: &MatrixKey| size(&key.switch)[0] > pitch[0] + 0.01 || size(&key.switch)[1] > pitch[1] + 0.01;
            for key in &present {
                report.switches += 1 + key.alternates.iter().filter(|alternate| on_board(alternate)).count();
                if let Some(diode) = key.diode.as_deref().filter(|diode| on_board(diode)) {
                    report.diodes += 1;
                    near.push(json!({"part": diode, "part_of": key.switch, "max_mm": DIODE_GAP_MM}));
                }
                for alternate in key.alternates.iter().filter(|alternate| on_board(alternate)) {
                    report.alternates.push(format!("{alternate} (with {})", key.switch));
                    near.push(json!({"part": alternate, "part_of": key.switch, "max_mm": DIODE_GAP_MM}));
                }
            }
            // A wide key beside its neighbour in the row.
            for (at, key) in present.iter().enumerate().filter(|(_, key)| is_wide(key)) {
                let neighbour = present[..at].iter().rev().chain(present[at + 1..].iter()).find(|other| !is_wide(other));
                report.wide.push(key.switch.clone());
                if let Some(neighbour) = neighbour {
                    near.push(json!({"part": key.switch, "part_of": neighbour.switch, "max_mm": 2.0}));
                }
            }
            // Short rows (half the longest or less) are thumb keys: below
            // the grid, given room.
            let thumb = 2 * present.len() <= longest && longest >= 3;
            if thumb {
                thumbs.push(format!("{} ({})", row.net, present.iter().map(|key| key.switch.as_str()).collect::<Vec<_>>().join(", ")));
            }
            // Runs, each with the width of the wide keys between it and the
            // run before (when only wide keys on neighbouring columns are).
            let mut runs: Vec<(Vec<&MatrixKey>, Option<f64>)> = Vec::new();
            let mut current: Vec<&MatrixKey> = Vec::new();
            let mut current_gap: Option<f64> = None;
            let mut across: Option<f64> = None;
            let mut previous_column: Option<usize> = None;
            for key in &present {
                let column = column_index(&key.column);
                let adjacent = previous_column.is_none_or(|previous| column == previous + 1);
                previous_column = Some(column);
                if !adjacent {
                    if !current.is_empty() {
                        runs.push((std::mem::take(&mut current), current_gap));
                    }
                    across = None;
                }
                if is_wide(key) {
                    if !current.is_empty() {
                        runs.push((std::mem::take(&mut current), current_gap));
                        across = Some(0.0);
                    }
                    across = across.map(|width| width + size(&key.switch)[0]);
                    continue;
                }
                if current.is_empty() {
                    current_gap = across.take();
                }
                current.push(key);
            }
            if !current.is_empty() {
                runs.push((current, current_gap));
            }
            let whole = runs.len() == 1 && !present.iter().any(|key| is_wide(key));
            let mut previous_leader: Option<String> = None;
            for (at, (run, gap)) in runs.iter().enumerate() {
                let leader = run[0].switch.clone();
                lines.push(Line {
                    members: run.iter().map(|key| key.switch.clone()).collect(),
                    columns: run.iter().map(|key| key.column.clone()).collect(),
                    leader: leader.clone(),
                    height: present.iter().map(|key| size(&key.switch)[1]).fold(0.0, f64::max),
                    thumb,
                    first: at == 0,
                    whole,
                    right_of: gap.and_then(|gap| previous_leader.clone().map(|leader| (leader, round(gap + 2.0)))),
                });
                previous_leader = Some(leader);
            }
            if runs.is_empty()
                && let Some(key) = present.first()
            {
                // Only wide keys: the first stands for the row.
                lines.push(Line {
                    members: vec![key.switch.clone()],
                    columns: vec![key.column.clone()],
                    leader: key.switch.clone(),
                    height: size(&key.switch)[1],
                    thumb,
                    first: true,
                    whole: false,
                    right_of: None,
                });
            }
        }
        let mut leaders: Vec<String> = Vec::new();
        let mut previous: Option<(String, f64)> = None;
        let mut at = 0;
        while at < lines.len() {
            let first = &lines[at];
            let mut end = at + 1;
            while end < lines.len()
                && first.whole
                && first.members.len() >= 2
                && !first.thumb
                && lines[end].whole
                && !lines[end].thumb
                && lines[end].columns == first.columns
            {
                end += 1;
            }
            let block = &lines[at..end];
            let height = block.iter().map(|line| line.height).fold(0.0, f64::max);
            if block.len() >= 2 {
                grids += 1;
                rows.push(json!({
                    "parts": block.iter().flat_map(|line| line.members.iter().cloned()).collect::<Vec<_>>(),
                    "pitch_mm": pitch[0], "columns": first.members.len(), "row_pitch_mm": pitch[1]}));
            } else if first.members.len() >= 2 {
                rows.push(json!({"parts": first.members, "pitch_mm": pitch[0]}));
            }
            if first.first {
                if let Some((above, above_height)) = &previous {
                    let gap = if first.thumb { 10.0 } else { round((pitch[1] - height.max(*above_height)).max(0.0) + 2.0) };
                    relative.push(json!({"part": first.leader, "below": above, "max_gap_mm": gap}));
                }
                previous = Some((first.leader.clone(), height));
            } else if let Some((left, gap)) = &first.right_of {
                relative.push(json!({"part": first.leader, "right_of": left, "max_gap_mm": gap}));
            }
            leaders.push(first.leader.clone());
            at = end;
        }
        let switches: usize = group.rows.iter().flat_map(|row| row.keys.iter()).filter(|key| on_board(&key.switch)).count();
        let half_at = group.half.map(|half| format!("{half}-half"));
        if let Some(at) = &half_at {
            let mut parts = leaders.clone();
            if let Some(controller) = group.controller.as_ref().filter(|controller| on_board(controller)) {
                parts.push(controller.clone());
            }
            regions.push(json!({"parts": parts, "at": at}));
        }
        report.halves.push(KiCadStarterHalf {
            at: half_at,
            rows: group.rows.iter().map(|row| row.net.clone()).collect(),
            columns: group.columns.clone(),
            switches,
            controller: group.controller.clone(),
        });
    }
    report.matrix = [row_nets.len(), column_nets.len()];
    report.pitch_mm = pitches.first().copied();
    let mut comment = vec![format!(
        "Starter constraints import-kicad-netlist inferred from the netlist alone: a key matrix of {} rows x {} columns, {} switches, {} with their own diode{}. Rows are the {} nets ({}).",
        report.matrix[0],
        report.matrix[1],
        report.switches,
        report.diodes,
        match matrix.halves_by {
            Some(how) => format!(", in two halves ({how})"),
            None => String::new(),
        },
        if matrix.rows_by == "names" { "row-named" } else { "fewer" },
        report.halves.iter().map(|half| half.rows.join(", ")).collect::<Vec<_>>().join("; ")
    )];
    comment.push(format!(
        "Each matrix row is a `row` of its keys in column order at {} mm (the key pitch of the switch footprints){}, each row `relative` below the one before, each key's diode `near` it within {DIODE_GAP_MM} mm{}.",
        report.pitch_mm.map_or("19.05".into(), |pitch| format!("{}", pitch[0])),
        if grids > 0 {
            format!(
                "; rows over the same columns one after the other are one grid (`columns`, `row_pitch_mm` {} mm)",
                report.pitch_mm.map_or("19.05".into(), |pitch| format!("{}", pitch[1]))
            )
        } else {
            String::new()
        },
        if regions.is_empty() { String::new() } else { ", and each half's rows and controller in a `region` of its half of the board".into() }
    ));
    let mut decide = vec![
        "the key pitch and the stagger (a `row` is a straight line at one pitch; a column-staggered board needs a `place` or `relative` per column)".to_string(),
        "the thumb clusters' position and angle".to_string(),
        "the controller's edge and its USB connector".to_string(),
        "the outline (`{}` sizes it from the parts)".to_string(),
    ];
    if !thumbs.is_empty() {
        decide[1] = format!("the thumb clusters ({}: placed below the rows, up to 10 mm away; turn or fan them as the design wants)", thumbs.join("; "));
    }
    let controllers: Vec<String> = report.halves.iter().filter_map(|half| half.controller.clone()).collect();
    if !controllers.is_empty() {
        decide[2] = format!("the controller's edge ({}: an `edge` or `place`) and its USB connector", controllers.join(", "));
    }
    if !report.wide.is_empty() {
        decide.push(format!("where the wide keys go ({}: near their neighbour in the row)", report.wide.join(", ")));
    }
    if !report.alternates.is_empty() {
        decide.push(format!("the alternate footprints of a key ({}: drawn over each other on the designer's board; near their key here)", report.alternates.join(", ")));
    }
    if !report.unassigned.is_empty() {
        decide.push(format!("the switches outside the matrix ({})", report.unassigned.join(", ")));
    }
    comment.push(format!("Needs your decision: {}.", decide.join("; ")));
    comment.push("To use it, copy this file to constraints.json (or name it in layout.json's placer) and edit it there: the import rewrites this file, never constraints.json.".into());
    base["comment"] = json!(comment);
    if !rows.is_empty() {
        base["row"] = rows.into();
    }
    if !near.is_empty() {
        base["near"] = near.into();
    }
    if !relative.is_empty() {
        base["relative"] = relative.into();
    }
    if !regions.is_empty() {
        base["region"] = regions.into();
    }
    (base, report)
}

/// Constraints as a person writes them: the keys in reading order, one
/// entry of a list per line.
pub(crate) fn constraints_text(value: &serde_json::Value) -> String {
    let Some(object) = value.as_object() else {
        return value.to_string();
    };
    let order = ["version", "comment", "move_all", "outline", "edge", "row", "near", "relative", "region"];
    let mut keys: Vec<&String> = object.keys().collect();
    keys.sort_by_key(|key| order.iter().position(|known| known == key).unwrap_or(order.len()));
    let mut text = String::from("{");
    for (index, key) in keys.iter().enumerate() {
        let item = &object[key.as_str()];
        text.push_str(if index == 0 { "\n  " } else { ",\n  " });
        text.push_str(&serde_json::Value::String(key.to_string()).to_string());
        text.push_str(": ");
        match item.as_array() {
            Some(entries) if !entries.is_empty() => {
                text.push('[');
                for (at, entry) in entries.iter().enumerate() {
                    text.push_str(if at == 0 { "\n    " } else { ",\n    " });
                    text.push_str(&entry.to_string());
                }
                text.push_str("\n  ]");
            }
            _ => text.push_str(&item.to_string()),
        }
    }
    text.push_str("\n}\n");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(reference: &str, footprint: &str, symbol: &str, pins: &[(&str, &str)]) -> MatrixPart {
        MatrixPart {
            reference: reference.into(),
            footprint: footprint.into(),
            symbol: symbol.into(),
            value: symbol.into(),
            pins: pins.iter().map(|(pin, net)| (pin.to_string(), net.to_string(), Some("passive".to_string()))).collect(),
        }
    }

    /// A keyboard matrix: `rows` x `columns` keys `SW<first + n>` (row by
    /// row), each switch from its column's net to its own diode, the
    /// diode's other pin on its row's net; `controller` has a pin on every
    /// line.
    fn keyboard(rows: &[&str], columns: &[&str], first: usize, controller: &str) -> Vec<MatrixPart> {
        let mut parts = Vec::new();
        let mut n = first;
        let mut pins = Vec::new();
        for row in rows {
            for column in columns {
                let private = format!("Net-(D{n}-A)");
                parts.push(part(&format!("SW{n}"), "lib:SW_Hotswap_Kailh_Choc_V1", "SW_Push", &[("1", column), ("2", &private)]));
                parts.push(part(&format!("D{n}"), "Diode_SMD:D_SOD-123", "D_Small", &[("1", row), ("2", &private)]));
                n += 1;
            }
        }
        for (at, line) in rows.iter().chain(columns.iter()).enumerate() {
            pins.push(((at + 1).to_string(), line.to_string()));
        }
        let pins: Vec<(&str, &str)> = pins.iter().map(|(pin, net)| (pin.as_str(), net.as_str())).collect();
        parts.push(part(controller, "lib:ProMicro", "ProMicro", &pins));
        parts
    }

    fn switches(line: &MatrixLine) -> Vec<&str> {
        line.keys.iter().map(|key| key.switch.as_str()).collect()
    }

    #[test]
    fn a_matrix_with_diodes_gives_rows_in_order_columns_in_order_and_each_key_its_diode() {
        // Listed out of order: COL2 first, ROW1 first.
        let mut parts = keyboard(&["ROW1", "ROW0"], &["COL2", "COL0", "COL1"], 1, "U1");
        parts.push(part("SW99", "Button_Switch_SMD:SW_Push", "SW_Push", &[("1", "RESET"), ("2", "GND")]));
        parts.push(part("D50", "LED_SMD:LED_0603", "LED", &[("1", "ROW0"), ("2", "+5V")]));
        let matrix = infer_key_matrix(&parts).unwrap();
        assert_eq!(matrix.groups.len(), 1);
        let group = &matrix.groups[0];
        assert_eq!(group.half, None);
        assert_eq!(matrix.rows_by, "names");
        assert_eq!(group.columns, ["COL0", "COL1", "COL2"]);
        assert_eq!(group.rows.iter().map(|row| row.net.as_str()).collect::<Vec<_>>(), ["ROW0", "ROW1"]);
        // ROW1's keys are SW1-SW3 on COL2, COL0, COL1: in column order.
        assert_eq!(switches(&group.rows[1]), ["SW2", "SW3", "SW1"]);
        assert_eq!(switches(&group.rows[0]), ["SW5", "SW6", "SW4"]);
        assert!(matrix.keys().all(|key| key.diode.as_deref() == Some(&key.switch.replace("SW", "D"))));
        assert_eq!(group.controller.as_deref(), Some("U1"));
        assert_eq!(matrix.unassigned, ["SW99"]);
        assert_eq!(matrix.halves_by, None);
    }

    #[test]
    fn rows_are_the_fewer_lines_when_names_do_not_say_and_order_follows_the_switches() {
        // Lines named for a controller's pins: three of one kind (on the
        // diodes' side), four of the other; numbers in pin names are no
        // order.
        let parts = keyboard(&["/P0.30", "/P0.02", "/P1.11"], &["/P0.07", "/P0.31", "/P0.05", "/P0.12"], 1, "U1");
        let matrix = infer_key_matrix(&parts).unwrap();
        assert_eq!(matrix.rows_by, "fewer nets");
        let group = &matrix.groups[0];
        assert_eq!(group.rows.iter().map(|row| row.net.as_str()).collect::<Vec<_>>(), ["/P0.30", "/P0.02", "/P1.11"]);
        assert_eq!(group.columns, ["/P0.07", "/P0.31", "/P0.05", "/P0.12"]);
        assert_eq!(switches(&group.rows[0]), ["SW1", "SW2", "SW3", "SW4"]);
    }

    #[test]
    fn two_separate_matrices_are_the_halves_their_names_say() {
        // The right half's switches come first; its nets say right.
        let mut parts = keyboard(&["R_ROW0", "R_ROW1"], &["R_COL0", "R_COL1", "R_COL2"], 1, "U2");
        parts.extend(keyboard(&["ROW0", "ROW1"], &["COL0", "COL1", "COL2"], 20, "U1"));
        let matrix = infer_key_matrix(&parts).unwrap();
        assert_eq!(matrix.halves_by, Some("separate matrices"));
        assert_eq!(matrix.groups.len(), 2);
        assert_eq!((matrix.groups[0].half, matrix.groups[0].controller.as_deref()), (Some("left"), Some("U1")));
        assert_eq!((matrix.groups[1].half, matrix.groups[1].controller.as_deref()), (Some("right"), Some("U2")));
        assert_eq!(switches(&matrix.groups[0].rows[0]), ["SW20", "SW21", "SW22"]);
    }

    #[test]
    fn one_matrix_with_columns_named_for_each_hand_is_split_in_halves() {
        // Rows shared by both hands, one controller: no half's own.
        let parts = keyboard(&["row_up", "row_home"], &["col_Lp", "col_Lr", "col_Rr", "col_Rp"], 1, "U1");
        let matrix = infer_key_matrix(&parts).unwrap();
        assert_eq!(matrix.halves_by, Some("column names"));
        let [left, right] = &matrix.groups[..] else { panic!("{matrix:?}") };
        assert_eq!((left.half, right.half), (Some("left"), Some("right")));
        assert_eq!(left.columns, ["col_Lp", "col_Lr"]);
        assert_eq!(switches(&right.rows[1]), ["SW7", "SW8"]);
        assert!(left.controller.is_none() && right.controller.is_none());
    }

    #[test]
    fn keys_wired_each_to_its_own_pin_are_no_matrix() {
        let mut parts: Vec<MatrixPart> = (1..=6)
            .map(|n| part(&format!("MX{n}"), "lib:SW_choc_mx_combined", "MX_SW", &[("1", "GND"), ("2", &format!("/KEY{n}"))]))
            .collect();
        parts.push(part("LED1", "lib:LED_MX_6028R", "MX_SK6812MINI-E", &[("1", "/KEY1"), ("2", "GND")]));
        let matrix = infer_key_matrix(&parts).unwrap();
        assert!(matrix.groups.is_empty());
        assert_eq!(matrix.unassigned.len(), 6, "the key LEDs are no switches: {:?}", matrix.unassigned);
        assert!(infer_key_matrix(&[part("R1", "R_0402", "R", &[("1", "A"), ("2", "B")])]).is_none());
    }

    #[test]
    fn alternate_footprints_share_a_key_and_a_column_diode_is_no_key_diode() {
        let mut parts = keyboard(&["row0", "row1"], &["col0", "col1", "col2"], 1, "U1");
        // SW9 is a second footprint of SW6 (same column, same diode net).
        parts.push(part("SW9", "lib:MX-Hotswap-Alps-2U", "SW_Push", &[("2", "col2"), ("1", "Net-(D6-A)")]));
        let matrix = infer_key_matrix(&parts).unwrap();
        let key = matrix.keys().find(|key| key.switch == "SW6").unwrap();
        assert_eq!((key.alternates.clone(), key.diode.as_deref()), (vec!["SW9".to_string()], Some("D6")));
        assert_eq!(matrix.keys().count(), 6);

        // A column whose keys reach their controller through one diode
        // they share: the diode's net is the column, each key keeps its own.
        let mut parts = Vec::new();
        for (n, (row, column)) in [("R0", "a"), ("R0", "b"), ("R1", "a"), ("R1", "b"), ("R2", "a"), ("R2", "b")].iter().enumerate() {
            let private = format!("Net-(D{}-A)", n + 10);
            parts.push(part(&format!("SW{}", n + 1), "lib:MX_1u", "SW_SPST", &[("1", &private), ("2", &format!("Net-(DC{column}-K)"))]));
            parts.push(part(&format!("D{}", n + 10), "lib:D_DO-35", "D", &[("2", &private), ("1", row)]));
        }
        for column in ["a", "b"] {
            parts.push(part(&format!("DC{column}"), "lib:D_DO-35", "D", &[("1", &format!("Net-(DC{column}-K)")), ("2", &format!("C{column}"))]));
        }
        let matrix = infer_key_matrix(&parts).unwrap();
        let group = &matrix.groups[0];
        assert_eq!(group.columns, ["Net-(DCa-K)", "Net-(DCb-K)"]);
        assert_eq!(group.rows.len(), 3);
        assert!(matrix.keys().all(|key| key.alternates.is_empty() && key.diode.as_deref().is_some_and(|diode| diode != "DCa")));
    }

    #[test]
    fn a_two_pin_part_on_a_row_and_a_column_is_a_key_and_a_ground_pin_is_left_out() {
        let mut parts = keyboard(&["ROW0", "ROW1"], &["COL0", "COL1"], 1, "U1");
        // A switch with a third pin on ground (a shield), a pad not named
        // as a switch that sits on a row and a column.
        parts.push(part("SW5", "PG1316:CPG1316S01D02", "sw", &[("1", "COL2"), ("2", "Net-(D5-A)"), ("3", "GND")]));
        parts.push(part("D5", "lib:D_SOD-123", "D", &[("1", "ROW0"), ("2", "Net-(D5-A)")]));
        parts.push(part("B7", "capacitive:pad_1U", "EC", &[("1", "ROW1"), ("2", "COL2")]));
        // COL2 has one named key: the controller's pin on it makes it a
        // column.
        parts.iter_mut().find(|part| part.reference == "U1").unwrap().pins.push(("9".into(), "COL2".into(), None));
        let matrix = infer_key_matrix(&parts).unwrap();
        let group = &matrix.groups[0];
        assert_eq!(group.columns, ["COL0", "COL1", "COL2"]);
        assert_eq!(switches(&group.rows[0]), ["SW1", "SW2", "SW5"]);
        assert_eq!(switches(&group.rows[1]), ["SW3", "SW4", "B7"]);
        assert!(matrix.unassigned.is_empty());
    }

    #[test]
    fn the_starter_has_a_grid_per_run_of_full_rows_thumbs_below_diodes_near_and_a_region_per_half() {
        // The left half: 3 x 4 keys, a thumb row of two on the last
        // columns; the right half 3 x 3, a 1.5u key in the middle of its
        // second row.
        let mut parts = keyboard(&["ROW0", "ROW1", "ROW2"], &["COL0", "COL1", "COL2", "COL3"], 1, "U1");
        for (n, column) in [(13, "COL2"), (14, "COL3")] {
            let private = format!("Net-(D{n}-A)");
            parts.push(part(&format!("SW{n}"), "lib:SW_Hotswap_Kailh_Choc_V1", "SW_Push", &[("1", column), ("2", &private)]));
            parts.push(part(&format!("D{n}"), "lib:D", "D", &[("1", "ROW3"), ("2", &private)]));
        }
        parts.extend(keyboard(&["R_ROW0", "R_ROW1", "R_ROW2"], &["R_COL0", "R_COL1", "R_COL2"], 20, "U2"));
        let matrix = infer_key_matrix(&parts).unwrap();
        let mut footprints = BTreeMap::new();
        for part in &parts {
            let size = if part.reference == "SW24" { [27.0, 16.0] } else if part.reference.starts_with("SW") { [17.5, 16.5] } else { [2.0, 1.0] };
            footprints.insert(part.reference.clone(), (part.footprint.clone(), size));
        }
        let base = serde_json::json!({"version": 1, "move_all": true, "outline": {}});
        let (starter, report) = starter_constraints(&matrix, &footprints, base);
        assert_eq!(report.matrix, [7, 7]);
        assert_eq!((report.switches, report.diodes), (23, 23));
        assert_eq!(report.pitch_mm, Some([18.0, 17.0]));
        assert_eq!(report.wide, ["SW24"]);
        let rows = starter["row"].as_array().unwrap();
        // Left: a 3 x 4 grid, the thumbs; right: R_ROW0, R_ROW1 in two
        // runs either side of the wide key (one key each: no row), R_ROW2.
        assert_eq!(rows[0], serde_json::json!({"parts": ["SW1", "SW2", "SW3", "SW4", "SW5", "SW6", "SW7", "SW8", "SW9", "SW10", "SW11", "SW12"],
                                               "pitch_mm": 18.0, "columns": 4, "row_pitch_mm": 17.0}));
        assert_eq!(rows[1], serde_json::json!({"parts": ["SW13", "SW14"], "pitch_mm": 18.0}));
        assert_eq!(rows[2]["parts"], serde_json::json!(["SW20", "SW21", "SW22"]));
        assert_eq!(rows[3]["parts"], serde_json::json!(["SW26", "SW27", "SW28"]));
        let relative = starter["relative"].as_array().unwrap();
        assert_eq!(relative, &vec![
            serde_json::json!({"part": "SW13", "below": "SW1", "max_gap_mm": 10.0}),
            serde_json::json!({"part": "SW23", "below": "SW20", "max_gap_mm": 2.5}),
            // Across the 27 mm wide key, right of the key before it.
            serde_json::json!({"part": "SW25", "right_of": "SW23", "max_gap_mm": 29.0}),
            serde_json::json!({"part": "SW26", "below": "SW23", "max_gap_mm": 2.5}),
        ]);
        let near = starter["near"].as_array().unwrap();
        assert!(near.contains(&serde_json::json!({"part": "D5", "part_of": "SW5", "max_mm": 3.0})));
        assert!(near.contains(&serde_json::json!({"part": "SW24", "part_of": "SW23", "max_mm": 2.0})));
        assert_eq!(starter["region"], serde_json::json!([{"parts": ["SW1", "SW13", "U1"], "at": "left-half"},
                                                         {"parts": ["SW20", "SW23", "SW25", "SW26", "U2"], "at": "right-half"}]));
        assert!(starter["comment"].as_array().unwrap().iter().any(|line| line.as_str().unwrap().starts_with("Needs your decision")));
        // What the placer reads: the file parses as constraints.
        let text = constraints_text(&starter);
        assert!(text.starts_with("{\n  \"version\": 1,\n  \"comment\": ["), "{text}");
        let parsed: crate::KiCadPlacementConstraints = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.row.len(), rows.len());
    }
}
