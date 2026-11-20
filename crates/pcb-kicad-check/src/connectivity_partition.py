# Copyright (C) 2026 Florian Loitsch.

"""Read named copper components with the KiCad PCB bindings."""

import json
import re
import sys
from collections import defaultdict
from pathlib import Path

import pcbnew


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


def main():
    path, result, mode, version = sys.argv[1:]
    if pcbnew.GetBuildVersion().strip() != version:
        raise ValueError("KiCad CLI and PCB bindings have different versions")
    board = pcbnew.LoadBoard(path)
    if not board or not board.BuildConnectivity():
        raise ValueError("KiCad could not build board connectivity")
    if mode == "refill":
        if not pcbnew.ZONE_FILLER(board).Fill(board.Zones()) or not board.BuildConnectivity():
            raise ValueError("KiCad could not build refilled connectivity")
    Path(result).write_text(json.dumps(partition(board)), encoding="utf-8")


if __name__ == "__main__":
    main()
