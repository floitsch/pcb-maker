# Copyright (C) 2026 Florian Loitsch.
"""Measure the shortest planar trace path between named PCB pads.

Tracks are split at same-layer centreline junctions and where they enter
native pad or via copper. Crossing a contact contributes zero length here.
The metric covers planar trace spans. It excludes vertical barrel length
and gives no estimates of return paths, impedance, or inductance.
"""

from collections import defaultdict
from dataclasses import dataclass
from heapq import heappop, heappush
from itertools import combinations, count
import math


class UnsupportedPath(ValueError):
    pass


def point(vector):
    return vector.x, vector.y


def sub(a, b):
    return a[0] - b[0], a[1] - b[1]


def cross(a, b):
    return a[0] * b[1] - a[1] * b[0]


def dot(a, b):
    return a[0] * b[0] + a[1] * b[1]


def distance_to_chord(at, start, end):
    chord, offset = sub(end, start), sub(at, start)
    length2 = dot(chord, chord)
    if not length2:
        return math.dist(at, start)
    t = max(0, min(1, dot(offset, chord) / length2))
    return math.hypot(offset[0] - t * chord[0], offset[1] - t * chord[1])


@dataclass(frozen=True)
class Track:
    start: tuple
    end: tuple
    layer: int
    width: int
    uuid: str

    def node(self, t):
        # Sub-nanometre rounding makes both sides of an exact crossing
        # share one graph node even when it lies between KiCad grid units.
        delta = sub(self.end, self.start)
        return self.layer, tuple(round(a + t * d, 4) for a, d in zip(self.start, delta))


def split_at_junction(a, b, a_cuts, b_cuts):
    v, w = sub(a.end, a.start), sub(b.end, b.start)
    delta = sub(b.start, a.start)
    determinant = cross(v, w)
    if determinant:
        t, u = cross(delta, w) / determinant, cross(delta, v) / determinant
        if 0 <= t <= 1 and 0 <= u <= 1:
            a_cuts.add(t)
            b_cuts.add(u)
    elif cross(delta, v) == 0:
        # Each end in a collinear overlap is also a junction on the other
        # segment. This covers the shared end and the overlap's boundaries.
        for segment, ends, cuts in ((a, b, a_cuts), (b, a, b_cuts)):
            direction = sub(segment.end, segment.start)
            for end in (ends.start, ends.end):
                t = dot(sub(end, segment.start), direction) / dot(direction, direction)
                if 0 <= t <= 1:
                    cuts.add(t)


def clip_convex(track, polygon):
    """The part of a centreline inside a counter-clockwise polygon."""
    first, last = 0.0, 1.0
    direction = sub(track.end, track.start)
    for a, b in zip(polygon, polygon[1:] + polygon[:1]):
        edge = sub(b, a)
        at_start, step = cross(edge, sub(track.start, a)), cross(edge, direction)
        if step > 0:
            first = max(first, -at_start / step)
        elif step < 0:
            last = min(last, -at_start / step)
        elif at_start < 0:
            return None
        if last < first:
            return None
    return first, last


def pad_polygon(pcbnew, pad, layer):
    if pad.GetShape(layer) == pcbnew.PAD_SHAPE_CUSTOM:
        raise UnsupportedPath("custom copper pad geometry on this net")
    shape = pad.GetEffectivePolygon(layer, pcbnew.ERROR_INSIDE)
    if shape.OutlineCount() != 1 or shape.HoleCount(0) != 0:
        raise UnsupportedPath("multi-island or cutout pad copper on this net")
    ring = shape.Outline(0)
    polygon = [point(ring.CPoint(i)) for i in range(ring.PointCount())]
    area = sum(cross(a, b) for a, b in zip(polygon, polygon[1:] + polygon[:1]))
    if not area:
        raise UnsupportedPath("degenerate pad copper on this net")
    if area < 0:
        polygon.reverse()
    closed = polygon + polygon[:2]
    if any(cross(sub(b, a), sub(c, b)) < 0 for a, b, c in zip(closed, closed[1:], closed[2:])):
        raise UnsupportedPath("non-convex pad copper on this net")
    return polygon


