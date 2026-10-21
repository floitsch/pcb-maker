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
  footprint by `uuid`; since tasks v7, `benchmarks/agent-tasks/tasks/v7`
  (committed; first made as `build/github-tasks-v7`), which is
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

Since x747 a layout is deterministic for a given input and binary (and
since the placement pool, whatever its thread count) unless the wall clock cuts it (docs/router.md, "Determinism and the
wall clock"; such rows carry `cut`, compare.py marks them CUT). One run per
side answers a quality question: a repeat gives the same rows. The work
rates are pinned at the reference's for every board (run.py sets
`PCB_WORK_RATE=3.15e6,150e6`; `--work-rate measured` lets each binary
measure its machine, which changes the rows with the machine and its
load), and rows carry `work_rate`, `work_rate_measured`, `work_wall_ratio`
and the rest of the verdict's rate fields (docs/router.md, "The machine's
rates"). The
benchmark, not the machine, was the bottleneck (a quick tier about an hour
at 6 jobs, a full sweep five hours, tiers serialised, pairs doubling every
measurement). The discipline:

- **Binaries and worktrees.** Work in a worktree of your own
  (`bash scripts/worktree.sh <name> <branch>`: `../pcb-maker-<name>` from
  main, with the main checkout's `build`, `artifacts` and harvested boards
  linked in; it prints the snapshot numbering rule), run
  `bash scripts/check.sh` before a commit (the test set, then a release
  build), and benchmark a copied binary: `bash scripts/snapshot.sh <n>`
  builds and copies it to `build/bin/pcb-maker-x<n>` and refuses a number
  that is taken. Never benchmark `target/release`, which the next build
  replaces. The current references are listed in docs/handover.md,
  "Benchmarks that count".
