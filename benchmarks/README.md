# Benchmarks

**The benchmarks that count today** are the harvested GitHub boards as
agent tasks ([agent-tasks](agent-tasks/README.md): the 109-board layout
sweep and its quick, short and schematic tiers), the Freerouting
comparison on the same boards ([github](github/README.md)) and PCBench
([pcbench](pcbench/README.md)); their current numbers are in
[docs/handover.md](../docs/handover.md#benchmarks-that-count).

| Directory | What it is |
| --- | --- |
| [`agent-tasks`](agent-tasks/README.md) | layout tasks as an agent poses them: the runner, the tiers, compare.py |
| [`github`](github/README.md) | the harvested open-source boards: harvest, manifest, Freerouting comparison |
| [`pcbench`](pcbench/README.md) | PCBench through PCBWorld's D3 preparation: fetch, manifest, runner, triage |
| [`from-schematic`](from-schematic/README.md) | schematic to board, with only the starter files `import-kicad-netlist` writes |
| [`fence`](fence/README.md) | boards from the breadboard fence project, with an issue log and feature targets |
| `corpus` | the KiCad-demo corpus runner (route and layout) |
| [`real`](real/README.md) | frozen real-board sources; `real/external` (not in git) holds the harvested boards |
| `tscircuit` | the tscircuit autorouter on DSN exports, for comparison |
| `imported` | four layout-trace JSON fixtures that the tests of `pcb-kicad`'s semantic template and of `layout-trace-model` include; they go with those modules |

The September 2026 research ladders (the declaration-to-KiCad ESP32
ladders, the exact-routing rungs and mechanism controls in `small`,
`smoke`, `analysis-lab`, `competitive`, `esp32-pad-gaps` and the rest of
`imported`) were removed with the research stack on 2026-10-20; they are in
git history up to commit be4b012 (see
[docs/changelog.md](../docs/changelog.md)).

Check the imported fixtures' bytes with:

```sh
cd benchmarks
sha256sum -c manifest.sha256
```