def clip_disc(track, centre, radius):
    v = sub(track.end, track.start)
    offset = sub(centre, track.start)
    length2 = dot(v, v)
    # Perpendicular distance to the infinite centreline, then clip the
    # chord to the finite segment.
    reach2 = radius * radius - cross(v, offset) ** 2 / length2
    if reach2 < 0:
        return None
    middle = dot(offset, v) / length2
    reach = math.sqrt(reach2 / length2)
    first, last = max(0.0, middle - reach), min(1.0, middle + reach)
    return (first, last) if first <= last else None


def add_link(graph, a, b, weight, track=None):
    if a != b:
        graph[a].append((b, weight, track))
        graph[b].append((a, weight, track))


def make_graph(pcbnew, board, net, pads, track_items=None, *, track_only=False, max_tracks=None):
    """Build track-centreline contacts. A track-only census excludes zone copper."""
    footprints = list(board.GetFootprints())
    zones = [*board.Zones(), *(zone for fp in footprints for zone in fp.Zones())]
    if not track_only and any(zone.GetNetCode() == net and not zone.GetIsRuleArea() for zone in zones):
        raise UnsupportedPath("a named copper zone on this net can provide an unmeasured path")
    graphics = [*board.GetDrawings(), *(shape for fp in footprints for shape in fp.GraphicalItems())]
    if any(isinstance(shape, pcbnew.PCB_SHAPE) and shape.IsOnCopperLayer() and shape.GetNetCode() == net
           for shape in graphics):
        raise UnsupportedPath("a copper graphic on this net can provide an unmeasured path")
    tracks, vias = [], []
    item_count = 0
    for item in board.GetTracks() if track_items is None else track_items:
        if item.GetNetCode() != net:
            continue
        item_count += 1
        if max_tracks is not None and item_count > max_tracks:
            raise UnsupportedPath(f"the target net exceeds the {max_tracks}-item track/via measurement limit")
        if isinstance(item, pcbnew.PCB_VIA):
            if item.GetViaType() != pcbnew.VIATYPE_THROUGH:
                raise UnsupportedPath("a non-through via on this net is outside the measurement")
            vias.append(item)
        elif item.GetClass() == "PCB_TRACK":
            a, b = point(item.GetStart()), point(item.GetEnd())
            if a != b:
                tracks.append(Track(a, b, item.GetLayer(), item.GetWidth(), item.m_Uuid.AsString()))
        else:
            raise UnsupportedPath("an arc or another unsupported track shape is present on this net")

    graph = defaultdict(list)
    cuts = [{0.0, 1.0} for _ in tracks]
    by_layer = defaultdict(list)
    for index, track in enumerate(tracks):
        by_layer[track.layer].append(index)
    for indices in by_layer.values():
        for a, b in combinations(indices, 2):
            split_at_junction(tracks[a], tracks[b], cuts[a], cuts[b])

    # Through pads and vias with copper on more than one layer connect
    # their planar contacts. A surface pad only has one copper face.
    contacts = defaultdict(list)
    items = [(("pad", identity), pad) for identity, pad in pads.items()]
    items.extend((("via", via.m_Uuid.AsString()), via) for via in vias)
    for key, item in items:
        layers = [layer for layer in by_layer if item.FlashLayer(layer)]
        is_pad = key[0] == "pad"
        if is_pad and len(layers) > 1 and item.GetAttribute() != pcbnew.PAD_ATTRIB_PTH:
            raise UnsupportedPath("non-plated pad with copper on more than one layer")
        for layer in layers:
            polygon = pad_polygon(pcbnew, item, layer) if is_pad else None
            for index in by_layer[layer]:
                track = tracks[index]
                interval = (clip_convex(track, polygon) if polygon is not None
                            else clip_disc(track, point(item.GetPosition()), item.GetWidth(layer) / 2))
                if interval is not None:
                    cuts[index].update(interval)
                    contacts[index].append((interval, key))

    for index, (track, cuts_on_track) in enumerate(zip(tracks, cuts)):
        parameters = sorted(cuts_on_track)
        vertices = [track.node(t) for t in parameters]
        for a, b in zip(vertices, vertices[1:]):
            add_link(graph, a, b, math.dist(a[1], b[1]), track)
        for (first, last), contact in contacts[index]:
            for t, vertex in zip(parameters, vertices):
                if first <= t <= last:
                    add_link(graph, contact, vertex, 0.0)
    return graph


