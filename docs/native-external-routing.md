# Optional Freerouting backend

`route-kicad-board-freerouting` routes a cold native KiCad project while retaining
its placement and physical rules. The existing adaptive router remains the
default. The external backend shares the verified whole-board export/import
pipeline; its Python and Java helpers are bundled into the executable.

```sh
pcb-maker route-kicad-board-freerouting source board-id output external.json
```

Example `external.json`:

```json
{
  "jar": "/absolute/path/freerouting-2.2.4.jar",
  "java": "/absolute/path/jdk-26/bin/java",
  "python": "/usr/bin/python3",
  "maximum_seconds": 1500,
  "maximum_passes": 100,
  "pipeline_maximum_seconds": 1800
}
```

Tool paths resolve relative to the configuration file. The jar must have SHA-256
`f5ed374182900ccc78e473518bbb9f6b869f4a07159495f663a76f52bb10523b`.
The Java installation must include `javac`; Python needs the native `pcbnew`
module. Existing KiCad verification and SVG-rendering dependencies also apply.
Routing uses one thread and disables post-routing optimization. A private settings
file prevents ambient Freerouting configuration from overriding this policy;
the effective settings are recorded.

The source must have a matching schematic/project, two copper layers and no
existing routed copper or copper zones. Unsupported custom rules, exchange
representations and rule translations fail explicitly. Native layer names and
roles are preserved. Both copper layers are enabled for external track routing:
KiCad's signal/power/mixed/jumper designation is descriptive and permits tracks
on every copper layer. Only the exchange representation is changed, with a
structural check that other DSN content is identical. This does not support
existing planes or custom layer-disallow rules.
[KiCad layer semantics](https://docs.kicad.org/10.0/id/pcbnew/pcbnew.html).
Physical dimensions cannot be overridden through this configuration.

Success requires the imported board to pass full native verification. Incomplete
or rejected results return a nonzero exit status and retain their artifacts.
See `output/routing/report.json`, `output/backend.log`, and
`output/routing/result/inspection-combined.svg`. An unproven DSN shape-equivalence
field is separate from the final native validity check.

To try bounded internal bridges after the external router leaves open
connections, add `island_bridges` to the configuration:

```json
"island_bridges": {
  "routing": { "resolution_mm": 0.25, "max_expansions": 2000000 },
  "maximum_attempts": 4,
  "maximum_pair_candidates": 8
}
```

This optional stage discovers native copper islands and chooses existing-pad
pairs automatically. It runs only after a preserved imported board has native
opens and no DRC/ERC/parity findings. Physical rules are compiled from that
native project; net-specific rules or forecasts are not accepted in this backend
option. Both stages share `pipeline_maximum_seconds`. The bridge process group
is terminated at the remaining deadline, retaining its last atomic selection.

`output/final-selection.json` identifies the final selected project and render.
The external report remains separate even when bridges complete the board.
Bridge-selected boards use their updated `preview.svg`; the original external
render shows the external stage only. See [bridge behavior and limits](native-island-bridges.md).

To route a newly generated native placement, pass `--freerouting-config
external.json` to `experiments/whole-board/area_probe.py` together with `--route`.
It is mutually exclusive with `--adaptive-config`; placement constraints and
full native acceptance remain unchanged.
