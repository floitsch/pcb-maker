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
(`pcb-maker-x19`, runner timeout 1500 s so the 1200 s ladder can finish).
Near misses of v2 rerouted with x19 (`build/near.sh`): MIDAS-MK2,
zpn_devboard, coco2, little-red-rover, Castor_and_Pollux, rosco_m68k all
complete now; the last two keep 6 and 5 `starved_thermal` findings beyond
the designer's (pour-net pads joined by tracks whose thermal spokes other
copper blocks). Vias now pay the thermal guard too: Castor_and_Pollux is
clean. rosco_m68k's starved pads reach only pour islands that signal
tracks cut off. KiCad ignores spokes into an island that connects to one
item only (drc_test_provider_zone_connections.cpp), but stitching such
islands with vias was tried and reverted: on Castor_and_Pollux it doubled
the starved thermals (3 -> 6, the designer's count is 3) and added a
hole_to_hole error, as the vias cut the fill; rosco's pours are one layer
per net, so there is nothing to stitch to anyway. The way out there is
keeping signals off the plane layer near those pads (or the exclusive
rung completing). Of the 42
boards open in v2, jiran-ble-lite is limited by keepouts its designer
violates (`items_not_allowed` 40 in their DRC).

## 2026-10-03, later: probe ladder, hole clearance

Three fixed ways of sharing the ladder's budget each failed some large
board; the probe ladder (all rungs negotiate 75 s, the best continues)
beats all of them on the laptop motherboard, zpn_devboard and MIDAS-MK2.
A systematic KiCad finding our verifier missed: copper too close to via
drills (hole clearance); now in the stamps, the statics and the verifier.
Island stitching for starved thermals was tried and reverted (worse).
Then: budgets in work (search expansions), not seconds, so results no
longer depend on machine load (identical runs verified); probes count
stranded pour pads; resume keeps the probe's price of sharing. And a
reporting bug: `summary.py` never counted DRC errors, so the v2 table
said 25 clean where 17 were (fixed, table corrected). Sweep restarted as
`build/github-route-v9` (`pcb-maker-x30`, 2700 s per board: work budgets
take longer in wall time on a busy machine). Layout sweep: still to rerun
with `--jobs 1`.
The 11 v2 boards with copper errors beyond the designer's, rechecked on
the newest binary: 7 of them clean now (hole clearance, KiCad 6/7 net
classes, and custom-pad arcs swept to cover KiCad's outside arc
approximation). Starved thermals remain the one class: 0xCB-1337 6
(designer 3, USB-C GND pads), ErgoSNM 5 (designer 2), plus the boards
noted above. Later: text on a solder mask layer is an opening; routed
copper now stays out from under it (eurorack-pmod: 21 solder_mask_bridge
errors -> none). Tried and reverted: retrying the best rung with the
board's smallest legal via (eurorack's corner pad still had no room under
the hole clearance; 4 open instead of 2).

## 2026-10-04

Fixed: items on undefined layers (koeg-board), loops outside the main
outline are further board pieces (Multi-Protocol Hub: ~340 open -> 111),
solder mask openings (eurorack), custom-pad arcs (capybully). New:
`benchmarks/github/guide.py` makes guided copies (each net at its
designer's width, like PCBench's guide) because designers often route far
narrower than their class (ohdsp DSP: 160/190 at the class width, 190/190
guided); running in the background (`build/github-guide.log`), then sweep
`boards-guided.json`. The unguided sweep `build/github-route-v9` keeps
running. todo.md: neck down where the class width does not fit.
Then: the ladder's last step routes signal nets at the project's smallest
predefined width when connections stay open (DSP board 158 -> 189 of 190
unguided). Then pads join thermal pours through KiCad-like spokes, solid
pours by touch (DSP board 15 -> 6 unconnected; MIDAS unchanged), and a
pad's own zone_connect overrides its pour's (18 boards set pads to no
connection). Sweep restarted as `build/github-route-v12` (`pcb-maker-x43`);
v9's first 47 boards are the comparison.