def shortest(graph, source, target):
    # Prefer fewer edges among equal-length paths. A tie counter keeps
    # nodes for physical contacts and planar points comparable in the heap.
    tie = count()
    search = [(0.0, 0, next(tie), source)]
    best = {source: (0.0, 0)}
    parent = {}
    while search:
        length, hops, _, current = heappop(search)
        if (length, hops) != best[current]:
            continue
        if current == target:
            route = []
            while current != source:
                previous, track = parent[current]
                route.append((previous, current, track))
                current = previous
            return length, list(reversed(route))
        for other, weight, track in graph[current]:
            score = length + weight, hops + 1
            if score < best.get(other, (math.inf, math.inf)):
                best[other] = score
                parent[other] = current, track
                heappush(search, (*score, next(tie), other))
    raise UnsupportedPath("no route found through supported track centreline contacts")


def measure_supply_paths(pcbnew, board, pairs, *, max_tracks_per_net=None):
    """Measure each (REF:pad, REF:pad) pair or give its unavailable reason."""
    pads = []
    pad_labels = defaultdict(list)
    pads_by_net = defaultdict(dict)
    for footprint in board.GetFootprints():
        for pad in footprint.Pads():
            label = f"{footprint.GetReference()}:{pad.GetNumber()}"
            # One graph contact per physical pad, including repeated pin
            # numbers. Only the user-facing endpoint lookup uses labels.
            identity = len(pads)
            pads.append((label, pad))
            pad_labels[label].append(identity)
            pads_by_net[pad.GetNetCode()][identity] = pad

    def pad_contact(identity):
        label, pad = pads[identity]
        return {"pad": label, "pad_uuid": pad.m_Uuid.AsString()}

    graphs = {}
    rows = []
    for source, target in pairs:
        row = {"from": source, "to": target}
        rows.append(row)
        try:
            if source == target:
                raise UnsupportedPath("the requested pads are the same")
            if len(pad_labels[source]) != 1 or len(pad_labels[target]) != 1:
                raise UnsupportedPath("a requested pad is missing or ambiguous")
            source_id, target_id = pad_labels[source][0], pad_labels[target][0]
            source_pad, target_pad = pads[source_id][1], pads[target_id][1]
            net = source_pad.GetNetCode()
            if net <= 0 or net != target_pad.GetNetCode():
                raise UnsupportedPath("the requested pads do not share a named net")
            start, end = point(source_pad.GetPosition()), point(target_pad.GetPosition())
            row["net"] = source_pad.GetNetname()
            row["pad_center_distance_mm"] = round(pcbnew.ToMM(math.dist(start, end)), 6)
            if net not in graphs:
                try:
                    graphs[net] = make_graph(pcbnew, board, net, pads_by_net[net], max_tracks=max_tracks_per_net)
                except UnsupportedPath as error:
                    graphs[net] = error
            if isinstance(graphs[net], UnsupportedPath):
                raise graphs[net]
            length, route = shortest(graphs[net], ("pad", source_id), ("pad", target_id))
            layer_name = board.GetStandardLayerName
            def point_mm(at):
                return [round(pcbnew.ToMM(v), 6) for v in at]

            spans = [{"uuid": track.uuid, "layer": layer_name(track.layer),
                      "track_start_mm": point_mm(track.start), "track_end_mm": point_mm(track.end),
                      "track_width_mm": round(pcbnew.ToMM(track.width), 6),
                      "from_mm": point_mm(a[1]), "to_mm": point_mm(b[1])}
                     for a, b, track in route if track is not None]
            changes = []
            for (before, contact, _), (_, after, _) in zip(route, route[1:]):
                kind, identity = contact
                if kind in ("via", "pad") and before[0] != after[0]:
                    change = {"kind": kind, "from": layer_name(before[0]), "to": layer_name(after[0])}
                    if kind == "pad":
                        pad = pad_contact(identity)
                        change.update(contact=pad["pad"], pad_uuid=pad["pad_uuid"])
                    else:
                        change["contact"] = identity
                    changes.append(change)
            offset = max((distance_to_chord(node[1], start, end)
                          for a, b, track in route if track is not None
                          for node in (a, b)), default=0)
            terminal_contacts = [
                {**pad_contact(identity), "component_face": layer_name(pad.GetParent().GetLayer()),
                 "trace_layer": layer_name(node[0])}
                for identity, pad, node in ((source_id, source_pad, route[0][1]), (target_id, target_pad, route[-1][0]))
            ]
            intermediate = [a[1] for a, _, _ in route[1:] if a[0] == "pad"]
            intermediate_contacts = []
            for identity in intermediate:
                at = point(pads[identity][1].GetPosition())
                intermediate_contacts.append({
                    **pad_contact(identity),
                    "centre_mm": point_mm(at),
                    "distance_from_pad_chord_mm": round(pcbnew.ToMM(distance_to_chord(at, start, end)), 6),
                })
            row.update({"status": "measured", "trace_length_mm": round(pcbnew.ToMM(length), 6),
                        "max_offset_from_pad_chord_mm": round(pcbnew.ToMM(offset), 6),
                        "layers": list(dict.fromkeys(span["layer"] for span in spans)),
                        "terminal_contacts": terminal_contacts,
                        "layer_changes": changes,
                        "intermediate_pads": [contact["pad"] for contact in intermediate_contacts],
                        "intermediate_pad_contacts": intermediate_contacts,
                        "vias": [a[1] for a, _, _ in route if a[0] == "via"],
                        "trace_segments": spans})
        except UnsupportedPath as error:
            row.update({"status": "unavailable", "reason": str(error)})
        except RuntimeError as error:
            row.update({"status": "unavailable", "reason": f"native copper geometry unavailable: {error}"})
    return rows


