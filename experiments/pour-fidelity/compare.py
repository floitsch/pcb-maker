# Copyright (C) 2026 Toit contributors.
"""Compares the router's pour map with KiCad's zone fill on the same board.

    python3 -I compare.py <board.kicad_pcb> <dump-directory> [--json out.json]

The dump directory comes from `pcb-maker dump-pour-map <dir> <id> <out>` (the
board as it is, its copper fixed) or from a run with PCB_ROUTER_POUR_DUMP set
(`finish-NNN` subdirectories, one per finished routing). Each `<net>-L<n>.pour`
holds per lattice node the flags of `Router::dump_pours` and the piece labels.

KiCad's fill is taken twice: with islands kept (the geometry) and as the
board's zones say (islands removed). Its brush centres are the fill deflated
by half the minimum width, sampled at the lattice nodes: that is what our free
mask models, so the two are compared node by node, and the pieces of each
(4- and 8-connected) are counted against KiCad's outlines.
"""

import argparse
import json
import math
import os
import sys

import numpy as np
import pcbnew

NM = 1.0e6


def load_dump(path):
    with open(path, "rb") as handle:
        header = json.loads(handle.readline())
        nx, ny = header["nx"], header["ny"]
        flags = np.frombuffer(handle.read(nx * ny), dtype=np.uint8).reshape(ny, nx)
        labels = np.frombuffer(handle.read(4 * nx * ny), dtype="<u4").reshape(ny, nx)
    return header, flags, labels


def rings_of(polys):
    """Per outline: the list of rings (outline first, then holes), in mm."""
    result = []
    for index in range(polys.OutlineCount()):
        rings = []
        for chain in [polys.Outline(index)] + [polys.Hole(index, hole) for hole in range(polys.HoleCount(index))]:
            count = chain.PointCount()
            points = np.array([[chain.CPoint(i).x, chain.CPoint(i).y] for i in range(count)], dtype=float) / NM
            if len(points) >= 3:
                rings.append(points)
        result.append(rings)
    return result


def rasterize(rings, grid, mask=None, value=True):
    """Marks the nodes inside `rings` (even-odd) in `mask`."""
    nx, ny, ox, oy, pitch = grid
    if mask is None:
        mask = np.zeros((ny, nx), dtype=bool)
    if not rings:
        return mask
    edges = np.concatenate([np.stack([ring, np.roll(ring, -1, axis=0)], axis=1) for ring in rings])
    x0, y0, x1, y1 = edges[:, 0, 0], edges[:, 0, 1], edges[:, 1, 0], edges[:, 1, 1]
    top = max(0, math.ceil((min(y0.min(), y1.min()) - oy) / pitch))
    bottom = min(ny - 1, math.floor((max(y0.max(), y1.max()) - oy) / pitch))
    for iy in range(top, bottom + 1):
        y = oy + iy * pitch
        crossing = (y0 > y) != (y1 > y)
        if not crossing.any():
            continue
        xs = x0[crossing] + (y - y0[crossing]) * (x1[crossing] - x0[crossing]) / (y1[crossing] - y0[crossing])
        xs.sort()
        for a, b in zip(xs[0::2], xs[1::2]):
            ix0 = max(0, math.ceil((a - ox) / pitch))
            ix1 = min(nx - 1, math.floor((b - ox) / pitch))
            if ix1 >= ix0:
                mask[iy, ix0:ix1 + 1] = value
    return mask


