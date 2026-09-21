# Interactive inspection interface

Run `python3 benchmarks/analysis-lab/inspect_board.py BOARD COMMAND ...`.
Do not read its source or any generator/private files. These commands use only
the provided board geometry; no answer keys or target localization are consulted.

| Command | Evidence |
| --- | --- |
| `summary` | Per-route lengths, endpoint distance, points, layers, transitions |
| `anchors` | Exact terminal coordinates, pad layers, rules, route endpoints |
| `net --net N` | Selected net plus foreign route and obstacle context |
| `region --bounds X0 Y0 X1 Y1` | Local geometry; intersecting routes remain whole |
| `render --output FILE.png` | Both layers, aligned and labeled |
| `render --net N --bounds X0 Y0 X1 Y1 --output FILE.png` | Selected-net crop |
| `render --overlay --output FILE.png` | Both layers on the same diagram |
| `probe --from X Y --to X Y --layer top --net N` | Named blockers and closest clearance margins for one proposed segment |
| `probe-via --at X Y --net N` | Named copper/keepout blockers for a through via on both layers, using its full diameter |
| `compile --proposal FILE.json` | Resolve explicit point references |
| `check --proposal FILE.json` | Independent exact semantic validity and score of a full edit |

`probe` is diagnostic; it does not validate complete connectivity, vias, or board
boundary conditions. `check` is the exact acceptance authority for this reduced
geometry contract. Use only when your assigned experiment permits feedback.
Record every probe/check and keep intermediate proposals.

Both `probe` and `probe-via` accept optional `--proposal FILE.json` to inspect
the geometry after all explicitly supplied route replacements together. This
avoids treating a route you intend to move as an unchanged obstacle. The
proposal is structurally checked, but this option does not run exact validation.
Via probes omit drill and board-boundary rules and do not certify connectivity.

An alternative image view is available with:

`python3 benchmarks/analysis-lab/via_space.py BOARD --net N --bounds X0 Y0 X1 Y1 --output IMAGE.png`

Colored regions exclude through-via centers after accounting for diameter and
clearance on both layers; white is clear in this copper/keepout model. Rounded
corners and trace caps retain their circular geometry. It does not select a
location or prove a complete route. Optional `--proposal FILE` applies an
explicit transaction; optional `--at X Y` adds a marker and numerical via-probe
evidence. The CLI also emits JSON evidence, which an images-only experimental
arm may require redirecting to `/dev/null`.

In all arms, proposed point lists may use `"start"` and `"end"` in place of
the existing route's precise endpoint coordinates. `{"point_index":1}` references
an existing point on that route; `{"pad":"P1"}` references a named pad.
These are coordinate references only. For example, `["start","end"]` with
layers `["top"]` proposes an endpoint chord; it does not imply the chord is safe.
