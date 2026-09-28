# Agent layout tasks

Can pcb-maker turn what an agent has into a finished board? Each task
starts where an agent starts:

- a real board with its tracks stripped;
- usually every movable footprint stacked at one point and no outline, the
  way a fresh netlist import looks;
- a `constraints.json` like an agent would write ([docs/constraints.md](../../docs/constraints.md)).

Then `layout-kicad-board` places and routes it.

**Pass criteria**, all of them:
- every constraint holds, except those the task lists as geometrically
  impossible;
- every connection is routed;
- native KiCad reports no unconnected item and no copper error.

```sh
benchmarks/agent-tasks/run.py build/agent-tasks
```

## Tasks

| Task | What the agent asks for |
| --- | --- |
| `hierarchy-from-netlist` | KiCad's complex-hierarchy demo from a netlist: a 100 × 80 mm board, power connectors on the left edge, signal connectors on the right, a decoupling capacitor at its amplifier, a ground plane on the top. |
| `esp32-dut-from-netlist` | An ESP32-C3 test board from a netlist: 65 × 56 mm, the controller module on the left edge, the device under test on the right, service headers on the top edge, the system header flush along the bottom, the power header on the left, decoupling within 4 mm of each module. |
| `esp-usb-corner` | The breadboard fence's USB/ESP corner: an ESP32-C3 module flush in the corner, its antenna keepout beyond the top edge; a USB-C receptacle below it, opening outwards; the regulator between them; capacitors at their pins. Two distance limits there are impossible (a pad 8.4 mm inside the module's courtyard) and are expected to be missed. |

## Results

2026-09-28, commit a05d028, all three pass:

| Task | Routed | Vias | Copper | Constraints | Time |
| --- | --- | --- | --- | --- | --- |
| hierarchy-from-netlist | 50/50 | 0 | 1012 mm | 7 of 7 | 48 s |
| esp32-dut-from-netlist | 42/42 | 21 (designer: 57) | 1021 mm | 10 of 10 | 148 s |
| esp-usb-corner | 107/107 | 163 | 3414 mm | 8 of 10 (the 2 impossible ones missed) | 430 s |
