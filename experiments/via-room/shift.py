# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.

"""Copies a KiCad project with one footprint moved (docs/placer.md, "A
far-side pad under fine pitch, and what it was (2026-11-05)": U10 moved
by 0.02 to 0.05 mm onto the router's lattice).

    python3 shift.py <project dir> <out dir> <board stem> <ref> <dx mm> <dy mm>
"""
import os
import shutil
import sys

import pcbnew

src, dst, stem, ref, dx, dy = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], float(sys.argv[5]), float(sys.argv[6])
os.makedirs(dst, exist_ok=True)
for name in os.listdir(src):
    if name.startswith(stem + '.') and not name.endswith('.kicad_pcb'):
        shutil.copy(os.path.join(src, name), dst)
board = pcbnew.LoadBoard(os.path.join(src, stem + '.kicad_pcb'))
part = board.FindFootprintByReference(ref)
part.Move(pcbnew.VECTOR2I(pcbnew.FromMM(dx), pcbnew.FromMM(dy)))
board.Save(os.path.join(dst, stem + '.kicad_pcb'))
print(ref, 'at', pcbnew.ToMM(part.GetPosition().x), pcbnew.ToMM(part.GetPosition().y))
