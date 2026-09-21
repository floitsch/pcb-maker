#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Read-only native copper islands and bounded existing-pad bridge suggestions.

Usage: inspect_native_islands.py BOARD OUTPUT.json [--maximum-pairs N]
Requires pcbnew and the Python standard library. OUTPUT.svg is always rendered.
No DRC report is read, no net is relabelled, and no source geometry is removed.
"""
import argparse
from collections import Counter, defaultdict
import hashlib
import heapq
import json
import math
from pathlib import Path
import sys
import xml.etree.ElementTree as ET


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def normalize_net(name):
    return name.removeprefix('/')


def pair_suggestions(nets, maximum_pairs):
    """Stable nearest-center ordering; integer native units avoid distance ties."""
    considered = 0

    def candidates():
        nonlocal considered
        for net in nets:
            islands = net['islands']
            for i, left in enumerate(islands):
                for right in islands[i + 1:]:
                    for a in left['pads']:
                        if not a['endpoint_eligible']:
                            continue
                        for b in right['pads']:
                            if not b['endpoint_eligible']:
                                continue
                            first, second = sorted([a, b], key=lambda p: (p['footprint'], p['pad'], p['uuid']))
                            distance_squared = sum((x - y) ** 2 for x, y in zip(a['at_nm'], b['at_nm']))
                            order = (distance_squared, net['connection'], first['footprint'], first['pad'],
                                     second['footprint'], second['pad'], first['uuid'], second['uuid'])
                            proposal = dict(connection=net['connection'],
                                start={k: first[k] for k in ['footprint', 'pad']},
                                finish={k: second[k] for k in ['footprint', 'pad']},
                                start_pad_uuid=first['uuid'], finish_pad_uuid=second['uuid'],
                                island_ids=sorted([left['id'], right['id']]),
                                distance_mm=math.sqrt(distance_squared) / 1e6)
                            considered += 1
                            yield order, proposal

    selected = heapq.nsmallest(maximum_pairs, candidates(), key=lambda row: row[0])
    return [dict(rank=i, **row[1]) for i, row in enumerate(selected)], considered


def inspect(board_path, maximum_pairs=8):
    import pcbnew
    if not hasattr(pcbnew.SwigPyIterator, 'next'):
        pcbnew.SwigPyIterator.next = pcbnew.SwigPyIterator.__next__
    board_path = Path(board_path).resolve()
    before = digest(board_path)
    board = pcbnew.LoadBoard(str(board_path))
    if board is None:
        raise ValueError('native board load failed')
    all_pads = list(board.GetPads())
    all_tracks = list(board.GetTracks())
    uid = lambda item: item.m_Uuid.AsString()
    copper_layers = lambda item: [pcbnew.LayerName(layer) for layer in item.GetLayerSet().CuStack()]
    raw_items = all_pads + all_tracks
    if len({uid(item) for item in raw_items}) != len(raw_items):
        raise ValueError('duplicate native copper item UUIDs')
    ignored = []
    items = {}
    for item in raw_items:
        if item.GetNetCode() <= 0 or not item.GetNetname() or not copper_layers(item):
            ignored.append(dict(uuid=uid(item), kind=item.GetClass(),
                reason='unassigned net or no copper layer; not a named electrical connection'))
        else:
            items[uid(item)] = item
    locator_counts = Counter((p.GetParentFootprint().GetReference(), p.GetNumber()) for p in all_pads)
    unsupported = []
    discovery_complete = True
    for zone in board.Zones():
        if not zone.GetIsRuleArea() and copper_layers(zone):
            unsupported.append(dict(kind='copper_zone_islands', connection=normalize_net(zone.GetNetname()),
                item_uuids=[uid(zone)], reason='One zone UUID may contain disconnected filled islands; per-zone island lowering is not implemented.'))
            discovery_complete = False
    if not board.BuildConnectivity():
        raise ValueError('native connectivity build failed')
    connectivity = board.GetConnectivity()
    parent = {key: key for key in items}

    def find(key):
        while parent[key] != key:
            parent[key] = parent[parent[key]]
            key = parent[key]
        return key

    def join(a, b):
        x, y = sorted([find(a), find(b)])
        parent[y] = x

    unknown = set()
    cross_net = set()
    for key in sorted(items):
        item = items[key]
        for other in connectivity.GetConnectedItems(item):
            other_key = uid(other)
            if other_key not in items:
                if other.GetNetCode() > 0:
                    unknown.add(other_key)
                continue
            if other.GetNetCode() != item.GetNetCode():
                cross_net.add(tuple(sorted([key, other_key])))
                continue
            join(key, other_key)
    if unknown:
        discovery_complete = False
        unsupported.append(dict(kind='unmodeled_native_connected_items', item_uuids=sorted(unknown),
            reason='Native connectivity references copper outside the pad/track/via inventory.'))
    if cross_net:
        discovery_complete = False
        unsupported.append(dict(kind='cross_net_physical_contacts', item_uuids=sorted({u for pair in cross_net for u in pair}),
            reason='Native connectivity links distinct assigned net codes; this tool does not resolve shorts.'))
    grouped = defaultdict(lambda: defaultdict(list))
    for key, item in items.items():
        grouped[item.GetNetCode()][find(key)].append(key)
    canonical_codes = defaultdict(set)
    nets = []
    for code, groups in grouped.items():
        sample = items[next(iter(next(iter(groups.values()))))]
        raw_name = sample.GetNetname()
        name = normalize_net(raw_name)
        canonical_codes[name].add(code)
        islands = []
        for keys in sorted(groups.values(), key=lambda v: sorted(v)):
            keys.sort()
            island_id = 'island-' + hashlib.sha256('\0'.join(keys).encode()).hexdigest()
            pads = []
            for key in keys:
                item = items[key]
                if not isinstance(item, pcbnew.PAD):
                    continue
                ref, number = item.GetParentFootprint().GetReference(), item.GetNumber()
                layers = copper_layers(item)
                reason = None
                if not ref or not number:
                    reason = 'empty endpoint locator'
                elif locator_counts[(ref, number)] != 1:
                    reason = 'ambiguous footprint/pad locator'
                elif not any(layer in ['F.Cu', 'B.Cu'] for layer in layers):
                    reason = 'pad has no outer-layer copper for the current route adapter'
                at = item.GetPosition()
                pads.append(dict(uuid=key, footprint_uuid=uid(item.GetParentFootprint()),
                    footprint=ref, pad=number, at_nm=[at.x, at.y], at_mm=[at.x / 1e6, at.y / 1e6],
                    copper_layers=layers, endpoint_eligible=reason is None, endpoint_reason=reason))
            pads.sort(key=lambda p: (p['footprint'], p['pad'], p['uuid']))
            islands.append(dict(id=island_id, item_uuids=keys,
                kind_counts=dict(sorted(Counter(items[key].GetClass() for key in keys).items())),
                pads=pads, pad_count=len(pads), eligible_pad_count=sum(p['endpoint_eligible'] for p in pads)))
        for island in islands:
            if len(islands) > 1 and not island['eligible_pad_count']:
                unsupported.append(dict(kind='no_eligible_pad_in_island', connection=name,
                    island_id=island['id'], item_uuids=island['item_uuids'],
                    reason='Distinct copper island has no unique existing outer-layer pad endpoint.'))
        nets.append(dict(connection=name, native_name=raw_name, net_code=code,
            island_count=len(islands), pad_count=sum(i['pad_count'] for i in islands), islands=islands))
    for name, codes in canonical_codes.items():
        if len(codes) > 1:
            discovery_complete = False
            unsupported.append(dict(kind='ambiguous_canonical_net_name', connection=name, net_codes=sorted(codes),
                reason='Distinct native net names collapse to the route adapter connection identity.'))
    nets.sort(key=lambda n: (n['connection'], n['net_code']))
    proposals, count = pair_suggestions(nets, maximum_pairs) if discovery_complete else ([], 0)
    unchanged = before == digest(board_path)
    if not unchanged:
        raise ValueError('source board changed during read-only discovery')
    has_open = any(n['island_count'] > 1 for n in nets) if discovery_complete else None
    report = dict(schema_version=1, board=str(board_path), board_sha256=before,
        source_unchanged=True, kicad_version=pcbnew.Version(), discovery_complete=discovery_complete,
        complete=discovery_complete and not has_open, has_open_islands=has_open,
        nets=nets, proposals=proposals, unsupported=unsupported, ignored_items=sorted(ignored, key=lambda x: x['uuid']),
        maximum_pairs=maximum_pairs, candidate_pad_pairs_considered=count,
        proposals_truncated=count > maximum_pairs,
        contract='Native connected pad/track/via islands only; no DRC, routing feasibility, or native admission claim. Copper zones are unsupported. Suggestions use existing unique pad identities and nearest centers; source geometry is unchanged.')
    return report, board


def render(board, report, output):
    """Combined copper inspection, no component bodies; native pad polygons."""
    import pcbnew
    ns = 'http://www.w3.org/2000/svg'
    ET.register_namespace('', ns)
    tag = lambda name: '{' + ns + '}' + name
    bbox = board.GetBoardEdgesBoundingBox()
    if bbox.GetWidth() <= 0 or bbox.GetHeight() <= 0:
        bbox = board.GetBoundingBox()
    x, y = bbox.GetX() / 1e6 - 2, bbox.GetY() / 1e6 - 2
    w, h = max(bbox.GetWidth() / 1e6 + 4, 5), max(bbox.GetHeight() / 1e6 + 4, 5)
    root = ET.Element(tag('svg'), viewBox=f'{x} {y} {w} {h}', width='1400', height=str(1400 * h / w))
    ET.SubElement(root, tag('rect'), x=str(x), y=str(y), width=str(w), height=str(h), fill='white')
    colors = {pcbnew.F_Cu: '#bb3544', pcbnew.B_Cu: '#286bc0'}
    def path_polygon(poly):
        chunks = []
        for i in range(poly.OutlineCount()):
            contours = [poly.COutline(i)] + [poly.CHole(i, j) for j in range(poly.HoleCount(i))]
            for contour in contours:
                points = [(contour.CPoint(k).x / 1e6, contour.CPoint(k).y / 1e6) for k in range(contour.PointCount())]
                if points:
                    chunks.append('M ' + ' L '.join(f'{a} {b}' for a, b in points) + ' Z')
        return ' '.join(chunks)
    for layer in [pcbnew.B_Cu, pcbnew.F_Cu]:
        group = ET.SubElement(root, tag('g'), opacity='.7', fill=colors[layer], stroke=colors[layer])
        for item in board.GetTracks():
            if not item.IsOnLayer(layer):
                continue
            if isinstance(item, pcbnew.PCB_VIA):
                p = item.GetPosition()
                ET.SubElement(group, tag('circle'), cx=str(p.x / 1e6), cy=str(p.y / 1e6), r=str(item.GetWidth(layer) / 2e6), stroke='none')
            elif isinstance(item, pcbnew.PCB_ARC):
                poly = pcbnew.SHAPE_POLY_SET()
                item.TransformShapeToPolygon(poly, layer, 0, 20_000, pcbnew.ERROR_INSIDE)
                ET.SubElement(group, tag('path'), d=path_polygon(poly), stroke='none', **{'fill-rule': 'evenodd'})
            else:
                a, b = item.GetStart(), item.GetEnd()
                ET.SubElement(group, tag('line'), x1=str(a.x / 1e6), y1=str(a.y / 1e6), x2=str(b.x / 1e6), y2=str(b.y / 1e6),
                    **{'stroke-width': str(item.GetWidth() / 1e6), 'stroke-linecap': 'round'})
        for pad in board.GetPads():
            if pad.IsOnLayer(layer):
                poly = pcbnew.SHAPE_POLY_SET()
                pad.TransformShapeToPolygon(poly, layer, 0, 20_000, pcbnew.ERROR_INSIDE)
                ET.SubElement(group, tag('path'), d=path_polygon(poly), stroke='none', **{'fill-rule': 'evenodd'})
    palette = ['#8e24aa', '#009688', '#e58b00', '#3445a4']
    for net in report['nets']:
        if net['island_count'] <= 1:
            continue
        for ordinal, island in enumerate(net['islands']):
            color = palette[ordinal % len(palette)]
            for pad in island['pads']:
                px, py = pad['at_mm']
                ET.SubElement(root, tag('circle'), cx=str(px), cy=str(py), r='.7', fill='none', stroke=color, **{'stroke-width': '.12'})
                text = ET.SubElement(root, tag('text'), x=str(px + .6), y=str(py - .6), fill=color,
                    **{'font-size': '.7', 'font-family': 'sans-serif', 'paint-order': 'stroke', 'stroke': 'white', 'stroke-width': '.1'})
                text.text = f"{pad['footprint']}.{pad['pad']} I{ordinal + 1}"
    pads = {p['uuid']: p for n in report['nets'] for i in n['islands'] for p in i['pads']}
    for proposal in report['proposals']:
        a, b = pads[proposal['start_pad_uuid']]['at_mm'], pads[proposal['finish_pad_uuid']]['at_mm']
        ET.SubElement(root, tag('line'), x1=str(a[0]), y1=str(a[1]), x2=str(b[0]), y2=str(b[1]), stroke='#171717',
            **{'stroke-width': '.12', 'stroke-dasharray': '.5 .4'})
    text = ET.SubElement(root, tag('text'), x=str(x + .5), y=str(y + 1), fill='#111', **{'font-size': '.9', 'font-family': 'sans-serif'})
    text.text = 'Native copper islands; dashed = suggested pad pairs, not routes. No DRC/admission claim.'
    ET.ElementTree(root).write(output, encoding='unicode')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('board', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--maximum-pairs', type=int, default=8)
    args = parser.parse_args()
    if args.maximum_pairs <= 0:
        parser.error('--maximum-pairs must be positive')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    if args.output.resolve() == args.board.resolve() or args.output.with_suffix('.svg').resolve() == args.board.resolve():
        parser.error('output must not overwrite the source board')
    report, board = inspect(args.board, args.maximum_pairs)
    render(board, report, args.output.with_suffix('.svg'))
    report['source_unchanged'] = digest(args.board) == report['board_sha256']
    if not report['source_unchanged']:
        raise ValueError('source changed before report publication')
    report['render'] = str(args.output.with_suffix('.svg').resolve())
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: report[k] for k in ['discovery_complete', 'complete', 'has_open_islands', 'source_unchanged']}))


if __name__ == '__main__':
    main()
