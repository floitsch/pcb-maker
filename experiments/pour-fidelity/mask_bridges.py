# Copyright (C) 2026 Toit contributors.
"""Solder mask apertures that expose two nets, and whether a pour is one.

    python3 -I mask_bridges.py <board.kicad_pcb>...

KiCad's bridge rule for a simple aperture (a mask graphic or a mask-only
pad: DRC_TEST_PROVIDER_SOLDER_MASK, KiCad 10): every copper item of a net
within the board's solder-mask-to-copper clearance plus the aperture's mask
expansion (plus the item's own, for a pad or an untented via) is exposed,
items of the aperture's own footprint aside when the board allows bridges
in footprints; two nets exposed is a bridge. Per board, after a refill:
each aperture that exposes two nets, the nets by kind (fixed pads, routed
tracks and vias, zone fill), and the verdict:

- tracks: only routed tracks or vias bring the second net (the router's
  clearance to the opening decides);
- pour: a zone's fill brings a net no fixed pad under the aperture has (the
  fill must be kept out of the opening);
- fixed: the pads alone expose two nets (the designer's footprint).

With `--pours`, also every aperture over a pour of a net other than its
own (the fill could reach in), routed or not.
"""

import argparse
import json
import os
import sys

import pcbnew

NM = 1.0e6


def mask_to_copper(board):
    project = os.path.splitext(board.GetFileName())[0] + ".kicad_pro"
    try:
        rules = json.load(open(project))["board"]["design_settings"]["rules"]
        return int(round(rules.get("solder_mask_to_copper_clearance", 0.0) * NM))
    except (OSError, KeyError, ValueError):
        return 0


def apertures(board):
    """(item, mask layer, copper layer) of every simple aperture."""
    result = []
    sides = [(pcbnew.F_Mask, pcbnew.F_Cu), (pcbnew.B_Mask, pcbnew.B_Cu)]
    for footprint in board.GetFootprints():
        for item in footprint.GraphicalItems():
            for mask, copper in sides:
                if item.GetLayer() == mask and isinstance(item, (pcbnew.PCB_SHAPE, pcbnew.PCB_TEXT)):
                    result.append((item, mask, copper))
        for pad in footprint.Pads():
            layers = pad.GetLayerSet()
            if not any(pcbnew.IsCopperLayer(layer) for layer in layers.Seq()):
                for mask, copper in sides:
                    if layers.Contains(mask):
                        result.append((pad, mask, copper))
    for item in board.GetDrawings():
        for mask, copper in sides:
            if item.GetLayer() == mask and isinstance(item, (pcbnew.PCB_SHAPE, pcbnew.PCB_TEXT)):
                result.append((item, mask, copper))
    return result


def expansion(item, layer):
    if isinstance(item, pcbnew.PAD):
        return item.GetSolderMaskExpansion(layer)
    if isinstance(item, pcbnew.PCB_VIA):
        return 0 if item.IsTented(layer) else item.GetSolderMaskExpansion()
    if isinstance(item, pcbnew.PCB_SHAPE) and not isinstance(item, pcbnew.PCB_TEXT):
        return item.GetSolderMaskExpansion()
    return 0


def name(item):
    if isinstance(item, pcbnew.PAD):
        return f"pad {item.GetParentFootprint().GetReference()}.{item.GetNumber()}"
    footprint = item.GetParentFootprint()
    return f"{item.GetClass()}{' of ' + footprint.GetReference() if footprint else ''}"


def survey(path, pours_too):
    board = pcbnew.LoadBoard(path)
    zones = [zone for zone in board.Zones() if not zone.GetIsRuleArea()]
    pcbnew.ZONE_FILLER(board).Fill(pcbnew.ZONES(zones))
    allow_in_footprints = board.GetDesignSettings().m_AllowSoldermaskBridgesInFPs
    base = mask_to_copper(board)
    copper_items = list(board.GetTracks()) + [pad for footprint in board.GetFootprints() for pad in footprint.Pads()]
    rows = []
    for aperture, mask, copper in apertures(board):
        footprint = aperture.GetParentFootprint()
        if footprint is not None and footprint.AllowSolderMaskBridges():
            continue
        shape = aperture.GetEffectiveShape(mask)
        reach = base + expansion(aperture, mask)
        box = aperture.GetBoundingBox()
        box.Inflate(reach + int(1 * NM))
        exposed = {}
        for item in copper_items:
            if item.GetNetCode() <= 0 or not item.IsOnLayer(copper) or not box.Intersects(item.GetBoundingBox()):
                continue
            if allow_in_footprints and footprint is not None and item.GetParentFootprint() == footprint:
                continue
            if isinstance(item, pcbnew.PAD) and not item.FlashLayer(copper):
                continue
            if item.GetEffectiveShape(copper).Collide(shape, reach + expansion(item, mask)):
                kind = "pad" if isinstance(item, pcbnew.PAD) else "routed"
                exposed.setdefault(str(item.GetNetname()), set()).add(kind)
        over = []
        for zone in zones:
            if zone.GetNetCode() <= 0 or not zone.IsOnLayer(copper):
                continue
            fill = zone.GetFilledPolysList(copper)
            if fill.OutlineCount() and fill.Collide(shape, reach):
                exposed.setdefault(str(zone.GetNetname()), set()).add("zone")
            elif pours_too and zone.GetBoundingBox().Intersects(box):
                outline = zone.Outline()
                if outline.Collide(shape, reach):
                    over.append(str(zone.GetNetname()))
        nets = sorted(exposed)
        if len(nets) >= 2:
            pad_nets = [net for net in nets if "pad" in exposed[net]]
            zone_only = [net for net in nets if exposed[net] == {"zone"}]
            routed_only = [net for net in nets if exposed[net] == {"routed"}]
            if len(pad_nets) >= 2:
                verdict = "fixed"
            elif zone_only and (pad_nets or len(zone_only) >= 2 or routed_only == []):
                verdict = "pour"
            else:
                verdict = "tracks"
            rows.append({"aperture": name(aperture), "layer": str(board.GetLayerName(mask)), "reach": reach / NM,
                         "nets": {net: sorted(kinds) for net, kinds in exposed.items()}, "verdict": verdict})
        elif pours_too:
            own = nets[0] if nets else None
            others = sorted(set(net for net in over if net != own))
            if others:
                rows.append({"aperture": name(aperture), "layer": str(board.GetLayerName(mask)), "reach": reach / NM,
                             "nets": {net: sorted(kinds) for net, kinds in exposed.items()}, "pour_outlines": others, "verdict": "pour outline only"})
    return rows


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("boards", nargs="+")
    parser.add_argument("--pours", action="store_true")
    args = parser.parse_args()
    for path in args.boards:
        try:
            rows = survey(path, args.pours)
        except Exception as error:  # noqa: BLE001
            print(json.dumps({"board": path, "error": str(error)}))
            continue
        print(json.dumps({"board": path, "apertures": rows}))
        sys.stdout.flush()


if __name__ == "__main__":
    main()
