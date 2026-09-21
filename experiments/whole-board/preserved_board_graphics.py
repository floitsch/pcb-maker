# Copyright (C) 2026 Toit contributors.
"""Recognize explicitly nonphysical, silk-only native board artwork.

Unknown footprint content is not evidence of an empty physical envelope. These
footprints remain native objects and must be preserved, never packed as bodies.
"""
from decimal import Decimal, InvalidOperation
from pathlib import Path
from audit_dsn_classes import parse_sexpr, children

ROLE = 'preserved_board_graphic'
ARTWORK = {'fp_line', 'fp_rect', 'fp_circle', 'fp_arc', 'fp_poly', 'fp_text', 'fp_text_box'}
METADATA = {'layer', 'uuid', 'at', 'property', 'attr', 'embedded_fonts', 'descr', 'tags',
            'path', 'sheetname', 'sheetfile'}


def canonical(value, numeric=False):
    if isinstance(value, list):
        # Numeric-looking user text is artwork too: preserve it literally.
        coordinates = value and value[0] in {
            'at', 'xy', 'xyz', 'size', 'width', 'thickness', 'offset', 'scale',
            'rotate', 'start', 'end', 'mid', 'center', 'angle', 'radius'}
        return [canonical(v, bool(coordinates and i > 0)) for i, v in enumerate(value)]
    if numeric:
        try:
            number = Decimal(value)
            if number.is_finite():
                return format(number.normalize(), 'f')
        except InvalidOperation:
            pass
    return value


def is_board_graphic(fp):
    attrs = children(fp, 'attr')
    if len(attrs) != 1 or 'board_only' not in attrs[0][1:]:
        return False
    artwork = []
    for item in fp[2:]:
        if not isinstance(item, list):
            if item != 'locked':
                return False
            continue
        kind = item[0]
        if item == ['duplicate_pad_numbers_are_jumpers', 'no']:
            continue  # Native writer emits this default even on zero-pad graphics.
        if kind not in ARTWORK | METADATA:
            return False  # Includes pads/drills, models, zones and unknown geometry.
        if kind in ARTWORK or kind == 'property':
            layer = children(item, 'layer')
            hidden = any(q[0] == 'hide' and q[1:] == ['yes'] for q in item if isinstance(q, list))
            # Invisible property placeholders are metadata, not physical Fab artwork.
            if kind == 'property' and hidden and len(layer) == 1 and layer[0][1] in ('F.Fab', 'B.Fab', 'F.SilkS', 'B.SilkS'):
                continue
            if len(layer) != 1 or layer[0][1] not in ('F.SilkS', 'B.SilkS'):
                return False
            artwork.append(item)
    return bool(artwork)


def preserved_graphics(path):
    board = parse_sexpr(Path(path).read_text())
    assert board[0] == 'kicad_pcb'
    result = {}
    for fp in children(board, 'footprint'):
        if not is_board_graphic(fp):
            continue
        refs = [p[2] for p in children(fp, 'property') if p[1] == 'Reference']
        assert len(refs) == 1 and refs[0] not in result
        result[refs[0]] = canonical([item for item in fp
            if item != ['duplicate_pad_numbers_are_jumpers', 'no']])
    return result


def partition(board, mapping, poses):
    """Independently validate an exhaustive native/physical/graphic partition."""
    graphics = preserved_graphics(board.GetFileName())
    native = [str(fp.GetReference()) for fp in board.GetFootprints()]
    pose_refs = [p['component'] for p in poses]
    assert len(native) == len(set(native)) and len(pose_refs) == len(set(pose_refs))
    declared = {ref for ref, row in mapping.items() if row.get('placement_role') == ROLE}
    assert declared == set(graphics), 'Preserved graphic roles do not match native evidence'
    assert set(mapping) == set(native), 'Native mapping inventory mismatch'
    assert set(pose_refs) == set(native) - declared, 'Physical pose inventory mismatch'
    return graphics
