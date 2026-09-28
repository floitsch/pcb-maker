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

## Tasks generated from PCBench

`from_pcbench.py` turns PCBench/D3 boards into tasks
([../pcbench/README.md](../pcbench/README.md) prepares them).

- **Kept from the design:** the outline, and every connector the designer
  placed within 2 mm of an edge, on that edge and in its orientation.
- **Kept where they are:** parts hanging over the outline.
- **Placed from scratch** (`move_all`): everything else, starting from one
  stack.

```sh
benchmarks/agent-tasks/from_pcbench.py build/agent-pcbench --subset d3-test
benchmarks/agent-tasks/run.py build/agent-pcbench-run --tasks build/agent-pcbench/tasks.json --jobs 4
```

### 2026-09-29, the 114 D3 test boards

Commit f07b3fa: **97/114 pass (85 %)**.
- **Other methods.** For comparison, route mode on the designer's placement
  (the same boards) is 112/112 clean, and PCBWorld's Freerouting Clean Pass
  on D3-A is 0.80. There is no published layout-mode number to compare
  with.
- **The first run found three placer bugs,** which KiCad reported as shorts
  and clearance errors:
  - relaxed spacing let pads come closer than the copper clearance;
  - copper artwork occupied nothing;
  - SMD pads with a drill offset had the wrong box.

  Two more changes helped:
  - a last legalization level on a 0.1 mm grid, for small crowded boards;
  - three placement seeds, keeping the best.
- **Progress.** 71/114 in the first run; no board has a copper error now.
- **What is left:**
  - **15 boards cannot be placed legally.** Some designs overlap courtyards
    on purpose (a breakout whose carrier footprint covers the whole board,
    two modules sharing space across the sides); D3's rules switch that
    check off, and pcb-maker keeps bodies apart. Others are shields with
    mechanically fixed headers, or boards with cutouts where a connector
    sits.
  - **2 leave a connection open.**