- **Iterate on the short tier** (`--tier short`, 12 boards at a third of
  the budget, about seven minutes at the tier default of 8 x 2; section below). It holds the
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
- **Normalised against the designer** (since 2026-10-21): raw sums are
  dominated by the few large boards (one jetson outweighs twenty
  keyboards), so the run's last line, summary.json's `normalised` and
  compare.py's lines below the sums give per-board measures as medians
  over the boards: our vias over the designer's, our track length over
  the designer's, KiCad's unconnected items over the board's connections
  (median and mean: most boards are at 0), and the ground pours' cut by
  other tracks (quality.py's "ground pour cut by other tracks", the
  length of other nets' tracks across the pours), ours less the
  designer's, with the boards whose designer pour ours lost. The
  designer's numbers are in `designer-numbers.json` (`quality.py
  --designer-numbers`, score-kicad-board on every board of tasks/v7, with
  each board's SHA-256; a row of another board gets none). The pour cut
  needs our result scored: `run.py --quality` writes quality.py's
  measures of both boards into each row's `quality` (score-kicad-board,
  about 0.1 s a board; `quality.py <run> --all` then reads them), and
  `compare.py --score <binary>` scores the result boards of older runs.
  Over v16 against v17 (103 boards with the same input): vias x0.70 ->
  x0.72 of the designer's, length x1.03 -> x1.03, open 0.009 -> 0.009 of
  the connections (mean 0.164 -> 0.113).
- **Placement or routing?** The designer tier (`--tier designer`, section
  below) routes every board on the designer's own placement;
  `compare.py --gap <layout log> <designer log>` splits the layout's open
  connections into a placement gap and a routing gap. Run it before
  deciding which side the next work goes to.
- **CI** (`.github/workflows/check.yml`): check.sh without the
  walkthrough (with the router's fingerprint test at 1 and 4 threads),
  clippy and rustfmt as gates, the walkthrough where kicad-cli is
  installed. It guards the code, not the benchmarks: tiers run here.
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
  ([docs/changelog.md](../../docs/changelog.md#committed-on-2026-10-14-the-legaliser-agent-x701--x711-the-premise-was-wrong)).
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
- **The task lists are committed** (2026-10-21): `benchmarks/agent-tasks/tasks/v6`
  and `tasks/v7` (tasks.json and the constraint files, 170 KB each,
  copies of `build/github-tasks-v6` and `-v7`); every tier defaults to
  `tasks/v7`. v6 and v7 differ only in six panels (the PolyKybd molecules
  and OpenESC's 4in1-panel), none in the quick or short tier, so v6 tier
  logs stay bases (test_run.py checks it). Their tasks.json is the same
  file in v2 to v7 (sha256 c8f95caf5089): `tasks_sha256` never told the
  lists apart, the constraint files did. So since 2026-10-21 each row also
  records `task_sha256` (the task's entry with its constraints' digest in
  place of the file name, without where the board lies) and
  `board_sha256` (the designer's board), and compare.py marks a board
  INPUT when either differs. The lists keep the main checkout's absolute
  board directories (so that their SHA-256 stays the reference logs');
  run.py finds the same `benchmarks/real/external/...` path under its own
  checkout where that one is not there.
- **One tier at a time on the machine**: since 2026-10-21 run.py takes
  `build/quick-tier.lock` itself (an exclusive flock) for every tier run
  and every run of a whole task list (not for a few boards given with
  `--only` and no tier), waits for it as long as it takes and prints who
  holds it (the holder's pid and command line, from /proc/locks); the
  older `flock build/quick-tier.lock benchmarks/agent-tasks/run.py ...`
  still works (a caller that holds the lock lets the run through).
  `--no-lock` skips it (a run that must not wait, beside a tier you know
  of). Start only with the load under 40 (`uptime`) and more than 20 GB
  available (`free -g`). Results above a load of about 45 are unsound (the
  layout's wall-clock guard fires and the rows are CUT).
- **Parallelism: tiers default to 2 search threads a board and physical
  cores / 2 boards at once** (8 x 2 on the 5950X; `TIER_THREADS` in
  run.py, since 2026-10-16; `--jobs` or `--threads` given override it,
  and either one alone keeps the older defaults for the other). A layout's
  rows do not depend on its thread count: the placement pool's kept seeds
  and charge are thread-independent and routing is single-net batches.
  Checked on x1151 (2026-10-16): urchin, USB_Cable_Tester and reCamera
  laid out at 1, 2 and 4 threads (`PCB_ROUTER_NO_WALL_GUARD=1`) gave
  byte-identical source, placed and result boards and identical rows; the
  fingerprints matched but for one rung of reCamera's last wave at 4
  threads, started beside another and dropped unjudged at the deadline
  (memory decides how many rungs run side by side, not which are
  judged). The whole short tier at 6 x 4, 16 x 1, 8 x 2 and 4 x 4 gave the
  same 12 rows (routed, KiCad's findings, vias, length, work seconds,
  first-route open terminals), none cut. So rows are comparable across
  thread counts, as long as no row is CUT (the wall guard is the one thing
  the threads and the load can still change); CPU and wall seconds are not
  comparable. Should a later change make a layout depend on its threads
  again, this stops holding: compare at the same `--threads` then. The
  short tier's wall, one run each under the lock (load 9-25):

  | jobs x threads | wall s | CPU s | board seconds summed | WLED_Matrix (longest) s |
  | --- | ---: | ---: | ---: | ---: |
  | 16 x 1 | 448 | 3562 | 2732 | 433 |
  | 8 x 2 | **401** | 3263 | 2006 | 339 |
  | 4 x 4 | 570 | 3305 | 1877 | 327 |
  | 6 x 4 (old habit, oversubscribed) | 482 | 4143 | 2258 | 406 |

  The tier is as long as its longest board, and a board gains from a
  second thread (the placement pool runs its seeds on the search threads,
  and the negotiation's few parallel batches) but hardly from more; 6 x 4
  asks 24 threads of 16 cores.

## The quick tier (`--tier quick`)

The full harvested sweep (109 boards, 1800 s each) takes a night and was
memory-bound: three layout jobs at a time, most cores idle. The quick tier
judges a feature in about half an hour by running many small boards side
by side, and is meant to be diffed against its last run with `compare.py`.

```sh
benchmarks/agent-tasks/run.py build/quick-x161 --tier quick \
  --binary build/bin/pcb-maker-x161 > build/quick-x161.log
benchmarks/agent-tasks/compare.py build/quick-x160.log build/quick-x161.log --markdown
```

`quick.json` lists the boards (by task name in its task list, the
committed `benchmarks/agent-tasks/tasks/v7/tasks.json` since 2026-10-21,
whose quick and short boards are v6's to the byte;
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
  panels, krishveercard and SUMEC (not targets, see [docs/handover.md](../../docs/handover.md#not-targets));
  more than three boards of one family (the other bumwings, the second
  chiffre, OM-FlexGrid); jiran-ble-lite (7 GB a router at its fine pitch);
  every board slower than the limits above (Sisu, k30-SBC, the jetson,
  ...), which stay for the full sweep.

How the runner shares the machine:

- `--threads` (a tier's default: `TIER_THREADS`, 2, with the physical
  cores / 2 jobs, "How to measure"; otherwise, with several jobs: the
  physical cores / jobs, at most 4, at least 1; since 2026-10-15, before
  the logical CPUs / jobs)
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
  and `summary.json` records `jobs`, `threads`, `physical_cores` and
  `parallelism` ("tier default" or "command line"). Since the placement
  pool (2026-10-14) a layout does not depend on its thread count ("How to
  measure"): rows of runs at other `--threads` compare, CPU and wall
  seconds do not.
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
flock build/quick-tier.lock benchmarks/agent-tasks/run.py build/short-x1180 --tier short \
  --binary build/bin/pcb-maker-x1180 > build/short-x1180.log
benchmarks/agent-tasks/compare.py build/short-x1170.log build/short-x1180.log
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
to 20; the harness's timeout is the budget times the layout's wall guard
(2) plus 300 s for KiCad's checks, at least 1800 s (before 2026-10-21: a
fixed 300 s and the rest of 1800 s scaled, 800 s; `--timeout` keeps that
rule). The layout ends by work since x747
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
   is copied and `import-kicad-netlist <root sheet> <project> <board-id> --layers N --footprints-from <designer's board>`
   (the designer's layer count; since x1071 the designer's board gives the
   footprints no library has and its versions of the others, as an agent
   handed the old board would do) imports it. When footprints are missing
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
   keep them. The designer's footprints *with* nets that have no symbol
   (0xcb-static's footprints for other switches, USB_Keypad's mounting
   holes plated on GND, TelemetryOnboard's test point holes) become `add`
   entries in `constraints.json` (`from_board`, at the designer's spot, the
   pads on the imported board's names of their nets; a reference the
   designer's board repeats gets a new one, `REF**_1`), and the import runs
   again to put them on the board (since x1071). Since x1100 the
   designer's zones go on the board as the layout tier keeps them
   (`strip-kicad-tracks` of the designer's board: every rule area and the
   pours that cover a good part of the board, unfilled), each pour on the
   imported board's name of its net (row field `zones`), and the outline's
   shapes and the footprints taken from the board keep the designer's ids,
   so that the project's DRC exclusions still apply.
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
| `stopped`, `why` | The first stage that fell short: `import` (an import error, or footprints missing: the layout still runs without them), `constraints` (they could not be written, or the layout called them an input error), `placement` (parts without a legal place, constraints missed), `routing` (KiCad's unconnected items, as the pass rule; before x1100 also the router's open connections, so MokyaLora's pass said `routing`), `drc` (new errors, or no report); null when the board passed. |
| `import` | The import's exit code, `missing_first` (footprints missing before libraries were added) and `libraries_added`. |
| `missing`, `missing_parts`, `substituted`, `unmatched_pins` | The import's report, after the libraries were added. |
| `from_board`, `board_versions`, `drifted`, `drifted_footprints` | Footprints taken from the designer's board for want of a library, the board's versions used over the library's, and the library footprints that differ from the board's by more than a fifth on a side or in pad count (the first twelve lines). |
| `parts` | `designer` and `imported` parts, `matched` (by path or reference), `copied` (board-only netless footprints), `added` (board-only footprints with nets, by `add`), `renamed` (references that differ between board and schematic), `board_only` and `schematic_only` (the first twelve), `dropped` (task entries for a reference the designer's board repeats without a uuid). |
| `zones` | The designer's zones copied (pours and rule areas, since x1100). |
| `parity`, `parity_by_type` | KiCad's schematic parity findings on the result. |
| `drc_line` | `drc-kicad-board`'s line. |

The run ends with the stages' counts (`stopped at: ...`) and the summed
missing footprints and parity findings, also in summary.json.

```sh
flock build/quick-tier.lock benchmarks/agent-tasks/run.py build/sch-x1002 --tier schematic \
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

**x1071 (footprints from the designer's board, `add`), 2026-10-15**, 6 jobs
x 4 threads as x1033 (main on 8a1fd2c, `build/sch-x1033.log`), under the
lock, load 12-33: **5/12 pass, 1168 s wall** (x1033: 5/12, 1568 s).
`build/sch-x1071.log`. Changed rows:

| Board | x1033 | x1071 | Why |
| --- | --- | --- | --- |
| bumwings R64 rp2040zero | 80/81, KiCad 1 unconnected, copper_edge_clearance 1 | **81/81, 0, passed**, 11 vias (87) | The board's switch footprints: the schematic names `Kailh_PG1350_*_hotswap` (15.9 x 12.2 mm, 10 pads), the board has `*_solder` (12.85 x 11.6 mm, 6 pads) on 58 of 64 keys (4 `drifted_footprints`, 59 `substituted`). Parity 7 `footprint_symbol_mismatch`: the copied schematic names one footprint per id, and the 1.5u hotswap is a 1u solder footprint on one key. |
| slab-pcb-left | 76/76, passed | 74/76, KiCad 2 unconnected | 125 board versions, none drifted (same body sizes and pads; an older library's drawing: courtyards as lines, text positions). The placement moved 53 of the 129 parts and the route left 2 open: a variance of the placement, not of the footprints. |
| OpenFC | 80/82, 2 / clearance 3, annular_width 4 | 80/82, 2 / **no new error** | The board's versions of the WSON, the TF socket and the USB-C (one or two pads fewer than KiCad 10's: 4 drifted by pad count) remove the 7 errors; parity 0 to 5 `net_conflict` (no-connect pins those footprints have no pad for). |
| USB_Keypad | 37/39, 2, 77 vias | 37/39, 2, 94 vias | Its four mounting holes plated on GND are added (`add`) and joined to GND; the crystal is the board's (6.1 x 3.4 against 6.0 x 2.4). |
| 0xcb-static | 74/74, passed, 43 substituted, parity 1 | 74/74, passed, 0 substituted, parity 0, 1 via | The 11 footprints for other switches (MX37_1, ...) are added on their nets; the board's footprints replace the library's renamed ones. |
| TelemetryOnboard | 106/108, KiCad 3 | 106/108, KiCad 2 | Three test point holes on TX4 and other nets added (`REF**_1` to `_3`); the route differs. |
| le_chiffre | 11 substituted, parity 1, 78 vias | 0 substituted, parity 0, 52 vias | The board's versions; still the one copper_edge_clearance. |
| coco2 | 614 vias, parity 1 | 661 vias, parity 0 | The board's versions (156); passed as before. |
| MokyaLora | 395 vias | 383 vias | The board's versions (443); 261/262 as before. |

spell_tome, urchin and homebrew_486 are identical (vias, routed, KiCad).
Missing footprints 0 in both runs; parity 31 to 40. The import of all 79
schematics (`experiments/schematic-import/import_all.py`): complete 34 to
77, missing footprints 1740 to 10 ([changelog](../../docs/changelog.md#committed-on-2026-10-15-import-footprints-from-the-board)).

**x1100 (the designer's DRC exclusions and zones), 2026-10-15**, 6 jobs x 4
threads as x1071, under the lock, load 12-32: **8/12 pass, 684 s wall**
(x1071: 5/12, 1168 s). `build/sch-x1100.log`; unconnected 9 to 8, new
errors 5 to 0, vias 2877 to 3577, work 2247 to 3710 s. Every row changed
but spell_tome's:

| Board | x1071 | x1100 | Why |
| --- | --- | --- | --- |
| urchin main | 68/68, copper_edge_clearance 3 | **passed** | The designer excludes 5 pads over the edge (`drc_exclusions`, by edge and pad id); the drawn outline and the board's footprints had new ids, so 3 came back. Both keep the designer's ids now. |
| le_chiffre 36keys | 76/76, copper_edge_clearance 1, 52 vias | **passed**, 62 vias | The same, by the outline's id (its KiCad 7 pads kept their `tstamp` by accident). The GND pours copied. |
| MokyaLora | 261/262, KiCad 1, courtyards_overlap 1 | **passed** (261/262, KiCad 0), 420 vias | R34 (0 ohm) and R35 (DNP) overlap by design, excluded by footprint id. The open pad is Mtr2's shield (a custom pad the router never reaches, as in the layout tier); the designer's GND pour on six layers joins it, as in the layout tier. At the full budget without the pour (x1072) it stayed open: not the budget. |
| slab-pcb-left | 74/76, KiCad 2, 419 vias | 75/76, KiCad 1, 474 vias | The budget: at `--budget-scale 1` it passes without the pours (x1072, 718 s of work) and with them (x1100, 332 vias; the layout tier 315). |
| OpenFC | 80/82, KiCad 2 | 81/82, KiCad 2 | U10 (SOT-23-5 on the back) as in the layout tier: v16 81/82 at the full budget, the short tier's x976 81/82 KiCad 2, the layout tier at a third on x1100 81/82 KiCad 2. 9 zones copied. |
| TelemetryOnboard | 106/108, KiCad 2 | 106/108, KiCad 3 | The USB-C pair and U1's RX4; the layout tier at a third on x1100 107/108 KiCad 2, v16 at the full budget 107/108 KiCad 1. 43 zones copied (36 rule areas). |
| USB_Keypad | 37/39, KiCad 2, 94 vias | 37/39, KiCad 2, 110 vias | The layout tier's loss: a decoupling capacitor at -1.9, 12.7 (C11 in v16, C15 in x903 and here), its GND pad off the hatched pour and its 3.3 V pad short of its track. |
| coco2 | passed, 661 vias | passed, 1144 vias | Its GNDS pours (177 vias on GNDS, 23 before). The layout tier at a third gives 573 vias and 22,373 mm against 26,093: the stack keeps the designer's orientations, the import puts every part at 0 degrees; with the designer's angles on the imported board the same layout gives 549 vias and 22,505 mm, with the stack's position alone 1144 and 26,093 (the placement follows the orientations it starts from). |
| homebrew_486, bumwings, 0xcb-static | passed | passed | The pours: vias 907 to 1008, 11 to 17, 1 to 2. |

## The expansion tier (`--tier expansion`)

46 boards of the 2026-10-18 intake ([benchmarks/expansion](../expansion/README.md)),
routed on the **designer's placement** with the tracks stripped, at the
intake's 30 seconds of work: a router tier over a wider spread of
designers, tools and hardness than the harvested layout tiers. Each task
is a route-mode task (`"mode": "route"` in the task, not `--route-from`):
no placement, no constraints.

```sh
flock build/quick-tier.lock benchmarks/agent-tasks/run.py build/expansion-x1211 --tier expansion \
  --jobs 8 --threads 1 --binary build/bin/pcb-maker-x1211 > build/expansion-x1211.log
# the intake's deep run (300 s of work)
flock build/quick-tier.lock benchmarks/agent-tasks/run.py build/expansion300-x1211 --tier expansion \
  --budget-scale 10 --jobs 8 --threads 1 --binary build/bin/pcb-maker-x1211 > build/expansion300-x1211.log
benchmarks/agent-tasks/compare.py build/expansion-x1211.log build/expansion-x1220.log
```

`expansion.json` (the quick.json format, `budget_scale` 1) lists the
boards; `expansion-tasks.json` is its task list. Both are written by
`from_expansion.py` from `benchmarks/expansion/analysis/expansion-tasks.json`,
`expansion_tier.csv` and the harvest's manifest; each board's entry has its
bucket, size, licence, the intake's quick and deep outcomes (and the
intake's stripped board's SHA-256, routed count, vias and length), and the
tier run's seconds, peak and result (`x1211`). Run it with `--jobs 8
--threads 1`, as the intake did (one search thread a board): the rows of
the cut boards depend on the load (below).

**Selection** (benchmarks/expansion/analysis/expansion-tier.json): from
the intake's quick run, boards whose designer board is native-DRC clean
(no error, no unconnected item) under its own project rules (not
inferred), with a repository licence in harvest.py's `USABLE_LICENSES`, no
quality flag (panel, backup, fixture), tool and QA mirrors excluded
(KiCad's own repositories, kisurf, kicad-assistant, Huaqiu's KiCad,
re-hosted SparkFun designs, qa, demo and test_data paths), no resource
limit at 300 s; one board per repository, round-robin over layer counts
(most layers first), larger boards first, scarce buckets first. The
buckets (by the intake's quick and deep outcomes):

| Bucket | Rule | Boards |
| --- | --- | ---: |
| B1 clean beyond 30 s | complete and native-clean, after more than 30 s of routing (wall: `routing_seconds`) | 12 |
| B2 near | incomplete at the budget, 1-2 connections open at 30 s | 9 |
| B3 far | incomplete at the budget with 3+ open, or cut by the wall guard, at 30 s | 12 |
| B4 clean only at 300 s | incomplete or cut at 30 s, native-clean at 300 s | 9 |
| B5 gives up early | `incomplete_early` (the router stops with connections open) | 4 |

27 boards are 2-layer, 15 4-layer, 4 6-layer; 44 to 1887 pads.

**The fetch.** A task carries its source: repository, pinned commit,
repository path, URL, licence, the harvest's directory name and
`files` (file name to SHA-256: the `.kicad_pcb`, and the `.kicad_pro` and
`.kicad_dru` where the harvest found them). When
`build/github-expansion/<task>/source` does not hold those files with
those digests, run.py fetches them: from the harvest's copy
(`--harvest-data`, default `benchmarks/expansion/data`, `boards/<name>`)
when it matches, else from GitHub at the pinned commit next to the board
(harvest.py's `GitHub.raw`, raw.githubusercontent.com, 40 MB at most).
Every file is verified before use; a mismatch or a missing file fails the
row (`fetch: ...`, outcome `fetch_error`) and keeps nothing (the files
arrive in a staging directory that replaces the source directory when all
are verified). Only those files are fetched or copied, and nothing in them
is run. Rows say `fetched` (and `fetched_from`: `harvest` or `github`) and
`source_sha256`.

**The licence rule.** Every task carries its repository's licence
(`source.license`, GitHub's SPDX id, repository metadata: per-file
exceptions are not checked), and every row prints it (`license`). run.py
refuses to fetch a board whose licence is missing, empty or `NOASSERTION`.
The boards are fetched for local benchmarking only; this repository does
not redistribute them.

**What a task runs**, as the intake did (benchmarks/expansion/triage.py):

- the board, its `.kicad_pro` and `.kicad_dru` copied to `<out>/<task>/source`;
  the board through `strip-kicad-tracks <original> <copy>` (the intake's
  exact arguments: segments, arcs, vias, cached fills and copper zones
  under 10 % of the board's area go; the large pours and the rule areas
  stay); the row's `stripped_sha256` is the stripped board's;
- `route-kicad-board source <id> route router.json --reference none` with
  the task's `router` (the intake's `seeds` 1 and
  `skip_native_verification`), `ladder_budget_seconds` = the task's
  `budget_seconds` (30) times the budget scale, and the intake's wall
  guard `deadline_seconds` = max(90, 3 x budget); the harness's timeout is
  the intake's max(180, 3 x budget + 60) s unless `--timeout` is given;
- `drc-kicad-board` (zones refilled) on the result, the router having
  skipped its own check; the pass rule is the other tiers' (KiCad's
  unconnected items and copper errors less the designer's, with
  `designer_budget`).

The row is a route row (`mode: "route"`, `routed`, `vias`, `length_mm`,
`native`, `work_seconds`, `cut`) with `license`, `fetched`,
`source_sha256`, `stripped_sha256`, `budget_seconds`, `routing_seconds`,
`open` (connections the router left open), `drc_seconds` and `outcome`,
the intake's class (`clean_fast` within 3 routing seconds, `clean_medium`
within the budget, `clean_slow`, `native_drc_failure`, `cut_wall_clock`,
`incomplete_at_budget`, `incomplete_early`, ...; triage.py's rule). The
run ends with an `outcomes:` line. `peak_rss_mb` is the larger of the
route process and the DRC; the memory gate expects the board's
`peak_rss_mb` from `expansion.json`. compare.py reads the rows unchanged
(route rows have no constraints digest).

**First run, x1211, 2026-10-21** (8 jobs x 1 thread under the lock, load
13-23, 471 s wall, largest peak 2.2 GB, 45 boards fetched from GitHub):
**14/46 pass**; outcomes clean_slow 11, clean_medium 1, incomplete_at_budget
21, cut_wall_clock 9, incomplete_early 4; KiCad 456 unconnected, 294 new
errors, 4194 vias, CPU 4846 s, work 3143 s, 13 rows cut. Per bucket: B1
12/12 pass; B2 2/9 (LC_PAMV4b and Barklind: the router leaves one connection
open, KiCad counts none unconnected); B3, B4, B5 none.

The reproduction is faithful: all 46 stripped boards are byte-identical
to the intake's, and the 32 boards that neither run's wall guard cut
routed identically (routed count, vias, length, work seconds). **45 of 46
boards have the intake's outcome class.** The one that differs,
DIYSynth's Multi_saturation_module, is the same route (8 vias, 10.5 s of
work) in 27.0 routing seconds against the intake's 37.8: B1's line between
`clean_medium` and `clean_slow` is wall time, so B1 membership is the
load's. The 14 rows cut in either run (the 90 s wall guard fired in the
intake on 14, here on 13: TokayEdge finished uncut) depend on the load:
of the 13 cut both times, 5 routed identically and 8 differently (Wonkle
262/271 then, 235/271 now). compare.py leaves CUT rows out of
better/same/worse, so a third of this tier is no A/B evidence at 30 s
unless the machine is idle. The binary did not change in between: the
1.57x work calibration (015676c, 3.15e6 expansions a second) is an
ancestor of be4b012, the intake's main, and the router and pcb-kicad
crates are unchanged since be4b012 (x1211 only drops the research stack);
the work seconds of the uncut rows match the intake's to the tenth.

Errors the intake did not see: it checked KiCad only on complete routes,
so the incomplete rows' errors are new information: backplane (201:
track_width and hole_to_hole), tuxkeychain (45 clearance), FUEL4EP (32
hole_clearance), rpi_cm5_dronecarrier (11), anthracite (3
starved_thermal), RS485M_Unit (2 starved_thermal).

At 300 s (`--budget-scale 10`, three B4 boards, Zulassungskarte,
nervous-ecg and 3117_AUSIGN, 3 jobs x 1 thread): 3/3 pass, `clean_medium`,
work seconds 164.8, 102.5 and 129.6, the intake's deep run's to the tenth.

**x1261, 2026-10-21** (the ladder's work bound and memory limits,
docs/changelog.md): the ladder's work stays within the budget and 5 %
(9-34.2 s, median 31.3; x1231: median 52.7, up to 254 s), no row is cut
(13 on x1231), the run takes 136 s of wall and 1553 s of CPU (337 and
4902): 15/46 pass. Boards that used to overrun stop at the budget; nine
end with more open connections (93 in all), four with fewer. The rows no
longer depend on the load.

## The designer tier (`--tier designer`)

**The deciding experiment for placement versus routing work.** A layout
that leaves connections open has two causes mixed: a placement that
cannot be routed, and a router that cannot route what a human could.
The designer tier separates them: every board of the corpus (tasks/v7,
`designer.json`) routed on the designer's own placement, and
`compare.py --gap` joins it with a layout run of the same list:

- **routing gap** = KiCad's unconnected items of the route on the
  designer's placement: what our router leaves open on a placement a
  human routed. Router work moves it.
- **placement gap** = the layout's unconnected items less the routing
  gap: what our placement adds (negative where our placement routes
  better than the designer's). Placement work moves it.

The corpus totals and each gap's share of the layout's open connections
say where the next work pays. A board the layout has no result for (no
legal placement, a timeout) is listed apart.

```sh
benchmarks/agent-tasks/run.py build/designer-x1225 --tier designer \
  --binary build/bin/pcb-maker-x1225 > build/designer-x1225.log   # about 5 hours at 8 x 2
benchmarks/agent-tasks/compare.py --gap build/github-layout-v17.log build/designer-x1225.log
```

Each board: the designer's project copied as the layout tier copies it,
its tracks and vias stripped as the layout tier strips them
(`strip-kicad-tracks`: the pours that cover a good part of the board and
every rule area stay, unfilled; small filled zones are hand routing and
go, as they go for the layout), the project's own rules (.kicad_pro,
.kicad_dru), the task's router settings, `route-kicad-board` with the
layout tier's budget (`route_budget_seconds`, 1500 s of work; the layout's
`total_seconds`), `--reference none` and the layout's pass rule (KiCad's
findings less the designer's). Rows are route rows with `placement:
"designer"`. Compare a designer run with a layout run of the same binary
(or say which binaries) and the same list; the gap is only as sound as
the rows are uncut (CUT rows are marked).
`designer.json` gives the memory gate and the runner each board's peak
and seconds from its v17 layout row (x1170).

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
