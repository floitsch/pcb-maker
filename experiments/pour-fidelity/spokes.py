# Copyright (C) 2026 Toit contributors.
"""Thermal spokes per pad: KiCad's fill against the router's pour model.

    python3 -I spokes.py <board.kicad_pcb> <dump-directory> [--drc drc.json] [--json out.json]

The dump directory comes from `pcb-maker dump-pour-map <dir> <id> <out>` on
the same (routed) board: `<net>.spokes` holds, per pad and layer, what the
router's model makes of each of the pad's four spokes (none, blocked,
island, good).

KiCad's side: the board's zones are filled as `kicad-cli pcb drc
--refill-zones` fills them, and each thermal pad's spokes are counted the
way KiCad's DRC counts them (DRC_TEST_PROVIDER_ZONE_CONNECTIONS): the
crossings of the pad's outline with each fill outline, halved; crossings
with an isolated island counted apart. With `--drc`, the pads KiCad's DRC
reported starved are marked, so the count can be checked against it.

Per pad and layer the row says KiCad's count, ours (good spokes) and which
directions KiCad's spokes leave in (the direction of each crossing pair's
midpoint from the pad's centre), so a spoke we miss can be named.
"""

import argparse
import json
import math
import os
import re
import sys

import pcbnew

NM = 1.0e6


def resolved_connection(pad, zone):
    connection = pad.GetLocalZoneConnection()
    if connection == pcbnew.ZONE_CONNECTION_INHERITED:
        connection = pad.GetParentFootprint().GetLocalZoneConnection()
    if connection == pcbnew.ZONE_CONNECTION_INHERITED:
        connection = zone.GetPadConnection()
    if connection == pcbnew.ZONE_CONNECTION_THT_THERMAL:
        connection = pcbnew.ZONE_CONNECTION_THERMAL if pad.GetAttribute() == pcbnew.PAD_ATTRIB_PTH else pcbnew.ZONE_CONNECTION_FULL
    return {
        pcbnew.ZONE_CONNECTION_THERMAL: "thermal",
        pcbnew.ZONE_CONNECTION_FULL: "solid",
        pcbnew.ZONE_CONNECTION_NONE: "none",
    }.get(connection, str(connection))


def pad_label(pad):
    return f"{pad.GetParentFootprint().GetReference()}.{pad.GetNumber()}"


def fill(board):
    zones = [zone for zone in board.Zones() if not zone.GetIsRuleArea()]
    filler = pcbnew.ZONE_FILLER(board)
    filler.Fill(pcbnew.ZONES(zones))
    return zones


def segment_crossings(chain_a, chain_b):
    """Proper crossings of two closed polylines (in mm), by brute force."""
    def segments(chain):
        count = chain.PointCount()
        points = [(chain.CPoint(i).x / NM, chain.CPoint(i).y / NM) for i in range(count)]
        return [(points[i], points[(i + 1) % count]) for i in range(count)]

    result = []
    b_segments = segments(chain_b)
    bx0 = min(min(a[0], b[0]) for a, b in b_segments)
    bx1 = max(max(a[0], b[0]) for a, b in b_segments)
    by0 = min(min(a[1], b[1]) for a, b in b_segments)
    by1 = max(max(a[1], b[1]) for a, b in b_segments)
    for p, q in segments(chain_a):
        if max(p[0], q[0]) < bx0 or min(p[0], q[0]) > bx1 or max(p[1], q[1]) < by0 or min(p[1], q[1]) > by1:
            continue
        for r, s in b_segments:
            d = (q[0] - p[0]) * (s[1] - r[1]) - (q[1] - p[1]) * (s[0] - r[0])
            if d == 0:
                continue
            t = ((r[0] - p[0]) * (s[1] - r[1]) - (r[1] - p[1]) * (s[0] - r[0])) / d
            u = ((r[0] - p[0]) * (q[1] - p[1]) - (r[1] - p[1]) * (q[0] - p[0])) / d
            if 0 <= t < 1 and 0 <= u < 1:
                result.append((p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])))
    return result