v12 against v9: OpenFC 1 -> 9 open, ESC-mini 4 -> 16. Bisected
(tracks-only, deterministic: `build/topo-set/tracks-bisect.json`) to the
solder mask keepouts. Two lowering bugs: a filled mask rectangle's bare
edges were openings of their own, saw no pad and fenced the pad off from
its own net (OpenFC's hand-drawn QFN openings; now one net per graphic,
OpenFC 81/82 again), and mask text was its estimated box, ignoring
vertical justification (OpenESC's "ESC" label sat over U11's pad row; now
TrueType text uses its `render_cache` polygons, and text counts only over
a pour, whose fill shows through: eurorack's logos on GND keep 0 bridges).
Then plated holes of routed pads are hole obstacles of their net:
the 0.25 hole clearance reaches past a thin annular ring (eurorack U1's
thermal vias). Tracks-only on the stripped boards, x43 -> x49: ESC-mini
136 -> 150 of 152, OpenFC 73 -> 81 of 82. The v12 sweep is frozen at x43
and keeps running.
Caution: `build/near.sh` used to route `build/github-route-v2/<board>/source`,
which still holds the designer's tracks (ESC-mini: 1194 segments, 1130
vias), so its numbers before 2026-10-04 evening are not comparable to the
sweeps. It now strips tracks first, as `benchmarks/corpus/run.py` does.

## 2026-10-05

