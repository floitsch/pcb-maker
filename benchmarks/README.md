# Benchmarks

**The benchmarks that count** are the harvested GitHub boards as agent
tasks ([agent-tasks](agent-tasks/README.md): the 109-board layout sweep,
its quick, short, schematic and designer tiers, and the route-mode
expansion tier), measured every night on main. Their current numbers are in
[docs/benchmarks.md](../docs/benchmarks.md) and
[docs/handover.md](../docs/handover.md).

| Directory | What it is |
| --- | --- |
| [`agent-tasks`](agent-tasks/README.md) | layout tasks as an agent poses them: the runner, the tiers, compare.py, the nightly |
| [`github`](github/README.md) | the harvested open-source boards: harvest, manifest, licences, schematics, the Freerouting and KiCadRoutingTools comparison scripts |
| [`expansion`](expansion/README.md) | a wider intake of GitHub boards; the expansion tier's source |
| [`pcbench`](pcbench/README.md) | PCBench through PCBWorld's D3 preparation: fetch, manifest, runner, triage |
| [`from-schematic`](from-schematic/README.md) | schematic to board, with only the starter files `import-kicad-netlist` writes |
| [`fence`](fence/README.md) | boards from the breadboard fence project, with an issue log and feature targets |
| `corpus` | the KiCad-demo corpus runner (route and layout) |
| [`real`](real/README.md) | pinned upstream KiCad demo projects; `real/external` (not in git) holds every fetched or harvested board |
| `tscircuit` | the tscircuit autorouter on DSN exports, for comparison |

`harness_common.py` holds what the runners share (the memory ledger's
admission among it); `ledger_headroom.py` prints the measured memory floor
run.py uses.