def direction_name(dx, dy, round_pad):
    angle = math.degrees(math.atan2(dy, dx)) % 360
    if round_pad:
        names = ["+x+y", "-x+y", "-x-y", "+x-y"]
        return names[int(((angle - 45) % 360 + 45) // 90) % 4]
    names = ["+x", "+y", "-x", "-y"]
    return names[int(((angle + 45) % 360) // 90)]


def outline_items(board, zones, zone, layer, polys, index, net_items):
    """How many items KiCad's connectivity joins to fill outline `index`
    (CN_VISITOR::checkZoneItemConnection), counted up to two: pads and vias
    by their flashed shape, tracks by theirs, other zones of the net on the
    layer by overlap."""
    unit = polys.UnitSet(index)
    box = unit.BBox()
    count = 0
    for item in net_items:
        if not item.IsOnLayer(layer) or not box.Intersects(item.GetBoundingBox()):
            continue
        if unit.Collide(item.GetEffectiveShape(layer, pcbnew.FLASHING_ALWAYS_FLASHED) if hasattr(pcbnew, "FLASHING_ALWAYS_FLASHED") else item.GetEffectiveShape(layer)):
            count += 1
            if count > 1:
                return count
    for other in zones:
        if other is zone or other.GetNetCode() != zone.GetNetCode() or not other.IsOnLayer(layer):
            continue
        others = other.GetFilledPolysList(layer)
        if others.OutlineCount() and box.Intersects(others.BBox()):
            overlap = pcbnew.SHAPE_POLY_SET(unit)
            overlap.BooleanIntersection(others)
            if overlap.OutlineCount():
                count += 1
                if count > 1:
                    return count
    return count


def kicad_spokes(board, zones):
    """Per (pad label, layer name): KiCad's spoke count as its DRC makes
    it (DRC_TEST_PROVIDER_ZONE_CONNECTIONS::testZoneLayer), the spokes into
    single-connection outlines apart, the directions and the settings."""
    result = {}
    by_net = {}
    items_by_net = {}
    for footprint in board.GetFootprints():
        for pad in footprint.Pads():
            by_net.setdefault(pad.GetNetCode(), []).append(pad)
            items_by_net.setdefault(pad.GetNetCode(), []).append(pad)
    for track in board.GetTracks():
        items_by_net.setdefault(track.GetNetCode(), []).append(track)
    connectivity = board.GetConnectivity()
    for zone in zones:
        if zone.GetNetCode() <= 0:
            continue
        for layer in zone.GetLayerSet().Seq():
            if not pcbnew.IsCopperLayer(layer):
                continue
            polys = zone.GetFilledPolysList(layer)
            if polys.OutlineCount() == 0:
                continue
            box = polys.BBox()
            layer_name = str(board.GetLayerName(layer))
            single_cache = {}
            for pad in by_net.get(zone.GetNetCode(), []):
                if not pad.FlashLayer(layer):
                    continue
                if not zone.GetBoundingBox().Intersects(pad.GetBoundingBox()):
                    continue
                connection = resolved_connection(pad, zone)
                gap_override = pad.GetLocalThermalGapOverride()
                gap = gap_override if isinstance(gap_override, int) and gap_override > 0 else zone.GetThermalReliefGap()
                outline = pcbnew.SHAPE_POLY_SET()
                pad.TransformShapeToPolygon(outline, layer, gap // 2, pcbnew.ARC_LOW_DEF if hasattr(pcbnew, "ARC_LOW_DEF") else 5000, pcbnew.ERROR_OUTSIDE)
                if outline.OutlineCount() == 0:
                    continue
                pad_chain = outline.Outline(0)
                pad_box = pad_chain.BBox()
                spokes = 0
                ignored = 0
                directions = []
                centre = pad.GetPosition()
                round_pad = pad.GetShape(layer) == pcbnew.PAD_SHAPE_CIRCLE
                for index in range(polys.OutlineCount()):
                    chain = polys.Outline(index)
                    if not chain.BBox().Intersects(pad_box):
                        continue
                    points = sorted(set(segment_crossings(chain, pad_chain)))
                    if len(points) < 2:
                        continue
                    if index not in single_cache:
                        single_cache[index] = outline_items(board, zones, zone, layer, polys, index, items_by_net.get(zone.GetNetCode(), [])) == 1
                    single = single_cache[index]
                    if single:
                        ignored += len(points) // 2
                    else:
                        spokes += len(points) // 2
                    names = sorted(direction_name(x - centre.x / NM, y - centre.y / NM, round_pad) for x, y in points)
                    directions.extend(f"{name}{'*' if single else ''}" for name in names)
                tracks = 0
                if connection == "thermal" and (spokes or ignored) and spokes < 2:
                    for track in connectivity.GetConnectedTracks(pad):
                        start, end = track.GetStart(), track.GetEnd()
                        # As the DRC: a track starting in the ring counts
                        # by its end only.
                        if pad_chain.PointInside(start):
                            tracks += polys.Collide(end)
                        elif pad_chain.PointInside(end):
                            tracks += polys.Collide(start)
                key = (pad_label(pad), layer_name, centre.x / NM, centre.y / NM)
                row = {
                    "pad": key[0],
                    "layer": layer_name,
                    "net": str(pad.GetNetname()),
                    "at": [centre.x / NM, centre.y / NM],
                    "size": [pad.GetSize(layer).x / NM, pad.GetSize(layer).y / NM] if pad.GetSize(layer) is not None else None,
                    "shape": int(pad.GetShape(layer)),
                    "connection": connection,
                    "kicad": spokes,
                    "kicad_single": ignored,
                    "kicad_tracks": tracks,
                    "directions": directions,
                    "spoke_angle": pad.GetThermalSpokeAngleDegrees(),
                    "spoke_width": zone.GetThermalReliefSpokeWidth() / NM,
                    "gap": gap / NM,
                }
                previous = result.get(key)
                if previous is None or row["kicad"] > previous["kicad"]:
                    result[key] = row
    return result


def kicad_starved(row):
    """The DRC's verdict from the counts (minimum 2)."""
    spokes, ignored = row.get("kicad", 0), row.get("kicad_single", 0)
    if spokes == 0 and ignored == 0:
        return False
    spokes += row.get("kicad_tracks", 0)
    if spokes == 0:
        spokes = ignored
    return spokes < 2


def our_starved(row):
    """The model's verdict, by the DRC's rule on its counts."""
    if "ours" not in row:
        return False
    spokes, ignored = row["ours"], row.get("ours_island", 0)
    if spokes == 0 and ignored == 0:
        return False
    spokes += row.get("ours_tracks", 0)
    if spokes == 0:
        spokes = ignored
    return spokes < 2


def ours(dump):
    names = json.load(open(os.path.join(dump, "layers.json")))
    result = {}
    for name in os.listdir(dump):
        if not name.endswith(".spokes"):
            continue
        for line in open(os.path.join(dump, name)):
            row = json.loads(line)
            key = (row["pad"], names[row["layer"]], row["anchor"][0], row["anchor"][1])
            good = sum(1 for spoke in row["spokes"] if spoke.startswith("good"))
            island = sum(1 for spoke in row["spokes"] if spoke.startswith("island"))
            result[key] = {"ours": good, "ours_island": island, "ours_tracks": row.get("tracks", 0), "our_spokes": row["spokes"], "round": row["round"]}
    return result


def starved_from_drc(path):
    """(label, layer) -> [(position, description)] of the DRC's starved pads."""
    result = {}
    for violation in json.load(open(path)).get("violations", []):
        if violation["type"] != "starved_thermal":
            continue
        layer = re.search(r"layer ([^;]+);", violation["description"]).group(1)
        for item in violation["items"]:
            match = re.match(r"(?:PTH |SMD |NPTH )?[Pp]ad (\S+) \[[^\]]*\] of (\S+)", item["description"])
            if match:
                result.setdefault((f"{match.group(2)}.{match.group(1)}", layer), []).append(((item["pos"]["x"], item["pos"]["y"]), violation["description"]))
    return result


def match_starved(starved, kicad):
    """Per row key, the DRC description of the row the report names (the
    nearest of the pads sharing a label)."""
    result = {}
    for (label, layer), reports in starved.items():
        candidates = [key for key in kicad if key[0] == label and key[1] == layer]
        for position, description in reports:
            if not candidates:
                result[(label, layer, None, None)] = description
                continue
            best = min(candidates, key=lambda key: (key[2] - position[0]) ** 2 + (key[3] - position[1]) ** 2)
            result[best] = description
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("board")
    parser.add_argument("dump")
    parser.add_argument("--drc")
    parser.add_argument("--json")
    parser.add_argument("--save-filled")
    parser.add_argument("--all", action="store_true", help="print every pad, not only those where KiCad and ours differ")
    args = parser.parse_args()
    board = pcbnew.LoadBoard(args.board)
    zones = fill(board)
    if args.save_filled:
        board.Save(args.save_filled)
    kicad = kicad_spokes(board, zones)
    model = ours(args.dump)
    starved = match_starved(starved_from_drc(args.drc), kicad) if args.drc else {}
    rows = []
    by_label = {}
    for key in model:
        by_label.setdefault(key[:2], []).append(key)

    def model_row(key):
        """Our pad of that label and layer nearest the pad (within 0.05 mm)."""
        best = None
        for candidate in by_label.get(key[:2], []):
            distance = (candidate[2] - key[2]) ** 2 + (candidate[3] - key[3]) ** 2
            if distance < 0.05 ** 2 and (best is None or distance < best[0]):
                best = (distance, candidate)
        return model[best[1]] if best else None

    for key, row in sorted(kicad.items()):
        if row["connection"] != "thermal":
            continue
        mine = model_row(key)
        row = dict(row)
        if mine:
            row.update(mine)
        row["drc_starved"] = starved.get(key)
        rows.append(row)
    for key in starved:
        if key not in kicad:
            rows.append({"pad": key[0], "layer": key[1], "drc_starved": starved[key], "missing": True})
    for row in rows:
        row.setdefault("at", None)
    if args.json:
        json.dump(rows, open(args.json, "w"), indent=1)
    for row in rows:
        if args.all or row.get("drc_starved") or kicad_starved(row) != our_starved(row):
            print(json.dumps({k: row.get(k) for k in ["pad", "layer", "kicad", "kicad_single", "kicad_tracks", "directions", "ours", "our_spokes", "drc_starved", "spoke_angle"]}))
    check = [row for row in rows if kicad_starved(row) != bool(row.get("drc_starved"))]
    print(f"# {len(rows)} thermal pad layers; DRC starved {len(starved)}; spoke rule disagrees with the DRC on {len(check)}", file=sys.stderr)
    for row in check:
        print("# check:", json.dumps({k: row.get(k) for k in ["pad", "layer", "kicad", "kicad_single", "kicad_tracks", "drc_starved"]}), file=sys.stderr)


if __name__ == "__main__":
    main()
