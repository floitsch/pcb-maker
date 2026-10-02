# Hand-over, 2026-09-29 18:20

The state of pcb-maker at the end of a long working day: what the goals are,
what is done, what is running, what is half-built, and what comes next. All
work is committed locally on `main`; **nothing is pushed.**

## Goals, in the order they were set

1. **Open-source boards as benchmarks** (PCBench/D3). Done: see
   `benchmarks/pcbench/README.md` and `benchmarks/agent-tasks/README.md`.
2. **"Make pcb-maker the preferred tool to fully create a PCB."** The three
   needs are:
   - objective quality measures (thermals, analog/digital separation,
     return paths, EMC);
   - easy ways to state non-objective intent ("the button goes left, the
     front is there");
   - lower cost for hackers (fab prices).

   Later addition: make more use of pin swapping (nice to have).
3. "Implement the TopoR algorithm (at least as an option)." Done and
   parked: see [The topological engine](#the-topological-engine-done-parked).
4. **Current goal (2026-09-30):** "an amazing open-source tool to create
   PCBs". It must be fast (faster than any open-source tool, ideally as
   fast as commercial ones), it must scale beyond toy examples, and it must
   automate everything agents (the primary users) should not have to do.
   The research behind it is in
   [reviews/2026-09-29-topor.md](reviews/2026-09-29-topor.md).

## The flow as it stands

```sh
pcb-maker import-kicad-netlist myproject/myboard.kicad_sch board myboard [--layers 4]
pcb-maker layout-kicad-board board myboard layout auto board/layout.json
pcb-maker score-kicad-board layout/result myboard
pcb-maker export-kicad-fab layout/result myboard fab
```

The steps are documented in [agents.md](agents.md) (the agent guide,
sections 0-5), [constraints.md](constraints.md), [quality.md](quality.md)
and [cost.md](cost.md).

### Added today (about 85 commits, `7fc869d` .. `dd24e9a`)

**Import (`crates/pcb-kicad/src/netlist_import.rs`).**
- `import-kicad-netlist` turns a schematic or `.net` file into an unplaced
  board, the way KiCad's "Update PCB from Schematic" would.
- Footprints come from the project and user `fp-lib-table`s (any
  `${KICADn_FOOTPRINT_DIR}`, `$(KIPRJMOD)`), from `.pretty` folders in the
  project, and from KiCad's own libraries. If none has the name, it tries
  KiCad 9's renames (Female/Male to Socket/Plug). Substituted ids are
  written into the board *and* the copied schematic.
- The board carries the schematic's fields (LCSC numbers reach the BOM),
  DNP and BOM flags synced both ways, and sheet paths. Sheets in
  subfolders are copied.
- Fresh UUIDs per footprint instance. Same-numbered pads on an unconnected
  pin get separate nets (`_1`), as KiCad does.
- A footprint whose own pads are closer than the clearance rule gets their
  gap as its clearance. This is measured by KiCad's DRC on a spread-out
  copy, because KiCad caps each violation type at 499 on a stacked board.
- It writes a starter `constraints.json` (`move_all`, `outline: {}`, and
  plug-in connectors on `"edge": "any"`) and a `layout.json`, never over
  existing files.
- Checked against every KiCad demo whose libraries exist: schematic
  parity is clean.

**Placement.**
- `"edge": "any"`: a throwaway first placement, then the nearest edge.
- Outline sizing starts at the designers' median area: 2 times the parts'
  area on 2 layers, 1.5 on 4 or more (the D3 medians are 1.9 and 1.4). It
  grows by 25 % while a quick route leaves opens; `"shrink": true` shrinks
  by 20 % while the board still routes.
- `move_all` also moves parts without nets; mounting holes go to the
  corners. `benchmarks/agent-tasks/run.py` fixes the net-less parts, so
  the D3 tasks behave as before.
- Relations anchored on a moved part now count (`Relation::involves`).
- Rails are recognised by their own name (`/sheet/VCC_PIC`).
- Parts held at an edge keep their *pads* the copper-to-edge clearance
  from it; the reach counts from there.
- A last relaxation level, `edge_copper`, places parts held at an edge by
  their copper alone, centred when there is less than 2 mm of play. This
  handles a PCIe card edge in its tab.
- SMD pads on the other side's copper occupy that side (an edge-mount
  SMA).