def label(mask, eight):
    """Connected components of `mask` by runs and union-find."""
    ny, nx = mask.shape
    parent = []

    def find(a):
        while parent[a] != a:
            parent[a] = parent[parent[a]]
            a = parent[a]
        return a

    runs_by_row = []
    for y in range(ny):
        row = mask[y]
        padded = np.concatenate([[False], row, [False]])
        changes = np.flatnonzero(padded[1:] != padded[:-1])
        runs = []
        for start, end in zip(changes[0::2], changes[1::2]):
            runs.append([start, end - 1, len(parent)])
            parent.append(len(parent))
        runs_by_row.append(runs)
    for y in range(1, ny):
        above = runs_by_row[y - 1]
        j = 0
        slack = 1 if eight else 0
        for run in runs_by_row[y]:
            while j < len(above) and above[j][1] < run[0] - slack:
                j += 1
            k = j
            while k < len(above) and above[k][0] <= run[1] + slack:
                a, b = find(run[2]), find(above[k][2])
                if a != b:
                    parent[a] = b
                k += 1
    labels = np.zeros((ny, nx), dtype=np.int32)
    roots = {}
    for y, runs in enumerate(runs_by_row):
        for start, end, index in runs:
            root = find(index)
            labels[y, start:end + 1] = roots.setdefault(root, len(roots) + 1)
    return labels, len(roots)


def dilate(mask, radius_cells):
    """Nodes within `radius_cells` (inclusive) of a marked node."""
    result = mask.copy()
    r = int(math.floor(radius_cells))
    ny, nx = mask.shape
    for dy in range(-r, r + 1):
        for dx in range(-r, r + 1):
            if dx * dx + dy * dy > radius_cells * radius_cells or (dx == 0 and dy == 0):
                continue
            src = mask[max(0, -dy):ny - max(0, dy), max(0, -dx):nx - max(0, dx)]
            result[max(0, dy):ny - max(0, -dy), max(0, dx):nx - max(0, -dx)] |= src
    return result


def fill(board, zones, keep_islands):
    for zone in zones:
        if keep_islands:
            zone.SetIslandRemovalMode(pcbnew.ISLAND_REMOVAL_MODE_NEVER)
    filler = pcbnew.ZONE_FILLER(board)
    filler.Fill(pcbnew.ZONES(zones))
    result = {}
    for zone in zones:
        net = str(zone.GetNetname())
        for layer in zone.GetLayerSet().Seq():
            if not pcbnew.IsCopperLayer(layer):
                continue
            polys = pcbnew.SHAPE_POLY_SET(zone.GetFilledPolysList(layer))
            result.setdefault((net, str(board.GetLayerName(layer))), []).append((zone, polys))
    return result


def piece_mapping(ours, ours_count, outlines, outline_count):
    """For each of our pieces, the KiCad outline holding most of its nodes
    (0 for none); per outline, how many of our pieces it holds."""
    flat_ours = ours.ravel()
    flat_kicad = outlines.ravel()
    selected = flat_ours > 0
    pairs = np.stack([flat_ours[selected], flat_kicad[selected]], axis=1)
    counts = {}
    unique, frequency = np.unique(pairs, axis=0, return_counts=True)
    best = {}
    for (piece, outline), count in zip(unique.tolist(), frequency.tolist()):
        if piece not in best or count > best[piece][1] or (count == best[piece][1] and outline > best[piece][0]):
            best[piece] = (outline, count)
    per_outline = np.zeros(outline_count + 1, dtype=int)
    for piece, (outline, _) in best.items():
        per_outline[outline] += 1
    return per_outline


