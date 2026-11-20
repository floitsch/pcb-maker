// Copyright (C) 2026 Florian Loitsch.

//! Keep generated straight segments off exactly overlapping retained runs
//! and materialize identical generated segments once.

use super::*;

type Point = [i32; 2];
type Run = [Point; 2];

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Line {
    direction: [i64; 2],
    offset: i128,
}

impl Line {
    fn through([a, b]: Run) -> Option<Self> {
        let [x, y] = [0, 1].map(|axis| i64::from(b[axis]) - i64::from(a[axis]));
        let (mut gcd, mut remainder) = (x.abs(), y.abs());
        while remainder != 0 {
            (gcd, remainder) = (remainder, gcd % remainder);
        }
        if gcd == 0 {
            return None;
        }
        let [dx, dy] = [x / gcd, y / gcd];
        Some(Self {
            direction: [dx, dy],
            offset: i128::from(dx) * i128::from(a[1]) - i128::from(dy) * i128::from(a[0]),
        })
    }
}

fn whole_nanometres(mm: f64) -> Option<i32> {
    let nm = mm * 1.0e6;
    let rounded = nm.round();
    // Allow floating-point conversion noise in decimals already on KiCad's
    // integer lattice. Leave finer or out-of-range source forms alone.
    ((nm - rounded).abs() <= 0.00001)
        .then(|| i32::try_from(rounded as i64).ok())
        .flatten()
}

fn point([x, y]: [f64; 2]) -> Option<Point> {
    Some([whole_nanometres(x)?, whole_nanometres(y)?])
}

fn millimetres(point: Point) -> [f64; 2] {
    point.map(|nm| f64::from(nm) / 1.0e6)
}

type RunsByLine = BTreeMap<(i32, Line), Vec<Run>>;

#[derive(Default)]
pub(super) struct RetainedSegments {
    coverage: BTreeMap<String, BTreeMap<String, RunsByLine>>,
    emitted: BTreeMap<String, usize>,
}

impl RetainedSegments {
    pub(super) fn of(pcb: &Expr) -> Self {
        let codes = net_codes(pcb);
        let mut coverage = Self::default();
        for item in pcb.children() {
            if let Some((net, layer, width, line, run)) = read_segment(item, &codes) {
                coverage
                    .coverage
                    .entry(net.to_string())
                    .or_default()
                    .entry(layer.to_string())
                    .or_default()
                    .entry((width, line))
                    .or_default()
                    .push(run);
            }
        }
        for layers in coverage.coverage.values_mut() {
            for lines in layers.values_mut() {
                for runs in lines.values_mut() {
                    runs.sort_unstable();
                    let mut merged: Vec<Run> = Vec::with_capacity(runs.len());
                    for &[start, end] in runs.iter() {
                        if let Some(previous) = merged.last_mut()
                            && previous[1] >= start
                        {
                            previous[1] = previous[1].max(end);
                        } else {
                            merged.push([start, end]);
                        }
                    }
                    *runs = merged;
                }
            }
        }
        coverage
    }

    /// The pieces of a new segment outside retained copper, in its original
    /// direction. An intact segment needs no allocation.
    fn uncovered(&self, native_net: &str, segment: &SupplementalSegment) -> Option<Vec<Run>> {
        let lines = self.coverage.get(native_net)?.get(&segment.layer)?;
        let mut run = [point(segment.start)?, point(segment.end)?];
        let reverse = run[1] < run[0];
        run.sort_unstable();
        let [start, end] = run;
        let line = Line::through(run)?;
        let cover = lines.get(&(whole_nanometres(segment.width)?, line))?;
        let first = cover.partition_point(|item| item[1] <= start);
        if cover.get(first)?[0] >= end {
            return None;
        }
        let mut cursor = start;
        let mut result = Vec::new();
        for &[start, stop] in &cover[first..] {
            if start >= end {
                break;
            }
            if start > cursor {
                result.push([cursor, start]);
            }
            cursor = stop;
            if cursor >= end {
                break;
            }
        }
        if cursor < end {
            result.push([cursor, end]);
        }
        if reverse {
            result.reverse();
            for piece in &mut result {
                piece.swap(0, 1);
            }
        }
        Some(result)
    }

    fn append(&mut self, items: &mut Vec<Expr>, segment: &SupplementalSegment) -> bool {
        let item = supplemental_segment_expr(segment);
        let id = form_atom(&item, "uuid", 1).expect("generated segment UUID");
        if let Some(index) = self.emitted.get(id) {
            if items[*index] == item {
                return false;
            }
        } else {
            self.emitted.insert(id.to_owned(), items.len());
        }
        items.push(item);
        true
    }

    /// Append generated copper and return the length already present.
    /// The union of same-width collinear capsules is unchanged by clipping.
    pub(super) fn emit(
        &mut self,
        items: &mut Vec<Expr>,
        native_net: Option<&str>,
        mut segment: SupplementalSegment,
    ) -> f64 {
        let length = distance_squared(segment.start, segment.end).sqrt();
        let Some(pieces) = native_net.and_then(|net| self.uncovered(net, &segment)) else {
            return if self.append(items, &segment) {
                0.0
            } else {
                length
            };
        };
        let mut emitted = 0.0;
        for [start, end] in pieces {
            segment.start = millimetres(start);
            segment.end = millimetres(end);
            if self.append(items, &segment) {
                emitted += distance_squared(segment.start, segment.end).sqrt();
            }
        }
        (length - emitted).max(0.0)
    }
}