Committed, in order:
- Mask openings: footprint mask graphics count; an opening exposes the
  pads under it, or else a pour's fill; it belongs to that one net, keeps
  all others out, and over several nets keeps everything out while pads
  inside leave it by their stubs (Sisu's zebra connector J_LCD1). Filled
  rectangles without stroke are just their fill. Sisu: mask bridges beyond
  the designer 51 -> 3.
- `strip-kicad-tracks` moves items on undefined layers (koeg-board's
  Rescue layer), so the baseline DRC loads.
- Benchmarks count error-severity findings only (KiCad passes warnings;
  the designer's reference was always read that way). `summary.py
  <run-dir>` recounts old runs from their saved reports.
- Escape stubs pass an exact clearance fit (0.4 pitch QFN with 0.2 pads
  and 0.2 tracks: RP2040 motor controller tracks probe 111 -> 142 of 186).
- A wall-clock deadline: router `Config.deadline`, route config
  `deadline_seconds`, and the layout puts all its routes under
  `total_seconds`. The layout benchmark had killed every large board at
  1800 s with no board; jetson-nano now finishes in 1575 s.
- kicad-cli reports time out (`PCB_KICAD_TIMEOUT`, 900 s) and die with the
  router (PR_SET_PDEATHSIG): 0xCB's panel refill hangs forever, and
  orphans held 4.4 GB for up to 21 h.
- Router: `diagonal_block_via` used the wrong radius between classes
  (0.145 of 0.15 next to vias of a narrower class).
- KiCad 9 graphics on several layers with a net (Sisu's GND shapes on
  F.Cu and F.Mask) are that net's copper; they were ignored (9 shorts).

- Overlapping pads of one number are one terminal (OpenRX's panel); a
  through-hole pad's terminal pointed at its drill since b6e3530 (fixed).
- `summary.py` shows the designer's own unconnected items (OpenRX's panel
  has 105: a panel, not a fair routing target).
- Sisu with all of the above (x56, stripped, full ladder): 177/192 routed,
  0 internal violations, nothing beyond the designer but 3
  pth_inside_courtyard (placement); v12 had 185/192 with 27 mask bridges,
  6 clearance and 5 shorting findings.
- Unfilled circles are rings (custom pads and graphics): Sisu's dome
  switches and speaker contacts drew the outer contact as a ring that was
  lowered as a disc burying the inner pad. Sisu x58: 36 unconnected in
  KiCad (x56: 48), keypad nets all connected; left open: GND 12, +3.3V 7
  (tracks mode), seven single connections. Its first attempt alone takes
  ~1900 s on a loaded machine: its continued probe sits at 27-32
  conflicted nets from iteration 6 to 38 (~650 s at the price cap) because
  a new low by one net resets the stall counter. A stall rule that needs
  a real drop (10 %, or two nets) would save that, but must first be
  measured on the corpus: some boards finish only in that tail.
- Projects that rate clearance as warning/ignore (6 of 109) get a last
  ladder step at the board's min_clearance: the A13 module's designer kept
  0.149 against classes of 0.2 (its project made clearance a warning).
  It joins the narrow-signal step (one attempt). A13 module x60: 219/223
  routed (v12: 195), 14 unconnected; the last step took 119 -> 33 open.
  Left: SW1's own pads at the edge (placement) and one KiCad teardrop
  reaching into a keepout. The run needs ~3000 s on a loaded machine.
- Placement has a wall-clock deadline too (40 % of the layout budget);
  OpenESC's 404-part panel placed for >25 min before. Layout prints its
  phases on stderr.
- Agent tasks (`from_pcbench.py --corpus`): parts over the board's own
  cutouts stay fixed (Sisu's J8). Regenerated into `build/github-tasks-v2`
  for the next sweep (5 tasks differ). Open: edge constraints on boards
  of several pieces (zpn_devboard: two boards side by side) are measured
  against the union's box; per-piece edges are needed.

- Per-layer padstacks lower to the union of their layers' shapes
  (OpenESC 30x30 panel errored; its designer's panel has 130 unconnected).

- Agent tasks: hollow parts (anything may enter) only where the project
  lets courtyards overlap; otherwise the designer's contained parts stay
  (link's U1 got 18 extra overlaps). `build/github-tasks-v3`, 72 tasks
  differ (mostly switches with their diodes).
- Done (x64): thermal reliefs keep their gap around the pads in the pour
  model, and stitching vias keep hole-to-hole from the net's vias.
  PolyKybd split72 right: 55 -> 5 unconnected, 0 zone islands, nothing
  beyond the designer; split72 left 66 -> 5, corne right 57 -> 6 (v12 also
  had 17 starved thermals there), both nothing beyond the designer. Diagnostics with KiCad's own fill (pcbnew Python
  works; under Python 3.14 index containers instead of iterating):
  benchmarks/github/kicad_components.py <board> <net>.
- Starved thermals on x64 (stripped, full ladder): mackerel-30-proto 8 ->
  0 (217/217, clean), video 4 -> 0 (371/371, clean), BAGEL-MK1 1 (J102's
  pad has 4 spokes into an island the model thought joined).
- Starved thermals (ohdsp DSP, 5 beyond the designer): KiCad counts spokes
  whatever tracks join the pad (U201 pad 1 has six GNDD tracks and is
  still flagged at 1 of 2 spokes). Other nets' tracks take the second
  spoke's path (DVDD past pins 19 and 54); on fine-pitch QFN pins only the
  outward spoke may fit at all. Options: keep spoke corridors clear (a
  hard block where the thermal guard is only a cost), or count spokes in
  the pour model against min_resolved_spokes and report the pads that
  cannot get enough, so the agent decides on solid connection.
- Was the next completeness target, measured on v12: KiCad finds GND fill islands
  that our pour model counts as joined ("Zone"-"Zone" unconnected items):
  PolyKybd split72 left 42, right 39, corne right 31, left 8 (most of their
  unconnected), OpenRX panel 30, A13 6. Our model splits GND into 683-1514
  pieces and reports it stitched and complete. Needs KiCad's fill to
  compare (gerber export with zone refill), or a feedback pass that joins
  the islands KiCad reports (the ratsnest endpoints are on each island).
  Edge constraints on multi-piece boards need per-piece bounds in the
  placer (constraints.rs uses the union's box).

- Placement fixes found by layout v6 (x65): custom pads count their
  primitives in the body (Sisu's dome rings, OpenFC's J52), and board
  copper graphics on several layers block placement. Layout tasks (tasks
  v3): OpenFC passes, 82/82 clean in 636 s (v6: 81/82, 2 shorts); Sisu
  162/192 with no shorts, clearance or mask findings (v6: 173/192 with 17
  shorts, 17 clearance, 24 bridges), 6 starved thermals left.

- Thermal guard on spoke corridors only (cost 10, was a disc at 4): DSP
  starved 5 -> 2, BAGEL-MK1 1 -> 0 (138/138), PolyKybd split72 right
  unchanged (5 unconnected, clean); Sisu layout task (x66) 180/192 (x65
  162), starved 6 -> 5, but 4 mask bridges.

- Placer: the board's mask graphics and mask text block placement on
  their side; tasks v4 (`build/github-tasks-v4`) fix parts under the
  board's own mask openings (13 tasks differ). Router: only mask openings
  let a pad leave by its stub through them (a rule area stays strict).
  Sisu layout task on x68: 172/192, 90 unconnected (x65: 125), only 3
  starved thermals and 1 pth_inside_courtyard left. OpenFC on tasks v4:
  81/82 (one +3.3V terminal, congestion; it passed on tasks v3 where U9
  could move).

- ATAT1800 routing on x68: 35 unconnected (v12 57), 1 starved, no
  dangling tracks; it did not finish the ladder within near.sh's hour on
  a loaded machine (speed).

- stall_drop measured (x69, `{"stall_drop": 0.1, "stall_at_cap": 12}`
  against the default): PolyKybd split72 right 3037 -> 3003 s, the same
  board; Sisu 2700 -> 2314 s (-14 %) but 180 -> 177 of 192 routed (KiCad
  32 unconnected both). Completeness first: it stays opt-in. Sisu's
  default on x69 is its best plain routing yet: 180/192, 32 unconnected,
  nothing beyond the designer but pth_inside_courtyard (placement).

- KiCad joins a custom pad only through its anchor (its connectivity
  ignores the primitives even where its hit test says inside): terminals
  carry a contact shape. Sisu (x70): 181/192, KiCad 19 unconnected (x69:
  32; the keypad rows' 11 are gone), the router's 17 open terminals now
  close to KiCad's count.

- Layout: no final polish after the deadline (it threw the moves' best
  board away: OpenAirScope 108/184 -> 177/184, no errors), and the
  in-place router stops at a stall sooner (stall_drop 0.1, patience 10;
  OpenAirScope first route 606 -> 439 s, 12 unconnected; MIDAS-MK2.1
  passes 154/154 in 529 s). Moves still take 200-290 s each on
  OpenAirScope: the next layout speed target. PCB_ROUTER_TIMING shows a
  move's 8-12 nets negotiated ~34 iterations at 5-8 s each (finish 3-22
  s); skipping nets already open before the move changed nothing (they
  were not pending). Faster price growth in the in-place router (2.0):
  first route 439 -> 313 s (18 open, was 22), moves 126-229 s, same final
  quality. Next candidate: windows bounded around the moved part.

- Tasks v5 (`build/github-tasks-v5`): a part holding up to three others
  keeps them where the designer put them. katia still fails: its back
  side controllers and connectors (U1, U2, J3, J6) sit half under the
  through-hole switches; "fix what overlaps a fixed part" froze whole
  boards under frame parts (reverted). Open: placement that may overlap
  courtyards where the project allows it, part by part. (Through-hole
  parts already block the far side only with their pins (`far_side`);
  katia's conflicts include its 220-piece outline's cutouts.) With
  the placer now reaching every level quickly (437 s), katia's four parts
  fail even with tight bodies: they collide with the switches' far-side
  pins and its fixed parts; whether those pin boxes are too generous
  (pad extent plus keep-away) is the next thing to check there. Checked
  with pcbnew: at the designer's spots U1 and J3 overlap no through-hole
  pad, so the pin boxes are not the cause; left: other moved parts taking
  those spots, or the copper-to-cutout margin ("cutout" is in the illegal
  list; the outline has 220 pieces). The cutout margin counted twice (fixed);
  katia still fails, now against its fixed switches: hot-swap switches
  carry their socket pads and courtyard on the back, and the designer put
  U1/J3 inside those courtyards (allowed by the project). That is the
  courtyard-overlap placement feature. Checked with pcbnew: at the
  designer's spots U1, U2 and J6 overlap no back-side copper (J3 touches
  SW13's socket pad box), so legal spots exist; the legalizer, starting
  from the global placement of a stacked task, does not find them on a
  back side our estimate puts at 139 %. Eviction (a failed part takes the spot that
  displaces least movable area; the displaced find new spots) seats U1
  and U2; J3 and J6 (through-hole connectors) still find no room. Their
  designer spots partly overlap switch hot-swap socket pads on the back
  (J3 over SW13's): allowed by the project's courtyard rule, but sockets
  are bodies, so letting the placer do the same (far-side pads tested
  against pads instead of bodies at the tight level) is a design decision
  to make with Florian, not an obvious fix. Florian: make it an option.
  Done (placer.overlap_far_side_pads, off by default). On katia it did not
  seat J3/J6 (3 unplaced in that run, levels slower). Next suspect: the
  placer models a cutout by its bounding box, and a long diagonal edge
  piece near J3 (129.7-139.05 x 160.9-177.2) may belong to a cutout loop
  whose box covers J3's designer spot; cutouts as polygons would tell.
  Confirmed and fixed: cutouts keep copper away by their own outline
  (Component.cutout_outline). katia places completely in 58 s, layout
  140/162 routed.
- MegaDrive layout on x87: hole_to_hole 34 -> 0 (stitch spacing fix),
  266/298, 150 unconnected (v6: 272/298, 167).

- The 13 boards of the Freerouting/tscircuit comparison (route mode,
  pours kept, x75, machine loaded by the layout sweep): all 13 complete
  and clean, StickHub included (45/45; September: 43/45, the one board
  Freerouting did better on). Routing seconds: ecc83 0.3, hierarchy 2,
  pic 16, interf-u 83 (wall 448: four rungs, a seed retry of 133 s for
  one open connection, then the skeleton rung completes it, then an
  extra tracks rung), olimex-c3 28, dut-c3 23, dut-c6 32, dut-s2 44,
  dut-s3 44, dut-esp32 32, sonde 2, multichannel 48, stickhub 38.
  interf-u keeps 84 vias where the tracks rung had 30 (3 starved).
  Seed retries deferred to the best rung (x76): same 13 results, wall
  interf-u 448 -> 320 s, olimex-c3 129 -> 75 s, stickhub 145 -> 92 s.
  Where the rest goes on mid-size boards: via reduction (dut-s3: 38.5 of
  44.5 s; negotiation 13 s). Rounds after a reverted round are kept 15
  times of 34 (91 reduction runs in x76, v9 and v12 logs), so stopping
  at the first revert would cost vias. Like for like with Freerouting
  (whose optimizer was off in the September comparison) is
  via_reduction_rounds 0: 3-10x faster than both other routers then.

- Netted filled board graphics are terminals (Sisu's /RF/ANT feed): Sisu
  x77 186/192, KiCad 12 unconnected (x70: 181, 19).
- Fixed: two pads of a net shared one escape node (the per-net escape map
  is keyed by node; the later stub overwrote the earlier): Sisu's U9 pad
  11 looked joined to the router and was not. Sisu x78: 186/192, KiCad 11
  unconnected, nothing beyond the designer but pth_inside_courtyard.

- A13 module: router and KiCad agree (GND 14, +1V5 3, two DDR nets open).
  GND's open pads are DDR3 balls whose fill pieces are islands the
  stranded-pad reroute cannot join (18-24 per attempt, 496-542 pieces):
  the BGA fan-out Florian suggested (dogbone vias from pour balls to the
  inner plane, where In1's GND pour lies under the chip) is the next
  completeness feature for boards like this. `fix_plane_stubs` is that
  fan-out already: tried in the final narrow+relaxed step (experiment),
  it joins GND's balls 170 of 172, +1V5 43/46, +3V3 31/31, but the fixed
  stubs take the DDR signals' channels: 158 open instead of 33 (192/223
  vs 219/223). Fan-out has to leave the signal escapes their way out:
  stubs placed with the signals, not before them. Tried that too (stubs
  left unfixed, ordinary branches): 127 open, still far behind 33 with
  pours as tracks and no stubs. On A13 the plane approach loses; its open
  GND balls need something else (a ball-by-ball dogbone search with the
  signals already routed?). Pours connected in that step (no
  stubs): 122 open. The default on x82: 221/223, KiCad 9 unconnected
  (x60: 219/223, 14), final step 18 open (was 33).

- jetson-nano routing on x82: 338/340, KiCad 2 unconnected, nothing
  beyond the designer (its hole_clearance findings are J2's own pins).

- Layout v6 at 65/109: 13 pass (new: PixelWave 343/343, SmartSpin2k
  panel 585/585 in 187 s, MotionCube). PolyKybd "molecule" boards are
  panels of one keyboard key (716 footprints, duplicate references, the
  designer's own board 266 unconnected): no fair target, like OpenRX's
  and OpenESC's panels.

- Starved thermals: diagnostics list each pad with advice; the opt-in
  `solid_starved_thermals` connects them solid and verifies again (ohdsp
  DSP: 6 pads, 0 starved left, nothing beyond the designer). Default off:
  thermal reliefs are for soldering, the designer's call.

- OpenAirScope routing on x83 (designer placement): 178/184, KiCad 6
  unconnected (signals around the SD lines), nothing beyond the designer;
  GND stitches complete in every pour rung. Its GND islands in layout
  come from the automatic placement, not the pour model.

- Layout v6 at 68/109, 13 pass. nonSNES SNSP-CPU-01 had 166 mask bridges
  in v6 (x62: parts placed under the board's F.Mask rectangles); on x86
  with tasks v5: 5 bridges, 295/363 routed (v6 272), 278 unconnected (416).
  The 5 left: large mask openings over a pad and pour fill (J2's polygon;
  the opening counts the pour only without pads under it, so the pad
  net's via goes in and bridges to the fill), and one copper text placed
  too near the edge by the layout's copper-text placement. Counting the pour as exposed when an opening is over
  four times its pads' area changed nothing there (reverted): the cause
  is elsewhere (the pour's centre test, or the polygon's shape). pcbnew: J2's large F.Mask polygon has a notch
  around J2's pads, and the J2-Audio via KiCad flags lies outside it (its
  hit test says so): the bridge is likely the via's own mask opening
  (untented vias) touching the polygon's opening. Vias then need the mask
  expansion as clearance from mask openings.

Layout v6 stopped at 76/109 (14 pass) on x62: stale after the placement
fixes (mask graphics, exact cutouts, eviction, custom-pad bodies, in-place
router stall and price growth, polish guard). Layout v7 started on x89
with tasks v5 (`build/github-layout-v7`, `--jobs 1`).

Queued: `build/chain-v13.sh` waits for layout v7 to finish, then runs the
routing sweep `build/github-route-v13` (`pcb-maker-x89`, all 109 boards,
timeout 2700 s; log `build/github-route-v13.log`). Summarise it with
`benchmarks/github/summary.py build/github-route-v13` and compare to v12.

Running: layout sweep `build/github-layout-v6` (`pcb-maker-x62`, tasks
`build/github-tasks-v2`, `--jobs 1`; requested by Florian; v3 stopped after
four 1800 s timeouts, v4 after the terminal bug, v5 after 9 tasks for the
placer deadline, padstacks, rings and relaxed clearance). Let v6 finish
untouched. Routing sweep `build/github-route-v12` (x43) stopped at 63 of
109 boards so the layout sweep has the machine (its deadlines are wall
clock): 18 clean, 34 open, 5 drc, 2 mismatch, 4 error (`summary.py`,
errors only). Next routing sweep: the current binary, after layout v6.
v5's first rows (x57): jetson-nano 336/340, laptop 230/236, MokyaLora
259/262 placed and routed within ~1550 s each.
krishveercard is no benchmark for the class rules: the designer's own
board breaks them hundreds of times (use the guided set).

## 2026-10-06

- Budgets count work, not the wall clock (Florian: "less dependent on
  the actual hw"). The layout keeps a work clock in seconds of an idle
  machine: placement work (a thread-local count in pcb-placer: legality
  checks weighted by outline edges and parts visited, anneal moves by
  parts, pins and outline, global iterations by the field solve) at
  `pcb_placer::WORK_PER_SECOND` per seed thread, plus search expansions at
  `EXPANSIONS_PER_SECOND`. The placement share (40 %), the race cut-off,
  the move phase (65 %) and the final ladder's share come from it. The
  wall clock stays as a guard at `WALL_GUARD` (1.15) times the budget so
  a much busier machine still ends before the harness timeout.
  `KiCadBoardLayoutResult.work_seconds` reports it; `PCB_PLACER_DEBUG`
  prints work per level and the anneal's problem statistics.
- Polygons with `arc` entries in `pts` (KiCad 7 on) lost their arcs
  everywhere we read polygons: link's board-level F.Mask polygons (only
  arcs) vanished, parts went under them and layout v7 had 103 mask
  bridges there. `outline::pts_points` flattens arcs; every polygon
  reader uses it (SNSP has no arc polygons: its J2 bridges are another
  cause).
- SNSP's remaining mask bridges (J2's notched polygon and the board's
  large F.Mask polygon) were vias 0.005-0.05 mm inside the polygons'
  strokes (0.1 and 0.15 mm wide): polygons lowered as their bare outline.
  They now lower with their stroke, and unfilled ones as outline only.
- Layout v7's first rows spent their time in the placement race: boards
  with every part fixed raced one placement against itself (jetson-nano
  [38, 38], MokyaLora [24, 24, 24], laptop [44, 44]: 3-6 minutes each), and
  link's second placement negotiated 1000 s, so no move and no ladder
  followed. Alike placements now route once; others race with at most the
  first route's search expansions.
- Mask text over bare board was skipped (an OpenESC workaround from when
  text was a badly justified box; OpenESC's label is TrueType and lowers
  to its glyphs now). jetson-nano's B.Mask title then had three nets'
  vias and tracks under it (5 bridges). Text over bare board is a keepout
  for all nets again (ESC-mini routes the same, 149/152; jetson-nano
  checked with x92).
- ESC-mini lost a part since d03a49e (levels after a few failures only
  tried the kept placement; bisected x83 good, x84 bad). Levels with a
  finer grid or tight bodies anneal again. Levels whose bodies and spacing
  need more area than a side has are skipped (OpenESC's boards fit only
  with tight bodies). ESC 30x30 places completely in 740 s (was 3 parts
  short after 1301 s).
- Layout v7 (x89) stopped at 13 rows, stale after these fixes: MokyaLora
  262/262 (clean once exclusions count), laptop 231/236, jetson 333/340,
  link 175/251, Sisu 157/192, OpenRX panel 48/127, ESC boards unplaced.

- Legalization looked at every placed part and every outline edge for
  each of up to 90 000 spots on the fine grid. It now asks a bucket grid
  for the parts near a spot and tests the outline last: OpenESC 30x30
  places in 205 s (740 s), ESC mini in 65 s.
- jetson-nano with mask text kept clear (x92): 338/340, KiCad 2
  unconnected, no bridges, nothing beyond the designer.

- The anneal's overlap sum uses the same bucket grid (`buckets.rs`),
  kept with every move: katia places in 25 s (61), link 51 s (124),
  OpenESC 30x30 151 s. Exact (checked with `PCB_PLACER_CHECK_OVERLAP`),
  and over four seeds no worse: link 6144 mm against 6315, ESC 30x30
  ~630 against 669, ESC mini 444 against 440. Seeds differ by up to 15 %
  on ESC 30x30: compare placements over several seeds (`seeds.sh` in the
  session scratchpad sets `placer.seed`).
- v8's first row on x96: jetson-nano 337/340, 3 unconnected, no errors
  (v7: 333, 9, 5 bridges), but 1741 s: on the loaded machine a work
  second is about two wall seconds and the wall guard (then 1.15) ended
  it 59 s before the benchmark's limit. The guard is 1.1 now.

- The global placement's field solve adds rows to rows (vectorized):
  katia's global placement 1.7 s (6), same placements.
- A kept placement that seats every part on a new grid or with tight
  bodies no longer ends the levels: a fresh anneal runs there and the
  shorter placement wins (ESC 30x30 over four seeds: mean 619 mm, was
  669; one seed had kept 902 mm).

- Sisu's layout (157/192 in v7; 186/192 on the designer's placement):
  the task's `source` board has the movable parts piled at one spot, so
  `wirelength_source_mm` is no baseline; compare with
  `build/github-route-v2/<board>/reference`. Our total pad wirelength is
  below the designer's (4500 against 5038 mm), but local nets the
  designer keeps short were long (feedback divider 46 mm, RF feed 52 mm).
  More anneal effort or whitespace fill changed little (within 3 %).
  Two changes: refine also searches free spots within 3 mm of a part's
  target (katia 6080 mm against 6714 over four seeds, link -0.6 %, ESC
  mini -3 %), and nets named for a switch node, feedback, bootstrap, RF,
  antenna or crystal weigh three times (Sisu's six: 72 mm against 114;
  designer 33). Neither is in v8's binary.

- link in v8: 165/251 and 26 mask bridges, all with one F.Mask polygon
  and listed against GND items. KiCad names the items whose net differs
  from the first it found in the opening: that was a J9 track 0.025 mm
  from the polygon. KiCad drops mask slivers under the minimum web width
  (link: 0.1 mm), so copper within half of it counts as exposed (deleting
  the track: 0 bridges; moved to 0.07 mm: 0). Mask openings now keep
  copper `solder_mask_min_width / 2 + 0.01` away (a keepout's clearance
  was ignored before).
- Router search: one record per A* state, heap keys as one word, the
  layer's maps hoisted: 15 % faster on ESC mini's first 20 iterations
  (same routes). `stall_patience` (8 in layout trials). OpenAirScope
  layout at 900 s: 8 trials (6), 10 open (12).

- link's layout failures are supply nets (GND 142 items, the 3V3A nets
  112, +3V3 22 of 264). Racing pours as tracks against pours as planes in
  the layout's first route (same search cap) lost on link (881 against
  396 open): tracks need far more search. Reverted. Route mode on the
  designer's placement: 19 open (pours as tracks, one 2389 s attempt).
  Route mode's probes on our placement look alike (tracks 205 nets
  unfinished against 160 on the designer's), so the placement is not the
  main gap: link needs more time than the layout's 1500 s. Ranked by
  unfinished nets the race picks tracks (120 against 251) but ends worse
  in budget (KiCad 499 unconnected against ~265 with planes). Reverted
  too. With pours as tracks link spends most search in open searches
  (iteration 7: 305M of 835M expansions in 182 of 6335 searches): the
  next speed target.
- v8 rows on x100 (`build/github-layout-v8-x100`): jetson 337/340 (3
  unconnected), link 165/251 (26 bridges, fixed since), laptop 231/236
  (first route kept; the final ladder's tracks attempt ended at 789 open),
  OpenRX panel 49/127, ESC 30x30 panel.

Running: `build/chain-v8.sh`: layout sweep `build/github-layout-v8`
(`pcb-maker-x106` = 7be808d, tasks v5, `--jobs 1`), then routing sweep
`build/github-route-v13` (x106, timeout 2700 s). Leave it running. Compare v8 with v7's
first rows and v6; v13 with v12. (`build/github-layout-v8-x96` holds
the x96 jetson row.)
- MokyaLora in layout v7 routed 262/262 and failed only on two courtyard
  overlaps the designer excluded in the project (`drc_exclusions`,
  `"excluded": true` in KiCad's report). The benchmark runners now skip
  excluded violations, as the designer's baseline (run with
  `--severity-error`) always did.

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
