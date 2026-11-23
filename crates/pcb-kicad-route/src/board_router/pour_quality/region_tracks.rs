// Copyright (C) 2026 Florian Loitsch.

//! Stored straight track centrelines inside generated rectangular pours.

use super::*;
use crate::board_router::pour_plan::{PAD_REGION_PREFIX, region_name};
use crate::kicad_nets::{item_net, net_codes};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct KiCadBoundedTrackLength {
    /// Length inside the union, counting portions shared by regions once.
    pub other_straight_track_mm: f64,
    pub regions: Vec<KiCadRegionTrackLength>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct KiCadRegionTrackLength {
    pub zone_uuid: String,
    pub bounds_mm: [f64; 4],
    pub other_straight_track_mm: f64,
}

fn rectangle(polygon: &Expr) -> Option<[f64; 4]> {
    if !polygon
        .child("pts")?
        .children()
        .iter()
        .skip(1)
        .all(|edge| edge.head() == Some("xy"))
    {
        return None;
    }
    let points = outline::pts_points(polygon).ok()?;
    let [a, b, c, d] = points.as_slice() else {
        return None;
    };
    let axis_aligned = (a[1] == b[1] && b[0] == c[0] && c[1] == d[1] && d[0] == a[0])
        || (a[0] == b[0] && b[1] == c[1] && c[0] == d[0] && d[1] == a[1]);
    (axis_aligned && a[0] != c[0] && a[1] != c[1]).then_some([
        a[0].min(c[0]),
        a[1].min(c[1]),
        a[0].max(c[0]),
        a[1].max(c[1]),
    ])
}

/// The portion of a segment strictly inside a rectangle, parameterized 0..1.
fn inside_interval(start: [f64; 2], end: [f64; 2], bounds: [f64; 4]) -> Option<[f64; 2]> {
    let (mut low, mut high) = (0.0_f64, 1.0_f64);
    for axis in 0..2 {
        let delta = end[axis] - start[axis];
        let (min, max) = (bounds[axis], bounds[axis + 2]);
        if delta == 0.0 {
            if start[axis] <= min || start[axis] >= max {
                return None;
            }
            continue;
        }
        let enter = (min - start[axis]) / delta;
        let exit = (max - start[axis]) / delta;
        low = low.max(enter.min(exit));
        high = high.min(enter.max(exit));
    }
    (low < high).then_some([low, high])
}

fn union_length(intervals: &mut [[f64; 2]]) -> f64 {
    intervals.sort_unstable_by(|[a, _], [b, _]| a.total_cmp(b));
    let mut last_end = 0.0_f64;
    intervals
        .iter()
        .map(|&[start, end]| {
            let added = (end - start.max(last_end)).max(0.0);
            last_end = last_end.max(end);
            added
        })
        .sum()
}

pub(super) fn measure(pcb: &Expr) -> BTreeMap<(String, String), KiCadBoundedTrackLength> {
    let zones = pcb.children().iter().filter(|item| {
        item.head() == Some("zone")
            && !is_rule_area(item)
            && form_atom(item, "name", 1).is_some_and(|name| name.starts_with(PAD_REGION_PREFIX))
    });
    if zones.clone().next().is_none() {
        return BTreeMap::new();
    }
    let names = NetNames::of(pcb);
    let codes = net_codes(pcb);
    let mut groups = BTreeMap::<(String, String), KiCadBoundedTrackLength>::new();
    for zone in zones {
        let (Some(net), Some(layer), Some(name), Some(uuid)) = (
            item_net(zone, &codes).filter(|name| !name.is_empty()),
            form_atom(zone, "layer", 1),
            form_atom(zone, "name", 1),
            form_atom(zone, "uuid", 1).or_else(|| form_atom(zone, "tstamp", 1)),
        ) else {
            continue;
        };
        let net = names.normalize(net);
        if name != region_name(net, layer) || form_atom(zone, "fill", 1) == Some("no") {
            continue;
        }
        let Some(bounds_mm) = zone.child("polygon").and_then(rectangle) else {
            continue;
        };
        groups
            .entry((net.into(), layer.into()))
            .or_insert_with(|| KiCadBoundedTrackLength {
                regions: Vec::new(),
                other_straight_track_mm: 0.0,
            })
            .regions
            .push(KiCadRegionTrackLength {
                zone_uuid: uuid.into(),
                bounds_mm,
                other_straight_track_mm: 0.0,
            });
    }
    if groups.is_empty() {
        return groups;
    }
    let mut intervals = Vec::new();
    for track in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("segment"))
    {
        let (Some(net), Some(layer), Ok(start), Ok(end)) = (
            item_net(track, &codes)
                .filter(|name| !name.is_empty())
                .map(|name| names.normalize(name)),
            form_atom(track, "layer", 1),
            form_xy(track, "start"),
            form_xy(track, "end"),
        ) else {
            continue;
        };
        let length = distance_squared(start, end).sqrt();
        for ((pour_net, pour_layer), group) in &mut groups {
            if net == pour_net || layer != pour_layer {
                continue;
            }
            intervals.clear();
            for region in &mut group.regions {
                if let Some([inside_start, inside_end]) =
                    inside_interval(start, end, region.bounds_mm)
                {
                    intervals.push([inside_start, inside_end]);
                    region.other_straight_track_mm += length * (inside_end - inside_start);
                }
            }
            group.other_straight_track_mm += length * union_length(&mut intervals);
        }
    }
    for group in groups.values_mut() {
        group.regions.sort_by(|a, b| a.zone_uuid.cmp(&b.zone_uuid));
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_track_lengths_ignore_beside_and_boundary_tangent_tracks() {
        let pcb = parse(r#"(kicad_pcb
          (net 1 "rail") (net 2 "signal")
          (zone (net 1) (layer "In2.Cu") (name "pcb-maker pad region rail In2.Cu") (uuid "left")
            (polygon (pts (xy 0 0) (xy 4 0) (xy 4 4) (xy 0 4))))
          (zone (net "rail") (layer "In2.Cu") (name "pcb-maker pad region rail In2.Cu") (uuid "right")
            (polygon (pts (xy 3 0) (xy 7 0) (xy 7 4) (xy 3 4))))
          (segment (start -1 2) (end 8 2) (layer "In2.Cu") (net 2))
          (segment (start 1 0.5) (end 2 0.5) (layer "In2.Cu") (net "signal"))
          (segment (start -1 5) (end 8 5) (layer "In2.Cu") (net "signal"))
          (segment (start -1 0) (end 8 0) (layer "In2.Cu") (net "signal"))
          (segment (start -1 3) (end 8 3) (layer "F.Cu") (net "signal"))
          (segment (start -1 2) (end 8 2) (layer "In2.Cu") (net "rail"))
          (arc (start 1 1) (mid 2 2) (end 3 1) (layer "In2.Cu") (net "signal")))"#).unwrap();
        let groups = measure(&pcb);
        let group = &groups[&("rail".into(), "In2.Cu".into())];
        assert!((group.other_straight_track_mm - 8.0).abs() < 1.0e-12);
        let [left, right] = group.regions.as_slice() else {
            panic!("both zones measured")
        };
        assert_eq!(left.zone_uuid, "left");
        assert!((left.other_straight_track_mm - 5.0).abs() < 1.0e-12);
        assert_eq!(right.zone_uuid, "right");
        assert!((right.other_straight_track_mm - 4.0).abs() < 1.0e-12);
    }
}
