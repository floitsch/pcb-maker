# Native board improvement task

Find useful routing improvements in the supplied fixed-placement board. There
may be zero or more opportunities; no particular defect or count is promised.
Minimize copper length in mm plus 2 mm per via, without reducing widths or
changing connectivity, pads, other fixed geometry, or rules. Vias pass through
both layers. Through-hole pads may connect on both exported copper layers.

The exact native geometry packet preserves circle/rect/oval pads and their
rotation, holes, widths, layers, net classes, and board rules. The native KiCad
board/project remains the acceptance authority. All initial proposals will be
checked after submission. Do not run a validator in the initial phase.

Submit a JSON object with diagnosis, confidence, missing_evidence, and one
simultaneous proposal using `remove_tracks`, `remove_vias`, and `add_tracks`.
Every added track needs exact `start`, `end`, `width`, `layer`, and `net`, using
the names/IDs in the geometry packet. Empty lists mean abstention. Pick the
strongest supported improvement; combining compatible edits is permitted.

If your assigned arm permits tools, run:

`python3 benchmarks/analysis-lab/native_inspect.py BOARD COMMAND ...`

- `index`: net IDs, length, pad IDs, via positions and IDs.
- `objects --ids V1 T2 P3`: exact named geometry.
- `region --bounds X0 Y0 X1 Y1`: local tracks/vias/pads, retaining whole objects.
- `neighborhood --ids V1 --radius 3`: local context around named objects.
- `net --net N1`: all selected-net objects plus nearby foreign context.
- `render --net N1 --bounds X0 Y0 X1 Y1 --output FILE.png`: labeled local view.

Index length is stored segment length, useful for navigation only. Final score
counts same-net/layer collinear centerline union once. Effective rules include
KiCad defaults even when the project file does not declare them explicitly.

Do not read tool source, original board history, private fixtures, prior agent
outputs, or parent experiment notes. No subagents. Record start/end UTC,
inspection commands, and evidence gaps. Send short observable progress updates
to the parent. Filesystem isolation is procedural, not enforced.
