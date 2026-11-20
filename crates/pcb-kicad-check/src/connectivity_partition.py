# Copyright (C) 2026 Florian Loitsch.

"""Read named copper components with the KiCad PCB bindings."""

import hashlib
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

import pcbnew

MAX_ISLANDS = 128
MAX_FILLED_VERTICES = 250_000


def track_geometry(item):
    if item.Type() != pcbnew.PCB_TRACE_T:
        raise ValueError("ambiguous native copper UUID on a non-segment")
    return (
        item.GetNetname(), item.GetLayer(), item.GetStart().x, item.GetStart().y,
        item.GetEnd().x, item.GetEnd().y, item.GetWidth(),
    )


def partition(board):
    by_uuid = defaultdict(list)
    for item in list(board.GetPads()) + list(board.GetTracks()):
        if item.GetNetCode() <= 0:
            continue
        uid = item.m_Uuid.AsString().lower()
        if not re.fullmatch(
            r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}", uid
        ):
            raise ValueError("invalid native copper UUID")
        if uid == "00000000-0000-0000-0000-000000000000" or not item.GetNetname():
            raise ValueError("native copper without an identity or net name")
        by_uuid[uid].append(item)
    copper = {}
    by_address = {}
    duplicates = []
    for uid, items in by_uuid.items():
        if len(items) == 1:
            identities = [uid]
        else:
            # Some boards contain literally duplicate straight tracks. Include
            # each instance, but give no UUID-only DRC witness credit to any.
            # A collision across different geometry cannot certify a handoff.
            if len({track_geometry(item) for item in items}) != 1:
                raise ValueError("native copper UUID collision across different geometry")
            identities = [f"{uid}#{index}" for index in range(len(items))]
            duplicates.append(set(identities))
        for identity, item in zip(identities, items):
            copper[identity] = item
            by_address[int(item.this)] = identity

    connectivity = board.GetConnectivity()
    seen = set()
    groups = []
    group_by_id = {}
    for uid, item in copper.items():
        if uid in seen:
            continue
        members = set()
        for other in connectivity.GetConnectedItems(item):
            if other.Type() != pcbnew.PCB_ZONE_T:
                member = by_address.get(int(other.this))
                if member is None:
                    raise ValueError("KiCad connectivity contains uncatalogued copper")
                members.add(member)
        net = item.GetNetname()
        if uid not in members or seen & members or not members <= copper.keys():
            raise ValueError("KiCad did not return a complete copper partition")
        if any(copper[member].GetNetname() != net for member in members):
            raise ValueError("KiCad connectivity component crosses net names")
        group_by_id.update((member, len(groups)) for member in members)
        groups.append((net, sorted(members)))
        seen.update(members)
    if any(len({group_by_id[member] for member in group}) != 1 for group in duplicates):
        raise ValueError("identically indexed copper occupies different native components")
    return sorted(groups)


def contour_identity(contour):
    points = [(contour.CPoint(i).x, contour.CPoint(i).y)
              for i in range(contour.PointCount())]
    if len(points) < 3:
        raise ValueError("invalid native filled polygon contour")
    lowest = min(points)
    return min(sequence[i:] + sequence[:i]
               for sequence in (points, points[::-1])
               for i, point in enumerate(sequence) if point == lowest)


def island_topology(board):
    """Give each native filled polygon its own item in a diagnostic board."""
    if any(not z.GetIsRuleArea() for fp in board.GetFootprints() for z in fp.Zones()):
        raise ValueError("footprint-owned copper zones are unsupported")
    ordinary = partition(board)
    pieces = []
    keys = set()
    shapes = []
    vertices = 0
    for zone in list(board.Zones()):
        if zone.GetIsRuleArea():
            continue
        layers = zone.GetLayerSet().CuStack()
        if len(layers) != 1 or not zone.GetNetname():
            raise ValueError("fill islands require named single-layer zones")
        layer = layers[0]
        polygons = zone.GetFilledPolysList(layer)
        count = polygons.OutlineCount()
        if len(pieces) + count > MAX_ISLANDS:
            raise ValueError("too many native fill islands for a bounded proof")
        first = None
        for index in range(count):
            shape = pcbnew.SHAPE_POLY_SET()
            shape.AddOutline(polygons.Outline(index))
            for hole in range(polygons.HoleCount(index)):
                shape.AddHole(polygons.Hole(index, hole), 0)
            box = shape.BBox()
            key = (zone.GetNetname(), board.GetLayerName(layer), zone.IsTeardropArea(),
                   zone.IsIsland(layer, index),
                   (box.GetLeft(), box.GetTop(), box.GetRight(), box.GetBottom()),
                   "" if zone.IsTeardropArea() else zone.m_Uuid.AsString().lower())
            if key in keys:
                raise ValueError("native fill islands have ambiguous identities")
            keys.add(key)
            outline = shape.Outline(0)
            holes = [shape.Hole(0, i) for i in range(shape.HoleCount(0))]
            vertices += outline.PointCount() + sum(hole.PointCount() for hole in holes)
            if vertices > MAX_FILLED_VERTICES:
                raise ValueError("too many native filled vertices for a bounded proof")
            contours = [contour_identity(outline), sorted(map(contour_identity, holes))]
            shapes.append((key, hashlib.sha256(json.dumps(contours).encode()).hexdigest()))
            item = zone
            if index == 0:
                first = shape
            else:
                item = pcbnew.Cast_to_ZONE(zone.Duplicate(False))
                if item.m_Uuid.AsString() == zone.m_Uuid.AsString():
                    raise ValueError("a native island clone has no independent identity")
                item.SetFilledPolysList(layer, shape)
                item.SetIsFilled(True)
                board.Add(item)
            pieces.append((key, item))
        if count > 1:
            zone.SetFilledPolysList(layer, first)
    if not board.BuildConnectivity() or ordinary != partition(board):
        raise ValueError("isolating the fill polygons changed ordinary native connectivity")
    group_of = {uid: (net, members) for net, members in ordinary for uid in members}
    islands = []
    for key, zone in pieces:
        members = sorted({item.m_Uuid.AsString().lower()
                          for item in board.GetConnectivity().GetConnectedItems(zone)
                          if item.Type() != pcbnew.PCB_ZONE_T})
        if not members or any(group_of.get(uid) != (key[0], members) for uid in members):
            raise ValueError("a native fill island lacks one unambiguous ordinary component")
        islands.append((key, members))
    return {"ordinary": ordinary, "islands": sorted(islands), "shapes": sorted(shapes)}


def main():
    path, result, mode, version, *extras = sys.argv[1:]
    if pcbnew.GetBuildVersion().strip() != version:
        raise ValueError("KiCad CLI and PCB bindings have different versions")
    board = pcbnew.LoadBoard(path)
    if not board or not board.BuildConnectivity():
        raise ValueError("KiCad could not build board connectivity")
    if mode == "refill":
        if not pcbnew.ZONE_FILLER(board).Fill(board.Zones()) or not board.BuildConnectivity():
            raise ValueError("KiCad could not build refilled connectivity")
    if extras not in ([], ["islands"]):
        raise ValueError("unsupported connectivity probe")
    graph = island_topology(board) if extras else partition(board)
    Path(result).write_text(json.dumps(graph), encoding="utf-8")


if __name__ == "__main__":
    main()
