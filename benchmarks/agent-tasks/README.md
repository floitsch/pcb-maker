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
- **Kept where they are:** parts hanging over the outline, and parts whose
  courtyard holds other parts (a shield's outline, a module above parts),
  which become `hollow`: other parts may sit inside them.
- **Placed from scratch** (`move_all`): everything else, starting from one
  stack.

```sh
benchmarks/agent-tasks/from_pcbench.py build/agent-pcbench --subset d3-test
benchmarks/agent-tasks/run.py build/agent-pcbench-run --tasks build/agent-pcbench/tasks.json --jobs 3
```

### 2026-09-29, the 114 D3 test boards

Commit 93d331e, one binary for all tasks: **110/114 pass (96 %)**. Every
board that could be placed legally was routed clean.

| | This run |
| --- | --- |
| Time per task | median 9 s, mean 25 s, longest 290 s (3 in parallel) |
| Copper (110 passing boards) | 0.80 × the designer's; route mode on the designer's placement: 0.96 × |
| Vias | 0.52 × the designer's (295 against 569) |

- **Other methods.** For comparison, route mode on the designer's placement
  (the same boards) is 112/112 clean, and PCBWorld's Freerouting Clean Pass
  on D3-A is 0.80. There is no published layout-mode number to compare
  with.
- **Bugs and gaps these tasks found**, all fixed:
  - relaxed spacing let pads come closer than the copper clearance;
  - copper artwork occupied nothing;
  - SMD pads with a drill offset had the wrong box; a hole drawn with a
    token 0.001 mm pad was nearly invisible;
  - router-driven moves were never legal on a crowded board: they were
    checked against the full spacing the placement had given up;
  - layout mode never tried route mode's finer pitches when connections
    stayed open;
  - legalization gave up on parts a big part had left no room for (it
    now retries with them first);
  - the placement kept for the least wire is not always one that routes:
    when the first route leaves connections open, the other seeds'
    placements are routed too;
  - parts inside a shield's outline, parts with holes inside a courtyard
    (KiCad forbids it, unless the courtyard is malformed and KiCad skips
    it), connector bodies over the board's peg holes.
- **Progress.** 71/114 in the first run, 97 (with mixed binaries), 109,
  and now 110; no board has a copper error.
- **What is left:**
  - **4 boards cannot be placed legally**, all four by geometry:
    - FogDrive and HaveSome fill a side to 82-93 %; the hint says to move
      parts to the other side.
    - A 4.8 mm wide board whose parts are as wide as the board.
    - An 18.6 mm capacitor between mounting holes and copper text on a
      22 mm board.

## The quick tier (`--tier quick`)

The full harvested sweep (109 boards, 1800 s each) takes a night and was
memory-bound: three layout jobs at a time, most cores idle. The quick tier
judges a feature in a quarter of an hour by running many small boards side
by side, and is meant to be diffed against its last run with `compare.py`.

```sh
benchmarks/agent-tasks/run.py build/quick-x161 --tier quick --jobs 12 \
  --binary build/bin/pcb-maker-x161 > build/quick-x161.log
benchmarks/agent-tasks/compare.py build/quick-x160.log build/quick-x161.log --markdown
```

`quick.json` lists the boards (by task name in
`build/github-tasks-v5/tasks.json`, made by `from_pcbench.py --corpus
benchmarks/github/boards.json`), each with its layers, lattice, its v11
result and time, the failure kinds it covers, and the peak memory of its
last quick run. The rules that chose them:

- **Time:** the v11 sweep's time (three jobs at a time on a busy machine)
  under about 300 s. Two exceptions, the only small 4- and 6-layer boards
  with connections left open: MIDAS-MK1 (556 s, 4 layers, 8 unconnected)
  and OpenFC (671 s, 6 layers, 2 unconnected). The runner starts the
  longest boards first, so they do not lengthen the run.
- **Memory:** a peak of at most about 3.7 GB with 2 search threads. KiCad's
  DRC alone takes 2.2 GB on any board (`peak_rss_mb` counts it;
  `layout_rss_mb` is the layout process alone).
- **Coverage:** 26 two-layer, 9 four-layer and 2 six-layer boards; 21 that
  passed in v11, 9 with starved thermals, 6 with unconnected items, 3 with
  copper errors (a short and a starved thermal; solder-mask bridges or a
  connection width; an edge clearance), 2 that needed a finer lattice
  (0.05 and 0.075 mm).
- **Left out:** the five PolyKybd molecule panels, the OpenESC and OpenRX
  panels, krishveercard and SUMEC (not targets, see docs/handover.md);
  more than three boards of one family (bumwings, the second chiffre,
  OM-FlexGrid); jiran-ble-lite (7 GB a router at its fine pitch); every
  board slower than the limits above (Sisu, k30-SBC, the jetson, ...),
  which stay for the full sweep.

How the runner shares the machine:

- `--threads` (default with several jobs: 32 cores / jobs, at most 4)
  sets the router's search threads (`RAYON_NUM_THREADS`). Threads hardly
  speed a board up (k30-SBC's negotiation: 34.2, 32.4 and 31.3 s of search
  on 1, 2 and 4 threads) but each holds a search scratch of the board's
  size: k30 in route mode takes 907 MB with 32 threads, 539 MB with 2, in
  the same time. Jobs, not threads, use the cores.
- `--min-free-gb` is kept free after the expected peaks of the boards
  running: a board is admitted when the memory available, less what the
  running boards are still expected to grow by (their `peak_rss_mb` in
  `quick.json` minus their resident size now; `--expected-gb`, 3, for a
  board without one), leaves that much after its own expected peak. Jobs
  started together all see memory free that none has taken yet; the old
  check at start overcommitted. Admission comes before the designer's DRC
  (kicad-cli, 2.2 GB). Use 20 GB beside the Freerouting and cold-route
  sweeps, 12 on a quiet machine.

### First run, 2026-10-09, x160

x160 is HEAD 18268a6 with the memory changes (`docs/router.md`, Memory);
its routes are x155's (checked on six of these boards: the same routes,
vias, length and KiCad findings; CyberKeeb2040's layout process 6916 MB on
x155, 3395 on x160). 12 jobs, 2 threads each, `--min-free-gb 20` beside
the Freerouting and cold-route sweeps and three route-mode measurements:
**24 of 37 pass, 1654 s wall** (10266 s of board time: the gate, expecting
3 GB a board before `quick.json` had peaks, ran about 6 at a time). The
largest peak is 3.7 GB (bumwings_v001_xiao_s); KiCad's DRC sets the floor
of 2.2 GB. Against v11 (x136): ErgoSNM, bumwings_v001_core and
bumwings_v001_xiao_s now pass (their starved thermals are gone),
eternal-keypad lost its short but has 3 starved thermals, and PixelWave
panics in `refresh_pour` in the final ladder (index out of bounds in
`find`; x155 panics the same way on the same placement).