- The extents of arcs follow the arc (`outline::outline_points`).

**Labels.** Text height as KiCad measures it (size + stroke, found by DRC
bisection). Footprint texts block labels, visible values move like labels,
and pad boxes include drill offsets.

**Score.** Separation now counts only switching inductors, against
crystals, antennas and analog parts. New: hot parts (a tab or exposed pad,
with its planes, vias and nearby copper).

**`describe-kicad-board`** lists `swap_candidates` (GPIO/IO/PA/P0.x pins,
not on connectors).

**Cost summary** shows assembly apart from the board price.

**`copy_directory_tree`** no longer copies an output directory that sits
inside the project into itself.

### Benchmarks

- **Schematic to board** (`benchmarks/from-schematic/run.py`, new): the
  KiCad demos imported and laid out with the starter files, unchanged.
  - The run that just finished (binary `vc833449`, older than the last
    fixes): 8 of 10 pass; the video board is still running.
    - ECC83 (both versions), sonde, Interf-U, PIC, hierarchy, StickHub and
      multichannel are complete with no DRC errors.
    - Sizes are 1-2 times the designers'. Interf-U is 191 x 128 mm against
      116 x 108; StickHub's designer used both sides.
  - CM5 failed:
    - 6 edge-clearance errors, fixed in `d1f18f7`;
    - 2 opens on "MP" pads, fixed in `8804d5d`;
    - 4 parity issues left, where the library footprint lacks the
      symbol's mounting pins (reported as `unmatched_pins`).
  - ColdFire failed with 208/209 and 10 unconnected items; not analysed.
  - **Rerun it with the current binary.**
- **D3 agent tasks**, all 616 boards (`build/agent-pcbench-all-run2`,
  binary v48 from this morning).
  - **The run crashed at 381 of 616**: `run.py`'s `block_end` raised
    "unbalanced expression" on some board. That is a runner bug; fix it and
    rerun.
  - Before the crash: 297 of 348 pass (85 %).
  - Fixed since and verified on the task: RX5808 (shorts), POV (hole in a
    courtyard), R1002, MicroMaple, BB-PWR and the ESP32 breakout (card edge
    and edge placement).
  - Remaining failures:
    - placement packing on crowded edges (FRM16, Starling, bobc,
      Mini-Ultra, wiflier);
    - "side over 100 % full" boards whose designers overlap courtyards;
    - routing opens (bms-8s50, bmw-ibus, TinyTracker, T962A);
    - MAVRIC's outermost PCIe finger.
- **The 114 D3 test tasks have not been rerun since this morning's 110/114
  (v48).** Many placer changes went in since. **Rerun before claiming
  anything:**
  ```sh
  benchmarks/agent-tasks/run.py build/agent-pcbench-run --tasks build/agent-pcbench/tasks.json --jobs 3
  benchmarks/agent-tasks/quality.py ...   # designer against ours
  ```

### Parked

- `experiment/edge-first-legalization`: a third legalization pass with
  edge-held parts first. It made FRM16's placement take over 8 min instead
  of 30 s, so it was abandoned.
- `experiment/graph-first-placement`: older, no gain.

## The topological engine (done, parked)

TopoR's algorithm is implemented in `crates/pcb-topo` as an opt-in engine
(`{"engine": "topological"}`). Design, results and verdict are in
[topological.md](topological.md).
- It completes the simple two-layer boards, and they are KiCad clean.
- It is far behind the lattice router on dense and four-layer boards.
- It is not developed further.

What stays useful from it is **`{"tighten": true}`**. The lattice router's
copper is pulled tight into any-angle tracks with arcs, wherever the
result stays legal. PIC: 372 of 374 pieces tightened, 1911 mm to 1841 mm,
KiCad clean.

