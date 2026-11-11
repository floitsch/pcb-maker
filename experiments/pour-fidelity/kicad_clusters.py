# Copyright (C) 2026 Toit contributors.
"""KiCad's connectivity of the pads of some nets, after its own refill.

    python3 -I kicad_clusters.py <board.kicad_pcb> <out.json> <net>...

Refills every zone as KiCad's DRC does (ZONE_FILLER), builds the
connectivity and writes, per pad of the named nets, its place, copper
layers and whether it lies in the net's largest cluster of connected
pads (the cluster `GetConnectedItems` returns: pads, tracks, vias and
fill pieces that touch). A pad outside the largest cluster is one KiCad's
DRC reports among its unconnected items (agreement.py).
"""

import json
import sys

import pcbnew


def main():
    board_path, out, nets = sys.argv[1], sys.argv[2], set(sys.argv[3:])
    board = pcbnew.LoadBoard(board_path)
    pcbnew.ZONE_FILLER(board).Fill(board.Zones())
    board.BuildConnectivity()
    connectivity = board.GetConnectivity()
    pads_by_net = {}
    for footprint in board.GetFootprints():
        for pad in footprint.Pads():
            if pad.GetNetname() in nets:
                pads_by_net.setdefault(pad.GetNetname(), []).append((footprint.GetReference(), pad))
    rows = []
    for net, pads in sorted(pads_by_net.items()):
        cluster_of = {}
        sizes = []
        for _, pad in pads:
            key = pad.m_Uuid.AsString()
            if key in cluster_of:
                continue
            members = [pad] + [item for item in connectivity.GetConnectedItems(pad) if item.GetClass() == "PAD"]
            sizes.append(0)
            for member in members:
                member_key = member.m_Uuid.AsString()
                if member_key not in cluster_of:
                    cluster_of[member_key] = len(sizes) - 1
                    sizes[-1] += 1
        largest = max(range(len(sizes)), key=lambda index: (sizes[index], -index)) if sizes else -1
        for reference, pad in pads:
            cluster = cluster_of[pad.m_Uuid.AsString()]
            rows.append({
                "net": net,
                "pad": f"{reference}.{pad.GetNumber()}",
                "at": [pcbnew.ToMM(pad.GetPosition().x), pcbnew.ToMM(pad.GetPosition().y)],
                "layers": [pcbnew.BOARD.GetStandardLayerName(layer) for layer in pad.GetLayerSet().CuStack()],
                "cluster": cluster,
                "cluster_pads": sizes[cluster],
                "connected": cluster == largest,
            })
    with open(out, "w") as handle:
        json.dump(rows, handle)


if __name__ == "__main__":
    main()
