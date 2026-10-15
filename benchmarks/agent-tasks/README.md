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
  placed within 2 mm of an edge, on that edge and in its orientation
  (where a reference repeats, a panel's copies, each entry names its
  footprint by `uuid`; since tasks v7, `build/github-tasks-v7`, which is
  v6 with these uuids on six panels).
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

## How to measure

Since x747 a layout is deterministic for a given input, binary and thread
count unless the wall clock cuts it (docs/router.md, "Determinism and the
wall clock"; such rows carry `cut`, compare.py marks them CUT). One run per
side answers a quality question: a repeat gives the same rows. The
benchmark, not the machine, was the bottleneck (a quick tier about an hour
at 6 jobs, a full sweep five hours, tiers serialised, pairs doubling every
measurement). The discipline:

- **Iterate on the short tier** (`--tier short`, 12 boards at a third of
  the budget, about ten minutes at 6 jobs; section below). It holds the
  boards whose results flip; a change that moves nothing there will
  rarely move the quick tier.
- **Merge gate: the 30-board quick tier at the full budget** (`--tier
  quick`), once, diffed with `compare.py` against the last reference log.
- **Judge by continuous objectives from one run**, not by pass counts:
  passes move by single-error flips (USB_Cable_Tester passed in 5 of the
  12 rounds below, eternal-keypad's pass hung on one `shorting_items`).
  The run's last line (also `summary.json`) and compare.py's before/after
  lines give KiCad's unconnected items, new copper errors, vias, length,
  CPU seconds, work seconds, rows cut by the wall clock and the router's
  unconnected terminals after the first route, summed over the boards
  (new row fields: `work_seconds`, the verdict's work; and, for layouts,
  `first_unconnected_terminals`, from board-layout.json: the router's
  count of open terminals after the first route, not KiCad's).
  Totals compare only over the same rows with a result (compare.py sums
  the boards in both runs).
- **Router questions in route mode** (`--route-from <earlier output
  directory>`): the same placements routed again, no placement race, no
  legaliser wall time.
- **The DRC cache and CPU claims**: since x801 KiCad DRC reports are cached
  by board and rule hash (`~/.cache/pcb-maker/drc`, docs/agents.md). The
  side of a pair that finishes a board first pays the reference DRC and the
  other side hits it, which biases CPU seconds by 10 to 25 s per board, so
  CPU pairs run with `PCB_MAKER_NO_CACHE=1` on both sides, or both sides
  warm. Quality comparisons are unaffected: a hit is byte-identical to the
  stored run. run.py's own designer DRC is not cached yet.
- **Pinned simultaneous pairs only for CPU claims**: both binaries at the
  same time on symmetric halves of the cores, `taskset -c 0-7,16-23`
  against `8-15,24-31` (logical CPUs n and n+16 share a physical core:
  never give the two sides siblings), CPU seconds from rusage (the rows'
  `cpu_seconds`, or the shell's `time`: user plus sys), never wall seconds.
  A placement default change still needs several placer seeds per side
  (`--placer '{"seed": N}'`): it reshuffles crowded boards at random
  (docs/handover.md, the legaliser agent).
- **Lesson: one task list per comparison.** Pass `--tasks` on both sides
  or rely on the tier's default on both, never mix: v5 and v6 differ in
  the constraints of 12 of the 30 quick-tier boards (v6 fixes more parts),
  and quick30-x768 (v5) against x776 (v6) read as 174 more vias and 9 more
  errors from commits that changed no routing; on the 18 boards with the
  same constraints every row was identical. run.py now records `tasks`,
  `tasks_sha256` and `constraints_sha256` (the prepared constraints) in
  every row and the summary; compare.py names both lists on its first line
  (inferred from the prepared boards for older logs), says TASK LISTS
  DIFFER when they do, marks boards with other prepared constraints INPUT
  (not better / same / worse) and gives the objectives again over the
  boards with the same input. A reference log run before this change on
  the tier default (v5, the short tier's references among them) needs a
  v6 rerun before it is a base. Not checked:
  the prepared board itself (run.py's unplace rules can move a part
  without changing the constraints, as WLED_Matrix's LDO1 in x782).
- **One tier at a time on the machine**: take the lock,
  `flock build/quick-tier.lock benchmarks/agent-tasks/run.py ...`; start
  only with the load under 40 (`uptime`) and more than 20 GB available
  (`free -g`). Results above a load of about 45 are unsound (the layout's
  wall-clock guard fires and the rows are CUT).

## The quick tier (`--tier quick`)

The full harvested sweep (109 boards, 1800 s each) takes a night and was
memory-bound: three layout jobs at a time, most cores idle. The quick tier
judges a feature in about half an hour by running many small boards side
by side, and is meant to be diffed against its last run with `compare.py`.

```sh
benchmarks/agent-tasks/run.py build/quick-x161 --tier quick --jobs 8 \
  --binary build/bin/pcb-maker-x161 > build/quick-x161.log
benchmarks/agent-tasks/compare.py build/quick-x160.log build/quick-x161.log --markdown
```

`quick.json` lists the boards (by task name in its task list,
`build/github-tasks-v6/tasks.json` since 2026-10-14, the list of the v14
and v15 sweeps; `build/github-tasks-v5/tasks.json`, made by
`from_pcbench.py --corpus benchmarks/github/boards.json`, before: the tier
logs up to x782 and x768 are v5, x770, x780, x781 and x776 are v6), each with its layers, lattice, the failure
kinds it covers, its v11 result where it has one, and its time, peak memory
and result in the tier's run on x506. Since `run.py`'s unplace fix
(2026-10-10) every board is placed from stacked parts and routed, so the
times are layout times, two to four times v11's route-mode times. The
rules that chose the 30 boards (re-selected 2026-10-10 from the x506 run of
the old 37, 8 jobs, load about 45, and the v13 sweep):

- **Time:** a layout in about 1100 s or less on x506 (the longest,
  MIDAS-MK2.1, 1074 s; 13,698 s of board time in all, about half an hour
  of wall clock at 8 jobs). The runner starts the longest boards first.
  Times swing with the placement and the load (0xcb-static 153 to 403 s,
  osprey 384 to 1238 s between runs).
- **Memory:** a peak of at most about 3.7 GB with 2 search threads (the
  largest now 3.4 GB). KiCad's DRC alone takes 2.2 GB on any board
  (`peak_rss_mb` counts it; `layout_rss_mb` is the layout process alone).
- **Coverage:** 20 two-layer, 8 four-layer and 2 six-layer boards; 19 that
  passed in v11 (or v13 for the boards added), 6 with starved thermals, 4
  with unconnected items, 2 with copper errors, 2 that needed a finer
  lattice; framework_mobo_lefthalf stays as the cheap case the placer finds
  no legal placement for.
- **Dropped on 2026-10-10:** SmartSpin2k, PixelWave and ULK_sl_PG1316s
  (1800 s timeouts), MIDAS-MK1 (1671 s; MIDAS-MK1.1 takes its place, 4
  layers and connections left open in 696 s), laptop power (1336 s),
  osprey (1238 s), silkscreen (1204 s), CyberKeeb2040 (4.4 GB),
  bumwings_v001_core and bumwings_v001_xiao_s (4-6 GB; bumwings_v001 and
  bumwings_v001R64_nano_sd take their places in the family).
- **Left out:** the five PolyKybd molecule panels, the OpenESC and OpenRX
  panels, krishveercard and SUMEC (not targets, see docs/handover.md);
  more than three boards of one family (the other bumwings, the second
  chiffre, OM-FlexGrid); jiran-ble-lite (7 GB a router at its fine pitch);
  every board slower than the limits above (Sisu, k30-SBC, the jetson,
  ...), which stay for the full sweep.

How the runner shares the machine:

- `--threads` (default with several jobs: the physical cores / jobs, at
  most 4, at least 1; since 2026-10-15, before the logical CPUs / jobs)
  sets the router's search threads (`RAYON_NUM_THREADS`). Threads hardly
  speed a board up (k30-SBC's negotiation: 34.2, 32.4 and 31.3 s of search
  on 1, 2 and 4 threads) but each holds a search scratch of the board's
  size: k30 in route mode takes 907 MB with 32 threads, 539 MB with 2, in
  the same time. Jobs, not threads, use the cores. The work budgets are
  calibrated at one search thread a physical core: v16's five jobs of four
  threads on the 16 cores of the 5950X (beside other agents) ran the
  search 1.3-1.6 times slower than its counted work, and 8 of its 17 cut
  rows were that load. The physical cores come from /sys (else `lscpu`);
  an explicit `--jobs` x `--threads` above them is kept but warned about,
  and `summary.json` records `jobs`, `threads` and `physical_cores`. The
  thread count is part of a layout's input (a layout is deterministic per
  thread count): compare runs at the same `--threads`, so give it
  explicitly against logs made before this default (the short tier at 6
  jobs ran 4 threads, the default now gives 2).
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

## The short tier (`--tier short`)

For iterating on a change: 12 boards of the quick tier at a third of the
work budget, about ten minutes at 6 jobs. Use the quick tier at the full
budget as the merge gate ("How to measure").

```sh
flock build/quick-tier.lock benchmarks/agent-tasks/run.py build/short-x791 --tier short --jobs 6 \
  --binary build/bin/pcb-maker-x791 > build/short-x791.log
benchmarks/agent-tasks/compare.py build/bench-short/short-x791.log build/short-x791.log
```

**Selection, by flip history.** `flippers.py` reads the quick-tier logs of
the rounds since the placement gap work (x644, x651, x693, x722, x737,
x746, x764, x768, x770, x780, x781, x782; one log a round, in order) and
counts, per board, how often its pass verdict (compare.py's rule) and
KiCad's unconnected count changed from one round to the next ("flips";
an error row counts as a value of its own) and how often its copper error
count changed. The boards that flip are the ones a change moves, so they
carry the signal; the boards that never flip pass every time and would
only spend the budget. Chosen: every board with two flips or more (nine),
the fastest board with one (spell_tome; mackerel-08 at 516 s, Castor at
996 s and rp2040zero left out to keep the tier short), and
two fast boards that never changed as controls (Blue_Line and urchin;
E-Fidget-Lite would do as well). MIDAS-MK1.1 and WLED_Matrix are the slow
ones kept: both have connections open in every round, the cases a router
or placement change should improve. Coverage: 8 two-layer, 2 four-layer
(reCamera, MIDAS-MK1.1), 2 six-layer (OpenFC, framework_mobo, the cheap
case the placer finds no legal placement for).

```sh
benchmarks/agent-tasks/flippers.py build/quick30-x{644,651,693,722,737,746,764,768,770,780,781,782}.log --markdown
```

Flips over the 12 rounds (Pass, Unconn., Errors: changes of the pass
verdict, of KiCad's unconnected count, of the copper error count; s and
CPU s: medians at the full budget):

| Board | Layers | Flips | Pass | Unconn. | Errors | Passes | Unconnected | s | CPU s | Short |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- | ---: | ---: | --- |
| Business-Cards__USB_Cable_Tester__PCB | 2 | 6 | 6 | 0 | 7 | 5/12 | 0-0 | 181 | 244 | sensitive |
| OpenFC-Lite__OpenFC | 6 | 5 | 2 | 3 | 0 | 1/12 | 0-2 | 420 | 1132 | sensitive |
| MyKiCad__framework_mobo_lefthalf | 6 | 4 | 2 | 2 | 2 | 3/12 | 0-0 | 34 | 257 | sensitive |
| OSHW-reCamera-Series__reCamera_S101_v1.1 | 4 | 4 | 0 | 4 | 0 | 0/12 | 4-10 | 326 | 1150 | sensitive |
| ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA | 4 | 3 | 0 | 3 | 0 | 0/12 | 9-11 | 870 | 2841 | sensitive |
| Business-Cards__WLED_Matrix | 2 | 2 | 0 | 2 | 7 | 0/12 | 4-6 | 664 | 2233 | sensitive |
| Business-Cards__USB_Keypad | 2 | 2 | 0 | 2 | 4 | 0/12 | 2-3 | 269 | 858 | sensitive |
| 0xCB-1337__1337-v4.0 | 2 | 2 | 2 | 0 | 2 | 11/12 | 0-0 | 248 | 358 | sensitive |
| eternal-keypad__eternal-keypad | 2 | 2 | 2 | 0 | 2 | 1/12 | 0-0 | 345 | 631 | sensitive |
| mackerel-68k__mackerel-08-v1 | 4 | 1 | 1 | 0 | 2 | 9/12 | 0-0 | 516 | 745 |  |
| spell_tome__spell_tome_bottom | 2 | 1 | 1 | 0 | 1 | 10/12 | 0-0 | 86 | 115 | sensitive |
| bumwings-kbd__bumwings_v001R64_rp2040zero_sd | 2 | 1 | 1 | 0 | 1 | 10/12 | 0-0 | 123 | 150 |  |
| Castor_and_Pollux__mainboard | 4 | 1 | 1 | 0 | 1 | 10/12 | 0-0 | 996 | 1220 |  |
| CharlieBoard__Blue_Line | 2 | 0 | 0 | 0 | 0 | 12/12 | 0-0 | 32 | 52 | control |
| OnBoard__E-Fidget-Lite | 2 | 0 | 0 | 0 | 0 | 12/12 | 0-0 | 71 | 92 |  |
| urchin__main | 2 | 0 | 0 | 0 | 0 | 12/12 | 0-0 | 74 | 146 | control |
| OnBoard__MotionCubeViewAllForces | 2 | 0 | 0 | 0 | 0 | 12/12 | 0-0 | 129 | 363 |  |
| bumwings-kbd__bumwings_v001 | 2 | 0 | 0 | 0 | 0 | 12/12 | 0-0 | 150 | 180 |  |
| ligra__ligra_back | 2 | 0 | 0 | 0 | 0 | 12/12 | 0-0 | 152 | 227 |  |
| OpenMuscle-FlexGrid__OM-60-Flex | 2 | 0 | 0 | 0 | 0 | 12/12 | 0-0 | 210 | 427 |  |
| 0xCB-Static__0xcb-static | 2 | 0 | 0 | 0 | 0 | 12/12 | 0-0 | 213 | 671 |  |
| bumwings-kbd__bumwings_v001R64_nano_sd | 2 | 0 | 0 | 0 | 0 | 12/12 | 0-0 | 225 | 258 |  |
| 0xCB-1337__pcb | 4 | 0 | 0 | 0 | 0 | 12/12 | 0-0 | 263 | 470 |  |
| ergo-snm-keyboard__ErgoSNM_keyboard | 2 | 0 | 0 | 0 | 0 | 12/12 | 0-0 | 366 | 637 |  |
| ISS-PCB__MIDAS-MK2 | 4 | 0 | 0 | 0 | 0 | 12/12 | 0-0 | 389 | 878 |  |
| Qfwfq__qfwfq | 2 | 0 | 0 | 0 | 0 | 12/12 | 0-0 | 394 | 559 |  |
| OnBoard__keyboar_ | 2 | 0 | 0 | 0 | 0 | 12/12 | 0-0 | 395 | 955 |  |
| ISS-PCB__MIDAS-MK2.1__MIDAS-MK2.1-revA | 4 | 0 | 0 | 0 | 0 | 12/12 | 0-0 | 545 | 1028 |  |
| ISS-PCB__BAGEL-MK1 | 4 | 0 | 0 | 0 | 0 | 12/12 | 0-0 | 673 | 1069 |  |
| le_chiffre_keyboard_stm32__stm32_chiffre_36keys | 2 | 0 | 0 | 0 | 0 | 12/12 | 0-0 | 690 | 961 |  |

**Budget.** `short.json`'s `budget_scale` (0.3333, `--budget-scale`
overrides it) scales the layout's work budget: `total_seconds` 1500 to
500 (the placement race stops at 40 % of it, the moves at 65 %, the final
ladder gets what is left), `move_seconds` 600 to 200, `polish_seconds` 60
to 20; the harness's timeout keeps a fixed 300 s for KiCad's checks and
scales the rest (1800 to 800 s). The layout ends by work since x747
(docs/router.md, "Determinism and the wall clock"), so the scale changes
what the layout does, not how the machine's load cuts it. Most quick-tier
boards finish well inside 500 s of work; the scale bites where the final
ladder runs (boards with open connections), which is where the flips are.
The placement seed is fixed by default (the placer's `seed` 1, the race's
16 candidates derived from it), so the tier needs no seed of its own;
`--placer '{"seed": N}'` changes it.

**Runs on x791 (main, 2026-10-14), 6 jobs, 4 threads each:**

| Board | Routed | KiCad unconnected / errors | Pass | Seconds | CPU s | Work s |
| --- | --- | --- | --- | ---: | ---: | ---: |
| WLED_Matrix | 102/104 | 4 / connection_width 8, copper_edge_clearance 2 | no | 503 | 1084 | 361 |
| eternal-keypad | 69/69 | 0 / shorting_items 1 | no | 421 | 669 | 259 |
| MIDAS-MK1.1 | 151/156 | 9 | no | 409 | 882 | 233 |
| USB_Keypad | 37/39 | 2 | no | 404 | 1099 | 512 |
| OpenFC | 81/82 | 1 | no | 362 | 971 | 449 |
| reCamera_S101 | 55/56 | 4 | no | 344 | 1017 | 60 |
| 1337-v4.0 | 47/47 | 0 | yes | 243 | 467 | 126 |
| USB_Cable_Tester | 54/54 | 0 | yes | 189 | 251 | 184 |
| spell_tome_bottom | 18/18 | 0 | yes | 85 | 113 | 26 |
| urchin | 68/68 | 0 | yes | 54 | 128 | 44 |
| framework_mobo_lefthalf | no legal placement (M1, J1) | - | no | 43 | 326 | 18 |
| Blue_Line | 26/26 | 0 | yes | 34 | 57 | 4 |

(Work: the verdict's, the final ladder included; the first run's rows
carry the report's, which stops before that ladder.)

- First run, load 36-48 (other agents' sweeps beside it): **604 s wall**,
  5/12 pass; objectives: unconnected 20, new errors 11, vias 1017, length
  19768 mm, CPU 7063 s, cut 0, first-route open terminals 41 (11 rows).
  `build/bench-short/short-x791.log`.
- Repeat, load 47-59: 736 s wall; the 11 rows the wall clock did not cut
  are identical to the first run (routed, unconnected, errors, vias,
  length); WLED_Matrix was CUT (the layout's wall guard, 1.1 times the
  500 s budget, fired under that load) and lost 8 connection_width errors
  and gained 10 vias. `build/bench-short/short-x791b.log`.
- The board time is about 3100 s (515 s a job at 6 jobs, the bound for
  any order); the six largest boards start first. Neither run had an
  idle machine; on one the boards' wall time shrinks toward their work.
- A third of the budget leaves less wall-clock margin in absolute terms
  (50 s over 500 s of work, while KiCad's checks and the legaliser do not
  shrink): WLED_Matrix is cut above a load of about 45. Keep the load
  under 40, and read a CUT row as no evidence.
- Against the full budget (x782's quick-tier rows; x782 to x791 changed
  only the command line's exit codes): 9 of the 11 boards with a result
  give the same board (routed, KiCad's counts and errors, vias), in less
  than half the wall clock (MIDAS-MK1.1 907 to 409 s, 1337-v4.0 505 to
  243 s); reCamera 53/56, 9 open to 55/56, 4 open, and WLED_Matrix
  101/104, 5 open, 3 connection_width to 102/104, 4 open, 8
  connection_width. Unconnected 26 to 20, passes 5 and 5. Compare
  short-tier runs with short-tier runs: the budget changes the boards
  whose final ladder runs.

## The schematic tier (`--tier schematic`)

The other tiers start from the designer's board with its parts stacked.
An agent starts from the KiCad project: the schematic and the project's
libraries. This tier runs the documented flow (docs/agents.md) on 12
harvested boards from their schematics (benchmarks/github/README.md,
"Schematics"), so that the import, the libraries and the constraints are
measured with the layout:

1. **Import.** The fetched project (`benchmarks/real/external/github-sch/<board>/repo`)
   is copied and `import-kicad-netlist <root sheet> <project> <board-id> --layers N`
   (the designer's layer count) imports it. When footprints are missing
   and `harvest_schematics.py` found their library elsewhere in the
   repository (a designer's absolute path, a library outside the project
   folder), the library goes into the project's `fp-lib-table` and the
   import runs again into the same directory, which is what agents.md tells
   an agent to do.
2. **Constraints.** The task list's constraints for the board
   (`from_pcbench.py`, tasks v7) are moved onto the new footprints
   (`schematic.py`): footprints are matched by the symbol they come from
   (`path`), else by a unique reference, and entries name the new
   reference (with `uuid` where it repeats). The parts the task keeps
   where they are (`fixed`, netless and locked parts) become `place`
   entries at the designer's spot; every part goes on the designer's side
   (`back`/`front` by the footprint's layer: the other tiers' stack keeps
   each part's side). The designer's Edge.Cuts are drawn onto the board
   (the outline is the task's, as the designer drew it: arcs and slots
   included, which `outline`'s `polygon`, since x1041, would approximate), and
   the designer's netless footprints that have no symbol (mounting holes,
   logos) are copied, fixed and marked `board_only`, as the other tiers
   keep them.
3. **Layout** with `layout-kicad-board <project> <board-id> <out> auto <project>/layout.json`
   at the tier's budget (a third, as the short tier), then
   **`drc-kicad-board`** on the result.

**Pass rule:** the layout's (every constraint kept, KiCad's DRC with no
unconnected item and no error beyond the designer's board's), and the
import found every footprint. KiCad's schematic parity count (`parity`,
`parity_by_type`, from the layout's verdict) is recorded, not judged:
no-connect pins without a pad are `net_conflict`s the verdict says need
nothing.

Each row has the layout row's fields and:

| Field | Meaning |
| --- | --- |
| `stopped`, `why` | The first stage that fell short: `import` (an import error, or footprints missing: the layout still runs without them), `constraints` (they could not be written, or the layout called them an input error), `placement` (parts without a legal place, constraints missed), `routing` (connections open, by KiCad or the router), `drc` (new errors, or no report); null when the board passed. |
| `import` | The import's exit code, `missing_first` (footprints missing before libraries were added) and `libraries_added`. |
| `missing`, `missing_parts`, `substituted`, `unmatched_pins` | The import's report, after the libraries were added. |
| `parts` | `designer` and `imported` parts, `matched` (by path or reference), `copied` (board-only netless footprints), `renamed` (references that differ between board and schematic), `board_only` and `schematic_only` (the first twelve), `dropped` (task entries for a reference the designer's board repeats without a uuid). |
| `parity`, `parity_by_type` | KiCad's schematic parity findings on the result. |
| `drc_line` | `drc-kicad-board`'s line. |

The run ends with the stages' counts (`stopped at: ...`) and the summed
missing footprints and parity findings, also in summary.json.

```sh
flock build/quick-tier.lock benchmarks/agent-tasks/run.py build/sch-x1002 --tier schematic --jobs 6 \
  --binary build/bin/pcb-maker-x1002 > build/sch-x1002.log
```

**Selection** (`schematic.json`, each board's `why`): 12 of the 79 boards
with a usable schematic whose import is complete (after the library step),
mixed in size and in what the import has to do: 8 two-layer boards
(spell_tome 25 parts, urchin, USB_Keypad, 0xcb-static, le_chiffre,
bumwings R64, slab-pcb-left, coco2), 2 four-layer (homebrew_486,
TelemetryOnboard), 2 six-layer (OpenFC, MokyaLora, 443 parts). Small boards
first: the expected board time at a third of the budget is about 2,500 s,
under 15 minutes at 6 jobs. Each exercises something: a KiCad 6 project
(urchin), board-only holes and logos copied (urchin 42, USB_Keypad 13),
a library outside the project added by the agent's step (0xcb-static, 43
footprints), substituted footprints (le_chiffre 11), references the board
renamed after the schematic (bumwings, 12: matched by symbol path), a
KiCad 5 library (coco2, OpenFC; fixed in x1030). laptop power was taken out: KiCad 10's
ESP32-S2-MINI-1 is 45.4 x 35.25 mm (its antenna clearance in the
courtyard) against 15.9 x 21 mm on the designer's board, and the
64 x 45 mm board has no room for it (a library drift no layout fixes; open
problem 9). Four boards are in the short tier too (spell_tome, urchin,
USB_Keypad, OpenFC).

**First run, x1002 (main), 2026-10-15**, 4 jobs x 2 threads (the machine
was shared; the tier is meant for 6 jobs), load 9-21, under the lock:
**4/12 pass, 1034 s wall** (board time 2,710 s; the memory gate held it to
two or three boards at a time). `build/sch-x1002.log`.

| Board | Layers | Parts (designer / imported / copied) | Import | Routed | KiCad unconnected / new errors | Vias | Parity | Stopped | s | Work s | Layout tier (v16, full budget) |
| --- | ---: | --- | --- | --- | --- | ---: | --- | --- | ---: | ---: | --- |
| spell_tome_bottom | 2 | 25 / 25 / 0 | complete | 18/18 | 0 | 10 | 0 | passed | 33 | 29 | 18/18, pass |
| urchin main | 2 | 122 / 80 / 42 | complete | 68/68 | 0 / copper_edge_clearance 3 | 41 | 6 net_conflict | drc | 51 | 53 | 68/68, pass |
| USB_Keypad | 2 | 98 / 81 / 13 | complete | 37/39 | 2 | 77 | 0 | routing | 180 | 26 | 37/39, 2 |
| 0xcb-static | 2 | 129 / 112 / 6 | 43 missing, then complete | 74/74 | 0 | 0 | 1 net_conflict | passed | 88 | 39 | 74/74, pass |
| le_chiffre 36keys | 2 | 118 / 117 / 1 | 11 substituted | 76/76 | 0 / copper_edge_clearance 1 | 78 | 1 net_conflict | drc | 275 | 368 | 76/76, pass |
| bumwings R64 rp2040zero | 2 | 130 / 129 / 1 | 12 renamed | 80/81 | 1 / copper_edge_clearance 1 | 87 | 0 | routing | 323 | 40 | 81/81, pass |
| slab-pcb-left | 2 | 128 / 126 / 3 | complete | 76/76 | 0 | 261 | 6 net_conflict | passed | 183 | 315 | 76/76, pass |
| coco2 | 2 | 156 / 140 / 1 | 16 KiCad 5 footprints unseen | 159/159 | 87 / 199 shorting_items, 199 solder_mask_bridge, ... | 254 | 1 | routing | 244 | 308 | 173/173, pass |
| homebrew_486 | 4 | 130 / 127 / 3 | complete | 191/191 | 0 | 907 | 0 | passed | 279 | 471 | 190/190, pass |
| TelemetryOnboard | 4 | 143 / 140 / 0 | complete | 106/108 | 3 | 136 | 16 footprint_filters_mismatch | routing | 273 | 179 | 107/108, 1 |
| OpenFC | 6 | 164 / 162 / 0 | 2 KiCad 5 footprints unseen | 76/78 | 17 / clearance 3, annular_width 4 | 155 | 0 | routing | 199 | 56 | 81/82, 1 |
| MokyaLora | 6 | 443 / 443 / 0 | complete | 261/262 | 1 / courtyards_overlap 1 | 395 | 0 | routing | 385 | 166 | 261/262, pass (KiCad 0) |

(Parity: KiCad's schematic parity findings on the result, not judged.
Layout tier: the same board's row in the v16 sweep, `build/github-layout-v16.log`,
the designer's board stacked at the full budget.)

What the rows say:
- **coco2 and OpenFC lose parts silently on x1002**: their KiCad 5 library
  footprints (`(module ...`) were imported under that head, which pcb-maker
  does not read as a footprint: they stayed at the import's point, unplaced
  and unrouted (coco2: 87 unconnected and 199 shorts at the stack; OpenFC's
  SD socket and a sensor: 17 unconnected). Fixed in x1030 (below).
- **Library drift costs the boards that passed as layout tasks**: the
  import takes today's library footprints, the designer's board the ones of
  its day. bumwings' hotswap sockets are 15.9 x 12.2 mm in the repository's
  library now against 12.85 x 11.6 mm on the board (64 of them: 80/81
  routed against 81/81), urchin's and le_chiffre's edge errors are pads of
  library footprints that differ from the board's at the designer's fixed
  spots.
- **Board-only parts** (mounting holes, logos, mouse bites drawn as
  footprints) are copied when they have no net; those with nets are lost
  (0xcb-static's 11 alternative switch footprints, USB_Keypad's four
  mounting holes plated on GND).
- Where import and library agree (spell_tome, slab, homebrew_486,
  0xcb-static after the library step), the schematic row matches the
  layout tier's.

**x1030 on the three boards the branch's import fixes touch**
(`--only ... --splice-from build/sch-x1002.log`, `build/sch-x1030-three.log`,
after a harvest refresh): **5/12 pass**. coco2 173/173, KiCad 0
unconnected, no new error, 614 vias: passed (as the layout tier's 173/173);
OpenFC 80/82, KiCad 2 unconnected, clearance 3, annular_width 4 (the layout
tier: 81/82, 1); 0xcb-static passed, its library now found in the project
on the first import with 43 footprints substituted by name (parity 44 from
the KiCad 6 schematic's per-instance footprint, rewritten since x1031:
1).

## Route mode (`--route-from`)

For router experiments: `--route-from <earlier output directory>` routes
each board's placement from that run (`<dir>/<task>/layout/placed`, the
layout's final placement without tracks) with `route-kicad-board` instead of
laying it out: no placement race, no legaliser wall time, the same
placement for both binaries. The task's router settings go with it
(`add_pours`), `--budget-scale` scales `ladder_budget_seconds` (default
1200 s of work), and the row and pass rule are the layout's (KiCad's
unconnected items and copper errors less the designer's findings) with
`mode: "route"`, `route_from`, `constraints: 0` (the placement is given)
and `work_seconds` (the ladder's work). With `--tier short` the tier's
third of the budget applies to the ladder too; `--budget-scale 1` routes
at the full budget. The binary runs with `--reference
none`, as a layout does: its own verdict counts every error, the row's
pass subtracts the designer's.

```sh
flock build/quick-tier.lock benchmarks/agent-tasks/run.py build/route-x791 --tier short \
  --route-from build/quick30-x782 --jobs 6 --binary build/bin/pcb-maker-x791 > build/route-x791.log
```

Verified 2026-10-14 on x791 against manual routes of the same placements
(`route-kicad-board placed <id> out auto --reference none`,
`RAYON_NUM_THREADS=4` as the runner sets with 2 jobs):

| Board | Runner row | Manual route |
| --- | --- | --- |
| USB_Cable_Tester (x782 placement) | 54/54, 0 unconnected, 86 vias, 2292.4 mm, 144.7 s work, pass | 54/54, KiCad 0 unconnected, 86 vias, 2292.4 mm, 144.7 s work; 1 courtyards_overlap (the designer has 2) |
| reCamera_S101 (x782 placement) | 53/56, 9 unconnected, 60 vias, 662.4 mm, 799.0 s work | 53/56, KiCad 9 unconnected, 60 vias, 662.4 mm, 799.0 s work |

## Incremental sweeps (`--splice-from`)

`--splice-from <log>` with `--only A B ...` runs only those boards and
writes a log (and results.json) where the tier's other boards' rows are
copied from the given log (a run.py log or output directory; the last row
of a board wins) and marked `spliced: true`. The pass line and the
objectives line count the whole tier and say how many rows are spliced;
`compare.py` marks spliced rows SPL and prints their count. Use it to
rerun the boards a change can touch, or the rows a run lost to a cut,
without a full tier. A spliced row is the old binary's: say so when the
result is quoted.

```sh
flock build/quick-tier.lock benchmarks/agent-tasks/run.py build/quick-x791-two --tier quick \
  --only bitshiftcrazy__spell_tome__spell_tome_bottom tomunderwood99__CharlieBoard__Blue_Line \
  --splice-from build/quick30-x782.log --jobs 2 --binary build/bin/pcb-maker-x791 > build/quick-x791-two.log
benchmarks/agent-tasks/compare.py build/quick30-x782.log build/quick-x791-two.log
```

The demo above (build/bench-short/splice-demo.log): 23/30 pass, 28 rows
spliced, compare.py: "30 boards in both: better 0, same 29, worse 0;
unconnected 26 -> 26; passes 23 -> 23" and "after: 28 of the 30 rows
spliced from an earlier run (SPL)".