fn read_segment<'a>(
    item: &'a Expr,
    codes: &'a BTreeMap<String, String>,
) -> Option<(&'a str, &'a str, i32, Line, Run)> {
    if item.head() != Some("segment") {
        return None;
    }
    let net = item_net(item, codes).filter(|name| !name.is_empty())?;
    let layer = form_atom(item, "layer", 1)?;
    let width = whole_nanometres(form_f64(item, "width", 1).ok()?)?;
    if width <= 0 {
        return None;
    }
    let mut run = [
        point(form_xy(item, "start").ok()?)?,
        point(form_xy(item, "end").ok()?)?,
    ];
    run.sort_unstable();
    Some((net, layer, width, Line::through(run)?, run))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(start: [f64; 2], end: [f64; 2]) -> SupplementalSegment {
        SupplementalSegment {
            connection: "Rail".into(),
            start,
            end,
            width: 0.6,
            layer: "B.Cu".into(),
        }
    }

    #[test]
    fn emission_removes_duplicates_and_clips_runs_without_editing_retained_items() {
        let pcb = parse(r#"(kicad_pcb
            (net 1 "Rail")
            (segment (start 1 3) (end 2 2) (width 0.6) (layer "B.Cu") (net 1) (uuid "first") (locked yes))
            (segment (start 3 1) (end 2 2) (width 0.6) (layer "B.Cu") (net 1) (uuid "second"))
            (segment (start 4 0) (end 5 -1) (width 0.6) (layer "B.Cu") (net 1) (uuid "third")))"#)
            .unwrap();
        note_net_names(&pcb);
        let existing = pcb.children();
        let mut coverage = RetainedSegments::of(&pcb);
        let mut items = existing.to_vec();
        let exact = coverage.emit(&mut items, Some("Rail"), segment([2.0, 2.0], [1.0, 3.0]));
        let interior = coverage.emit(&mut items, Some("Rail"), segment([1.5, 2.5], [2.5, 1.5]));
        assert_eq!(items, existing);
        assert_eq!(exact, 2.0_f64.sqrt());
        assert_eq!(interior, 2.0_f64.sqrt());

        let covered = coverage.emit(&mut items, Some("Rail"), segment([6.0, -2.0], [0.0, 4.0]));
        assert!((covered - 3.0 * 2.0_f64.sqrt()).abs() < 1.0e-12);
        assert_eq!(&items[..existing.len()], existing);
        let emitted: Vec<_> = items[existing.len()..]
            .iter()
            .map(|item| {
                [
                    form_xy(item, "start").unwrap(),
                    form_xy(item, "end").unwrap(),
                ]
            })
            .collect();
        assert_eq!(
            emitted,
            [
                [[6.0, -2.0], [5.0, -1.0]],
                [[4.0, 0.0], [3.0, 1.0]],
                [[1.0, 3.0], [0.0, 4.0]],
            ]
        );
    }

    #[test]
    fn duplicate_generated_segments_do_not_change_authored_copper() {
        note_net_names(&parse("(kicad_pcb)").unwrap());
        let track = segment([1.0, 3.0], [2.0, 2.0]);
        let authored = supplemental_segment_expr(&track);
        let mut items = vec![authored.clone(), authored.clone()];
        let mut writer = RetainedSegments::default();
        assert_eq!(writer.emit(&mut items, None, track.clone()), 0.0);
        assert_eq!(writer.emit(&mut items, None, track.clone()), 2.0_f64.sqrt());
        let mut other = track;
        other.width = 0.61;
        assert_eq!(writer.emit(&mut items, None, other.clone()), 0.0);
        assert_eq!(&items[..3], &[authored.clone(), authored.clone(), authored]);
        assert_eq!(items[3], supplemental_segment_expr(&other));
        assert_eq!(items.len(), 4);
    }

    #[test]
    fn only_exact_same_net_width_layer_and_line_supply_coverage() {
        let pcb = parse(
            r#"(kicad_pcb
            (segment (start 1 2) (end 1 8) (width 0.6) (layer "B.Cu") (net "/Rail"))
            (segment (start 1 2) (end 1 8) (width 0.6) (layer "F.Cu") (net "Rail"))
            (segment (start 1 2) (end 1 8) (width 0.600001) (layer "B.Cu") (net "Rail"))
            (segment (start 1.000001 2) (end 1.000001 8) (width 0.6) (layer "B.Cu") (net "Rail"))
            (arc (start 1 2) (mid 1.1 5) (end 1 8) (width 0.6) (layer "B.Cu") (net "Rail")))"#,
        )
        .unwrap();
        let retained = RetainedSegments::of(&pcb);
        let vertical = segment([1.0, 3.0], [1.0, 7.0]);
        assert_eq!(retained.uncovered("Rail", &vertical), None);
        assert_eq!(retained.uncovered("/Rail", &vertical), Some(Vec::new()));
        let touching = segment([1.0, 1.0], [1.0, 2.0]);
        assert_eq!(retained.uncovered("/Rail", &touching), None);
    }
}