Two debugging lessons came out of this work:
- An earlier session debugged the engine on an *unstripped* ECC83. The
  designer's tracks were net-less obstacles, and they caused its "unroutable
  wires". Always strip first: `strip-kicad-copper`.
- `PCB_TOPO_SVG=<dir>` pictures made the realization bugs obvious. Build
  pictures early.

## 2026-09-30: completeness first

Florian's direction after the TopoR work: **completeness before speed**
(speed only where it is cheap); Freerouting comparisons only when the
README is updated. State of the router work that followed:

### Done (committed)
- **Automatic planes** on 4+ layer boards without pours (GND on the first
  inner layer, busiest rail on the last; `automatic_planes: false`).
- **Exclusive planes** as a ladder rung: inner planes kept free of signals
  first, then open to them, then pour nets as tracks (`exclusive_planes`,
  `plane_cut_cost`). ColdFire wants exclusive, video wants open.
- **Neck zones**: within 1.5 mm of narrow pads a net routes at the board's
  neck width (`neck_reach`); escape stubs stamp at their real width (a bug:
  adjacent 0.5 mm pads' stubs collided at the full class width).
- Lattice phase: narrow pads vote per axis.
- Clean-up in parallel over disjoint windows; `present_factor`,
  `present_growth`, `stall_at_cap`, `via_reduction_present` (4 saves a
  third of Interf-U's time for 3 more vias; off) in the router config.
- Global tile-graph planner (`global_routing`, off: no gain yet, ColdFire
  slower).
- Agent-task runner: `REF**` parts really fixed (glob escapes); a runner
  failure fails its task only. 6 of 8 regressed tasks pass again.

### Open, with root causes found
- **Tiny Tapeout 82/108.** RP2040 QFN, 0.4 mm pitch, 0.2/0.2 rules. Nine
  +3V3/GND pads (0.23 mm class clearance) cannot be reached under the
  board's rules at all (the designer's board has those DRC errors). The
  rest need tracks at *exactly* the clearance next to each other, which
  the round clearance stamps (plus margin) forbid although KiCad accepts
  them. Needs direction-aware clearance in the lattice.
- **ColdFire GND 11-13 open.** GND and +3.3V alternate on the LQFP-100 at
  0.5 mm; one 0.8 mm via per pad cannot fit. The designer joins the power
  pads with tracks along the rows to 1 GND and 3 +3.3V vias. Pour nets
  need to form such trees; the fixed-stub pre-pass (`fixed_plane_stubs`,
  off) did not settle.
- **Brushless_ESC** (agent task, 46/50): its placement changed with
  43273fb (the switcher inductor pull, now limited to pins whose function
  says `SW`/`LX`) and 412f724 (rails by their own name, which changes the
  decoupling pulls). Both changes are right; the placement they give routes
  worse (5 -> 18 open on the first route). The placer does not optimise
  for routability, so heuristic changes swing completion on tight boards.
  The fix is a routability term in placement (congestion, crossings), not
  tuning against this board.
- **Fixed since:** PCB_constant_current_ac_hv (copper texts now move like
  labels), raspberry_pi_pullup_button (bodies may overhang the edge when
  their copper fits; a last level keeps only the rules' edge clearance),
  and a bad regression where every layout move failed ("incremental
  update needs the same nets and rules": the neck classes).
- **FogDrive, HaveSome_PCB** ("side 82-93 % full"): their designers put
  part bodies over other parts' outlines (resistors over a DIP), which the
  projects allow (courtyard overlap ignored; only holes may not lie inside
  a courtyard). Reproducing that needs hole-versus-courtyard legality in
  the placer instead of body boxes. Not done.

### Timings (unloaded machine, stripped boards)
Interf-U 63 s (via reduction is most of it), multichannel 96 s, ColdFire
375-415 s, video 730 s; Freerouting: ColdFire 1457 s with 8 unrouted,
video unfinished at 1800 s.

## 2026-09-30, later: more boards, fewer special cases

Florian: "we are focusing (specializing) too much on tiny-tapeout": get
other difficult boards, consider a fan-out phase; then: more benchmarks
from GitHub, strip and reroute, and place under constraints.

### Done (committed)
- **Exact clearance** (`2b96c06`): nanometre discs, bisector stamps for
  diagonal steps, a second map family for "a diagonal step from here would
  pass a node too closely". One conflict judge for rip-up and negotiation
  (they had diverged). A net stuck eight iterations is ripped up whole.
- **Ladder**: each attempt gets half the remaining budget; a rung where a
  quarter of the nets have no path is abandoned (`abandon_hopeless`).
- **GitHub harvest** (`benchmarks/github/`): 109 boards, 4-8 layers,
  BGAs/QFNs; route sweep `build/github-route-v1` (log of the same name)
  with `pcb-maker-x4` (before the fixes below), `summary.py` for the
  causes. Adapter gaps it found and their fixes: chamfered roundrect pads
  (exact union), rule-area holes (KiCad's further polygons; Glasgow's rim
  keepout covered the board). Open: per-layer padstacks (1 board).
- **Fixed branches hold their stamps through every rip-up** and the net's
  own search masks them (`mask_own`, epoch): the bulk rip-up before the
  hard rerouting used to unstamp plane skeletons and stubs, and other nets
  crossed them. The Jacobi mask reused generation 1 (a real bug).
- **Plane-stub pre-pass** (`fixed_plane_stubs`, in the KiCad config now)
  counts only inner planes, so every surface pad of a pour net gets its
  via before the signals route; unconflicted stubs stay fixed. A net may
  put its own vias inside its surface pads (`via_owner`). Framework
  mainboard half: GND 7 open -> 0. Eurorack: GND solved, connect rungs
  tighter, tracks rung wins 131/133 (as before). ColdFire: being measured.
- **Escape stubs** for fine-pitch rows (`escape_stub_mm`, off): no gain on
  Tiny Tapeout (82/108 either way) or ColdFire; the 0.4 mm QFN cannot
  stagger 0.62 mm vias at all (the designer vias 17 of 56 pads).

### What the harvested boards say (first 30 of 109, `pcb-maker-x4`)
1 clean, 17 open, 12 errors. Opens are mostly pour nets stranded on
shredded outer pours (hence the pre-pass), a few boards with dead 0.4 mm
pads under a 0.2 mm clearance (krishveercard: 9 of 19 nets pathless),
and big boards hitting the 900 s runner timeout (jetson 8L/391 fp,
ATAT1800). 2026-10-01: the second sweep is in benchmarks.md (92 boards: 25 clean,
42 open, 4 mismatch, 21 errors). From its failures, fixed the same day:
`unconnected-(...)` nets shared by two pads are routed (the 4 mismatches),
copper layers absent from the stackup are ignored (6 boards), KiCad 6/7
per-class net lists assign classes (clearance errors), via reduction is
capped by the attempt's budget (13 timeouts were mostly polish). Queued:
the layout tasks (`build/github-layout`, 107 tasks, `pcb-maker-x8`) and
then a rerun of the 67 not-clean boards with `pcb-maker-x10`
(`build/github-v3.sh` -> `build/github-route-v3`).
A second sweep with the newest snapshot (`build/bin/pcb-maker-x8`:
plane-stub rung, 0.127 mm neck when no minimum is stated, padstacks on
surface pads) is queued behind the first (`build/github-v2.sh`, output
`build/github-route-v2`); summarise both with `summary.py`, put the table
in benchmarks.md, then run the layout tasks (`build/github-tasks`).

## 2026-10-01/02: layout on the harvested boards

The first layout run (`build/github-layout`, 89 tasks, `pcb-maker-x8`)
passed 5. Causes and fixes (all committed): KiCad 6/7 references in
`fp_text reference` were never read (every part on those boards was
nameless: no constraint could address it); `(at x y unlocked)` lock flags
parsed as rotations (15 boards); Bezier outlines (8 boards), sliver outline
pieces, degenerate track arcs (5 boards failed to strip); footprints that
draw on Edge.Cuts moved with their parts (tasks now fix them); unnamed
netless parts stay under `move_all`; harvested-board tasks allow the
designer's own DRC findings (`designer_budget`). Speed: pour stitching
batched per pass (was one board-wide flood per via: 300 stranded GND pads),
clean-up budgeted (`cleanup_seconds`), via reduction capped. Running at the
hand-over: route sweep `build/github-route-v4` and layout
`build/github-layout-v2` (109 tasks, 2 jobs), both with `pcb-maker-x13`;
29 layout tasks had timed out at 1800 s before the speed fixes.

## 2026-10-03: memory and the hidden repair time

A layout check was killed by Claude Code for low memory: one route of
ATAT1800 (4 layers, 3952 x 1227 nodes) peaked at 8.65 GB. Bitset marks and
16-bit search generations bring it to 5.52 GB with identical results
(docs/router.md, "Memory"). The same board showed where large boards lose
their time: `resolve_remaining` (hard reroute after negotiation) took 960 s
for 107 nets; it is now budgeted by the negotiation's time and skipped on
hopeless rungs. Layout trial routes no longer polish, and the layout has a
total budget. Route sweep restarted as `build/github-route-v5`
(`pcb-maker-x17`); the layout sweep has not been rerun since the fixes
(run it with `--jobs 1` for memory).
Later the same day: seed retries only for attempts of at most 120 s (on
MIDAS-MK2 two retries took 740 s, both worse, and starved the rung that
completes it; now 156/156, clean apart from the designer's own findings).
The ladder's rungs now share its budget evenly (zpn_devboard completes,
ColdFire unchanged and faster). Sweep restarted as `build/github-route-v7`
(`pcb-maker-x19`, runner timeout 1500 s so the 1200 s ladder can finish). Of the 42
boards open in v2, jiran-ble-lite is limited by keepouts its designer
violates (`items_not_allowed` 40 in their DRC).

## Where things are

| What | Where |
| --- | --- |
| Benchmark outputs | `build/from-schematic/` (log `build/from-schematic.log`, first run `build/from-schematic-run1.log`), `build/agent-pcbench-all-run2/` (+ `.log`) |
| Snapshot binaries used by runs | `/tmp/claude-1000/-home-flo-programming-pcb-maker/bbd90446-2d49-4c91-90c8-c23269ad9495/scratchpad/bin/` (session scratch: may vanish) |
| TopoR research downloads | same scratch, `topor/` (manual text, papers) |
| Agent memory | `~/.claude/projects/-home-flo-programming-pcb-maker/memory/` (`goal-preferred-tool.md`, `topor-review.md`, `pcbench-benchmark.md`, ...) |
| Brief in the repo root | `pcb_pr_agent_brief.md` (untracked, not from this session) |

## Gotchas learned the hard way

- **Never run `cargo fmt`**: the repo is not rustfmt-clean and it churns
  unrelated files.
- **`pkill -f pattern`** kills its own shell. Use `[x]` in the pattern,
  or kill by PID.
- Commit before temporary debug edits: a `git checkout` of a file once
  lost work.
- **KiCad 10 specifics:**
  - pinfunction is `NAME_NUMBER`;
  - pad and text angles are absolute;
  - nets are by name;
  - DRC caps each violation type at 499 (at 199 for some types);
  - KiCad's silkscreen DRC sometimes names the wrong pad.
- **PCBench boards have no zones and no pinfunctions**, so plane, hot-part
  and swap measures cannot be tested there. Use the KiCad demos.
- Benchmark runs must use a copied snapshot binary: rebuilding
  `target/release` under a running benchmark mixes binaries.
- The machine has 8 cores. Two benchmark runs plus experiments at once
  make timing-dependent results (the move budgets, the quick sizing
  routes) unreliable.