def measure_near_paths(pcbnew, board, entries, *, only_net=None, max_tracks_per_net=None):
    """Resolve source pads for authored body-to-pin and explicit pad pairs."""
    footprints = defaultdict(list)
    for footprint in board.GetFootprints():
        footprints[footprint.GetReference()].append(footprint)
    if only_net and not only_net.startswith("/"):
        # The router drops one hierarchical slash only when the board has
        # no bare net of that name. Prefer an exact native bare label.
        names = {pad.GetNetname() for copies in footprints.values() for footprint in copies
                 for pad in footprint.Pads()}
        if only_net not in names and f"/{only_net}" in names:
            only_net = f"/{only_net}"

    def pads(ref):
        if len(footprints[ref]) != 1:
            raise UnsupportedPath(f"{ref} does not identify exactly one footprint")
        return list(footprints[ref][0].Pads())

    rows, requests, pending = [], [], []
    for entry in entries:
        part, pin_of = entry["part"], entry["pin_of"]
        ref, sep, pin = pin_of.partition(":")
        number = entry.get("pad")
        row = {"part": part, "pin_of": pin_of, "routed_path": {"to": pin_of}}
        rows.append(row)
        path = row["routed_path"]
        try:
            if not (sep and ref and pin):
                raise UnsupportedPath("pin_of must name REF:PAD")
            target = [pad for pad in pads(ref) if pad.GetNumber() == pin]
            if only_net is not None and not any(pad.GetNetname() == only_net for pad in target):
                rows.pop()
                continue
            if len(target) != 1:
                raise UnsupportedPath("the target pad number does not identify exactly one physical pad")
            net = target[0].GetNetCode()
            if net <= 0 or not target[0].GetNetname():
                raise UnsupportedPath("the target pad has no named net")
            path["net"] = target[0].GetNetname()
            source = pads(part)
            if number is None:
                same_net = [pad.GetNumber() for pad in source if pad.GetNetCode() == net]
                if not same_net:
                    raise UnsupportedPath("the source footprint has no pad on the target net")
                if len(same_net) > 1:
                    path["candidate_source_pads"] = sorted(f"{part}:{number}" for number in same_net)
                    raise UnsupportedPath("the source footprint has multiple pads on the target net; specify pad explicitly")
                number, = same_net
                if not number:
                    raise UnsupportedPath("the matching source pad has no pad number")
            matching = [pad for pad in source if pad.GetNumber() == number]
            if len(matching) != 1:
                raise UnsupportedPath("the source pad number does not identify exactly one physical pad")
            if matching[0].GetNetCode() != net:
                raise UnsupportedPath("the specified source pad is on a different net from the target")
            requests.append((f"{part}:{number}", pin_of))
            pending.append(row)
        except UnsupportedPath as error:
            path.update(status="unavailable", reason=str(error))
            if number:
                path["from"] = f"{part}:{number}"
    measured = measure_supply_paths(pcbnew, board, requests, max_tracks_per_net=max_tracks_per_net)
    for row, path in zip(pending, measured, strict=True):
        row["routed_path"] = path
    return rows