def cause_rasters(board, net, layer, grid, reach, edge_reach):
    """Per kind of foreign object: the nodes within `reach` of one (the
    reach a brush centre keeps), for naming what blocks a node."""
    error = 5000
    sets = {kind: pcbnew.SHAPE_POLY_SET() for kind in ["via", "via hole", "track", "pad", "pad hole", "graphic"]}
    clearance = int(round(reach * NM))
    for track in board.GetTracks():
        if str(track.GetNetname()) == net:
            continue
        if track.Type() == pcbnew.PCB_VIA_T:
            via = pcbnew.Cast_to_PCB_VIA(track) if hasattr(pcbnew, "Cast_to_PCB_VIA") else track
            if via.FlashLayer(layer):
                via.TransformShapeToPolygon(sets["via"], layer, clearance, error, pcbnew.ERROR_OUTSIDE)
            else:
                circle = pcbnew.SHAPE_POLY_SET()
                via.TransformShapeToPolygon(circle, layer, clearance, error, pcbnew.ERROR_OUTSIDE)
                # The drill alone: approximate by the ring's centre and
                # the drill radius plus the reach.
                centre = via.GetPosition()
                radius = via.GetDrillValue() / 2 + clearance
                chain = pcbnew.SHAPE_LINE_CHAIN()
                for k in range(64):
                    angle = 2 * math.pi * k / 64
                    chain.Append(int(centre.x + radius * math.cos(angle)), int(centre.y + radius * math.sin(angle)))
                chain.SetClosed(True)
                sets["via hole"].AddOutline(chain)
        elif track.IsOnLayer(layer):
            track.TransformShapeToPolygon(sets["track"], layer, clearance, error, pcbnew.ERROR_OUTSIDE)
    for footprint in board.GetFootprints():
        for pad in footprint.Pads():
            if str(pad.GetNetname()) == net:
                continue
            if pad.IsOnLayer(layer) and pad.FlashLayer(layer):
                pad.TransformShapeToPolygon(sets["pad"], layer, clearance, error, pcbnew.ERROR_OUTSIDE)
            if pad.HasHole():
                pad.TransformHoleToPolygon(sets["pad hole"], clearance, error, pcbnew.ERROR_OUTSIDE)
        for item in footprint.GraphicalItems():
            if item.IsOnLayer(layer):
                try:
                    item.TransformShapeToPolygon(sets["graphic"], layer, clearance, error, pcbnew.ERROR_OUTSIDE)
                except Exception:
                    pass
    for item in board.GetDrawings():
        if item.IsOnLayer(layer):
            try:
                item.TransformShapeToPolygon(sets["graphic"], layer, clearance, error, pcbnew.ERROR_OUTSIDE)
            except Exception:
                pass
    rasters = {}
    for kind, polys in sets.items():
        mask = np.zeros((grid[1], grid[0]), dtype=bool)
        for k in range(polys.OutlineCount()):
            chain = polys.Outline(k)
            points = np.array([[chain.CPoint(i).x, chain.CPoint(i).y] for i in range(chain.PointCount())], dtype=float) / NM
            if len(points) >= 3:
                # Union: each outline on its own (they overlap).
                mask |= rasterize([points], grid)
        rasters[kind] = mask
    outline = pcbnew.SHAPE_POLY_SET()
    board.GetBoardPolygonOutlines(outline, False)
    outline.Deflate(int(round(edge_reach * NM)), pcbnew.CORNER_STRATEGY_ROUND_ALL_CORNERS, error)
    inside = np.zeros((grid[1], grid[0]), dtype=bool)
    for rings in rings_of(outline):
        rasterize(rings, grid, inside)
    rasters["board edge"] = ~inside
    return rasters


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("board")
    parser.add_argument("dump")
    parser.add_argument("--json")
    parser.add_argument("--causes", action="store_true", help="name the kind of object behind each node our static map blocks")
    parser.add_argument("--slack", type=float, default=0.05, help="mm added to KiCad's reach when naming causes")
    parser.add_argument("--save-filled", help="write the board with KiCad's fill (islands removed) here")
    args = parser.parse_args()

    board = pcbnew.LoadBoard(args.board)
    zones = [zone for zone in board.Zones() if not zone.GetIsRuleArea() and zone.GetNetCode() > 0]
    removed = fill(board, zones, keep_islands=False)
    if args.save_filled:
        board.Save(args.save_filled)
    removal_modes = {zone.m_Uuid.AsString(): zone.GetIslandRemovalMode() for zone in zones}
    kept = fill(board, zones, keep_islands=True)

    # The router numbers the copper layers in stack order (front, inner,
    # back); `dump-pour-map` writes their names, a run's dump does not.
    names_path = os.path.join(args.dump, "layers.json")
    if os.path.exists(names_path):
        layer_names = json.load(open(names_path))
    else:
        layer_names = [str(board.GetLayerName(layer)) for layer in board.GetEnabledLayers().CuStack()]
    report = []
    for name in sorted(os.listdir(args.dump)):
        if not name.endswith(".pour"):
            continue
        header, flags, our_labels = load_dump(os.path.join(args.dump, name))
        net, layer = header["net"], header["layer"]
        layer_name = layer_names[layer]
        grid = (header["nx"], header["ny"], header["origin"][0], header["origin"][1], header["pitch"])
        pitch = header["pitch"]
        key = (net.lstrip("/"), layer_name)
        if key not in kept:
            print(f"{net} {layer_name}: no KiCad zone", file=sys.stderr)
            continue
        zone = kept[key][0][0]
        min_width = zone.GetMinThickness() / NM
        # KiCad's fill with islands kept: outlines and their nodes.
        outlines = np.zeros((header["ny"], header["nx"]), dtype=np.int32)
        outline_count = 0
        brush_centres = np.zeros((header["ny"], header["nx"]), dtype=bool)
        for _, polys in kept[key]:
            for rings in rings_of(polys):
                outline_count += 1
                rasterize(rings, grid, outlines, outline_count)
            deflated = pcbnew.SHAPE_POLY_SET(polys)
            deflated.Deflate(int(round(min_width / 2 * NM)) - 1000, pcbnew.CORNER_STRATEGY_ROUND_ALL_CORNERS, 5000)
            for rings in rings_of(deflated):
                rasterize(rings, grid, brush_centres)
        kicad_removed = sum(polys.OutlineCount() for _, polys in removed.get(key, []))
        kicad_filled = outlines > 0
        ours = (flags & 16) != 0
        # Pieces: ours as labelled by the router (4-connected), ours
        # 8-connected, KiCad's brush centres on the lattice 4- and
        # 8-connected, KiCad's outlines.
        our_layer_pieces = len(np.unique(our_labels[our_labels > 0]))
        _, ours8 = label(ours, True)
        _, centres4 = label(brush_centres, False)
        _, centres8 = label(brush_centres, True)
        ours_dilated = dilate(ours, min_width / 2 / pitch + 1e-6)
        # Node classes: KiCad's brush fits where ours is not free, and the
        # first reason ours is not.
        missing = brush_centres & ~ours
        extra = ours & ~brush_centres
        reasons = {}
        polygon = (flags & 1) != 0
        static = (flags & 2) != 0
        occupancy = (flags & 4) != 0
        ring = (flags & 8) != 0
        reason_masks = [
            ("outside polygon", missing & ~polygon),
            ("thermal ring", missing & polygon & ring),
            ("static map", missing & polygon & ~ring & ~static),
            ("routed copper", missing & polygon & ~ring & static & ~occupancy),
        ]
        accounted = np.zeros_like(missing)
        for reason, mask in reason_masks:
            reasons[reason] = int(mask.sum())
            accounted |= mask
        reasons["other (rim)"] = int((missing & ~accounted).sum())
        if args.causes:
            static_missing = missing & polygon & ~ring & ~static
            reach = zone.GetLocalClearance() / NM if hasattr(zone, "GetLocalClearance") and zone.GetLocalClearance() is not None else 0.0
            reach = max(reach, 0.0) + min_width / 2 + args.slack
            rasters = cause_rasters(board, net if net in [str(z.GetNetname()) for z in zones] else net.lstrip("/"), board.GetLayerID(layer_name), grid, reach, 0.075 + min_width / 2 + args.slack)
            left = static_missing.copy()
            causes = {}
            for kind in ["via", "via hole", "track", "pad", "pad hole", "graphic", "board edge"]:
                hit = left & rasters[kind]
                causes[kind] = int(hit.sum())
                left &= ~hit
            causes["unexplained"] = int(left.sum())
            reasons["static map by object"] = causes
            reasons["cause reach"] = reach
        # Our pieces against KiCad's outlines.
        per_outline = piece_mapping(our_labels.astype(np.int64), our_layer_pieces, outlines, outline_count)
        ours8_labels, _ = label(ours, True)
        per_outline8 = piece_mapping(ours8_labels.astype(np.int64), ours8, outlines, outline_count)
        centres4_labels, _ = label(brush_centres, False)
        per_outline_c4 = piece_mapping(centres4_labels.astype(np.int64), centres4, outlines, outline_count)
        def spanning(pieces):
            selected = (pieces > 0) & (outlines > 0)
            pairs = np.unique(np.stack([pieces[selected], outlines[selected]], axis=1), axis=0)
            per_piece = np.bincount(pairs[:, 0])
            return int((per_piece > 1).sum())
        # The splits that KiCad's thermal spokes bridge: our pieces joined
        # when KiCad's brush centres inside our thermal rings (its spokes,
        # drawn from the pad's centre across the pad) count as free.
        bridged, _ = label(ours | (brush_centres & ring), True)
        per_outline_bridged = piece_mapping(bridged.astype(np.int64), 0, outlines, outline_count)
        # Our pieces joined that way, counted per KiCad outline.
        merged_pieces = 0
        for piece_label in np.unique(bridged[bridged > 0]):
            members = np.unique(our_labels[(bridged == piece_label) & (our_labels > 0)])
            if len(members) > 1:
                merged_pieces += len(members) - 1
        row = {
            "net": net,
            "layer": layer_name,
            "pitch": pitch,
            "min_width": min_width,
            "kicad_outlines_islands_kept": outline_count,
            "kicad_outlines": kicad_removed,
            "our_pieces": our_layer_pieces,
            "our_pieces_8": ours8,
            "kicad_centre_pieces_4": centres4,
            "kicad_centre_pieces_8": centres8,
            "our_pieces_outside_kicad_fill": int(per_outline[0]),
            "outlines_holding_more_than_one_of_our_pieces": int((per_outline[1:] > 1).sum()),
            "our_pieces_in_shared_outlines": int(per_outline[1:][per_outline[1:] > 1].sum()),
            "largest_outline_piece_count": int(per_outline[1:].max()) if outline_count else 0,
            "outlines_with_more_than_one_8": int((per_outline8[1:] > 1).sum()),
            "our_pieces_8_in_shared_outlines": int(per_outline8[1:][per_outline8[1:] > 1].sum()),
            "kicad_centre_pieces_4_in_shared_outlines": int(per_outline_c4[1:][per_outline_c4[1:] > 1].sum()),
            "our_pieces_spanning_outlines": spanning(our_labels.astype(np.int64)),
            "our_pieces_joined_by_spokes": merged_pieces,
            "pieces_with_spokes_in_shared_outlines": int(per_outline_bridged[1:][per_outline_bridged[1:] > 1].sum()),
            "our_pieces_8_spanning_outlines": spanning(ours8_labels.astype(np.int64)),
            "kicad_fill_nodes": int(kicad_filled.sum()),
            "kicad_brush_centres": int(brush_centres.sum()),
            "our_free": int(ours.sum()),
            "centres_not_ours": int(missing.sum()),
            "centres_not_ours_by_reason": reasons,
            "ours_not_centres": int(extra.sum()),
            "ours_not_centres_outside_fill": int((ours & ~kicad_filled).sum()),
            "fill_not_ours_dilated": int((kicad_filled & ~ours_dilated).sum()),
            "ours_dilated_not_fill": int((ours_dilated & ~kicad_filled).sum()),
        }
        report.append(row)
        print(json.dumps(row))
        np.savez_compressed(
            os.path.join(args.dump, name.replace(".pour", ".kicad.npz")),
            outlines=outlines,
            brush_centres=brush_centres,
        )
    if args.json:
        with open(args.json, "w") as handle:
            json.dump(report, handle, indent=1)


if __name__ == "__main__":
    main()