```sh
benchmarks/agent-tasks/compare.py build/quick-x160.log build/quick-<new>.log --markdown
```

| Board | Layers | Routed | KiCad unconnected / errors | Pass | Seconds | Peak MB (layout) | v11 |
| --- | ---: | --- | --- | --- | ---: | ---: | --- |
| 0xCB-1337__1337-v4.0 | 2 | 47/47 | 0 | yes | 84 | 2209 (382) | 47/47, 0, pass |
| 0xCB-Static__0xcb-static | 2 | 74/74 | 0 | yes | 27 | 2202 (798) | 74/74, 0, pass |
| Qfwfq__qfwfq | 2 | 64/64 | 0 | yes | 201 | 2284 (1414) | 64/64, 0, pass |
| spell_tome__spell_tome_bottom | 2 | 18/18 | 0 | yes | 38 | 2141 (315) | 18/18, 0, pass |
| Business-Cards__USB_Keypad | 2 | 39/39 | 0 | yes | 132 | 2214 (153) | 39/39, 0, pass |
| SmartSpin2k__SmartSpin2k_Panelized | 2 | 585/585 | 0 | yes | 115 | 2180 (1718) | 585/585, 0, pass |
| urchin__main | 2 | 68/68 | 0 | yes | 15 | 2170 (509) | 68/68, 0, pass |
| OnBoard__E-Fidget-Lite | 2 | 20/20 | 0 | yes | 30 | 2169 (227) | 20/20, 0, pass |
| OnBoard__keyboar_ | 2 | 102/102 | 0 | yes | 285 | 2246 (1949) | 102/102, 0, pass |
| OnBoard__MotionCubeViewAllForces | 2 | 20/20 | 0 | yes | 328 | 2166 (566) | 20/20, 0, pass |
| ligra__ligra_back | 2 | 33/33 | 0 | yes | 50 | 2184 (692) | 33/33, 0, pass |
| OpenMuscle-FlexGrid__OM-60-Flex | 2 | 21/21 | 0 | yes | 88 | 2161 (552) | 21/21, 0, pass |
| ergo-snm-keyboard__ErgoSNM_keyboard | 2 | 58/58 | 0 | yes | 32 | 2172 (1247) | 58/58, 0 / starved_thermal 2 |
| le_chiffre_keyboard_stm32__stm32_chiffre_36keys | 2 | 76/76 | 0 | yes | 463 | 2213 (1380) | 76/76, 0, pass |
| CharlieBoard__Blue_Line | 2 | 26/26 | 0 | yes | 28 | 2149 (2058) | 26/26, 0, pass |
| bumwings-kbd__bumwings_v001_core | 2 | 98/98 | 0 | yes | 726 | 3668 (3668) | 98/98, 0 / starved_thermal 4 |
| bumwings-kbd__bumwings_v001_xiao_s | 2 | 78/78 | 0 | yes | 90 | 3719 (3719) | 78/78, 0 / starved_thermal 2 |
| bumwings-kbd__bumwings_v001R64_rp2040zero_sd | 2 | 81/81 | 0 | yes | 47 | 2150 (2081) | 81/81, 0, pass |
| CyberKeeb2040__MainBoard | 2 | 118/118 | 0 | yes | 818 | 3392 (3395) | 118/118, 0, pass |
| Business-Cards__USB_Cable_Tester__PCB | 2 | 54/54 | 0 / connection_width 1 | no | 263 | 2173 (309) | 54/54, 0 / connection_width 1 |
| Business-Cards__WLED_Matrix | 2 | 101/104 | 1 | no | 550 | 2306 (1288) | 101/104, 1 |
| eternal-keypad__eternal-keypad | 2 | 69/69 | 0 / starved_thermal 3 | no | 172 | 2187 (744) | 69/69, 0 / starved_thermal 1, shorting_items 1 |
| osprey__osprey_rev_a | 2 | 101/103 | 1 | no | 402 | 2287 (1656) | 101/103, 1 |
| ULK__ULK_sl_PG1316s | 2 | 70/70 | 0 / starved_thermal 3 | no | 412 | 2220 (1656) | 70/70, 0 / starved_thermal 3 |
| OnBoard__PixelWave | 2 | panic in refresh_pour | - | no | 226 | 2282 (1118) | 343/343, 0 / starved_thermal 1 |
| silkscreen-pcb__silkscreen_pcb | 2 | 113/113 | 1 / copper_edge_clearance 1 | no | 419 | 2292 (1844) | 113/113, 1 / copper_edge_clearance 1 |
| 0xCB-1337__pcb | 4 | 85/85 | 0 | yes | 106 | 2157 (233) | 85/85, 0, pass |
| laptop__power | 4 | 115/115 | 0 | yes | 207 | 2192 (301) | 115/115, 0, pass |
| ISS-PCB__MIDAS-MK2 | 4 | 156/156 | 0 | yes | 352 | 2231 (405) | 156/156, 0, pass |
| ISS-PCB__MIDAS-MK2.1__MIDAS-MK2.1-revA | 4 | 154/154 | 0 | yes | 426 | 2220 (404) | 154/154, 0, pass |
| mackerel-68k__mackerel-08-v1 | 4 | 96/96 | 0 / starved_thermal 5 | no | 375 | 2286 (2290) | 96/96, 0 / starved_thermal 5 |
| ISS-PCB__BAGEL-MK1 | 4 | 138/138 | 0 / starved_thermal 1 | no | 398 | 2160 (586) | 138/138, 0 / starved_thermal 1 |
| ISS-PCB__MIDAS-MK1__MIDAS-MK1-revA | 4 | 154/159 | 8 | no | 713 | 2247 (703) | 154/159, 8 |
| OSHW-reCamera-Series__reCamera_S101_v1.1 | 4 | 55/56 | 4 | no | 331 | 2168 (725) | 55/56, 4 |
| Castor_and_Pollux__mainboard | 4 | 126/126 | 0 / starved_thermal 15 | no | 370 | 2149 (896) | 126/126, 0 / starved_thermal 15 |
| MyKiCad__framework_mobo_lefthalf | 6 | 71/71 | 0 | yes | 179 | 2125 (331) | 71/71, 0, pass |
| OpenFC-Lite__OpenFC | 6 | 80/82 | 2 | no | 768 | 2188 (668) | 80/82, 2 |
