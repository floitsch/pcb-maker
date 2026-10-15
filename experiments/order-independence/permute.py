#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Copies a KiCad project with its board's footprints in another order,
and each footprint's pads and graphics (fp_*) too: the same board as
another save, import or schematic flow would write it.

    permute.py <project dir> <board id> <out dir> asis|reversed|shuffled [--seed N]

Only the order changes: every footprint, pad and graphic keeps its text,
and each kind keeps the places it held (the text between them stays).
"""
import argparse
import random
import shutil
from pathlib import Path

GRAPHICS = ("fp_line", "fp_rect", "fp_arc", "fp_circle", "fp_poly", "fp_curve", "fp_text", "fp_text_box")


def child_spans(text, start, end):
    """The (start, end) spans of the lists directly inside the list text[start:end]."""
    spans = []
    depth = 0
    index = start + 1
    in_string = False
    begin = None
    while index < end - 1:
        char = text[index]
        if in_string:
            if char == "\\":
                index += 1
            elif char == '"':
                in_string = False
        elif char == '"':
            in_string = True
        elif char == "(":
            if depth == 0:
                begin = index
            depth += 1
        elif char == ")":
            depth -= 1
            if depth == 0:
                spans.append((begin, index + 1))
        index += 1
    return spans


def head(text, span):
    word = text[span[0] + 1:span[1]].split(None, 1)[0]
    return word.rstrip(")")


def reorder(text, start, end, spans, kind, order):
    """text[start:end] with the spans for which kind(span) holds put in order(list)."""
    chosen = [span for span in spans if kind(span)]
    pieces = order([text[a:b] for a, b in chosen])
    result = []
    position = start
    replacement = iter(pieces)
    for span in spans:
        if kind(span):
            result.append(text[position:span[0]])
            result.append(next(replacement))
            position = span[1]
    result.append(text[position:end])
    return "".join(result)


def permute(text, mode, seed):
    rng = random.Random(seed)

    def order(items):
        items = list(items)
        if mode == "reversed":
            items.reverse()
        elif mode == "shuffled":
            rng.shuffle(items)
        return items

    root_start = text.index("(")
    root_end = text.rindex(")") + 1
    spans = child_spans(text, root_start, root_end)
    footprints = []
    for span in spans:
        if head(text, span) != "footprint":
            continue
        inner = child_spans(text, span[0], span[1])
        body = reorder(text, span[0], span[1], inner, lambda s: head(text, s) == "pad", order)
        inner = child_spans(body, 0, len(body))
        body = reorder(body, 0, len(body), inner, lambda s: head(body, s) in GRAPHICS, order)
        footprints.append(body)
    replaced = iter(order(footprints))
    result = [text[:root_start]]
    position = root_start
    for span in spans:
        if head(text, span) == "footprint":
            result.append(text[position:span[0]])
            result.append(next(replaced))
            position = span[1]
    result.append(text[position:])
    return "".join(result)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("project")
    parser.add_argument("board")
    parser.add_argument("out")
    parser.add_argument("mode", choices=["asis", "reversed", "shuffled"])
    parser.add_argument("--seed", type=int, default=1)
    arguments = parser.parse_args()
    out = Path(arguments.out)
    shutil.copytree(arguments.project, out, symlinks=True)
    board = out / f"{arguments.board}.kicad_pcb"
    text = board.read_text()
    permuted = permute(text, arguments.mode, arguments.seed)
    assert sorted(permuted) == sorted(text), "a permutation keeps every character"
    board.write_text(permuted)


if __name__ == "__main__":
    main()
