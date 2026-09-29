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
3. **Current goal: "implement the TopoR algorithm (at least as an
   option)."** It is in progress: see [The topological engine](#the-topological-engine-in-progress).
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

## The topological engine (in progress)

**Crate `crates/pcb-topo`, about 2000 lines, uncommitted until this
hand-over's commit.** Select it with `{"engine": "topological"}` in the
router config:

```sh
echo '{"engine": "topological"}' > topo.json
pcb-maker route-kicad-board benchmarks/real/external/kicad-ecc83-pp ecc83-pp out topo.json
PCB_TOPO_DEBUG=1 ...   # per-piece realization failures and the first violations
```

The optional `topological_rounds` sets the rip-up rounds (default 30).
`route_kicad_board` makes a single attempt with pour nets as tracks
(`board_router.rs`: the early branch plus `route_kicad_board_once`).

### Modules

- **`mesh.rs`.** One constrained Delaunay triangulation (`spade`) of every
  obstacle, shared by all layers:
  - circles and round ends become circumscribed octagons;
  - each face records the obstacles covering it and whether it is inside
    the outline;
  - `face_layers(face, net)` gives the layers a net may use there. Inside
    its own pad a net may go anywhere, the drill included.
- **`topo.rs`.** Wires are sequences of crossed edges, with an order per
  edge.
  - A* runs over the gaps between existing crossings. Its cost is length
    (through gap "portal" points) plus crossings × weight plus capacity
    overflow plus history.
  - Crossings are counted exactly, by chords interleaving around a
    triangle.
  - `crossing_pairs`, `overflowing` and `rip_up` are there.
  - Capacity merges all layers (edge length minus end clearances, times
    the number of free layers).
- **`layers.rs`.** Layer assignment on token chains:
  - tokens are terminals, faces and crossings; crossing partners must
    differ;
  - a layer change is a via, which is not allowed in a pad;
  - a local search moves runs of tokens between layers.
  - It is a heuristic, not TopoR's exact 2-layer max-cut.
- **`realize.rs`.** Geometry per layer:
  - a per-layer triangulation of that layer's obstacles and the vias,
    where circles and round ends are single points with a radius;
  - each piece's polyline is traced through it (`LineIntersectionIterator`)
    to get its channel, with back-and-forth crossings cancelled;
  - the funnel runs over portals shrunk by each vertex's radius: the
    obstacle's clearance plus half the width, plus the room of the other
    nets' pieces crossing closer to that vertex;
  - the result is exact tangents between discs and arcs, emitted as
    circumscribing polylines;
  - which wire owns each segment and via is recorded.
- **`lib.rs`.** The driver:
  - each net is a minimum spanning tree of pad-to-pad wires, routed widest
    first, then shortest;
  - each round assigns layers, realizes and runs `pcb_router::verify`, then
    rips up the offending wires (with history and oscillated weights);
  - the best round is kept, and violating wires are dropped until the
    verifier is satisfied. **So what it emits is always legal, possibly
    with opens.**

### Where it stands

On ECC83 (9 nets, 20 wires) the whole pipeline runs in 0.3 s. The copper
is legal: no internal violation and no KiCad error. **But only 9 of 20
wires survive.**
- **5 wires never routed.** A THT pad's centre lies inside its drill hole,
  which blocked the pad's own net. `face_layers` has just been fixed to
  allow it; this is untested.
- **Most realizations fail with "no tangent".** Consecutive discs of the
  funnel output have the same centre: an apex at the piece's own start or
  end vertex (the pad centre is a per-layer vertex), or the same vertex
  twice in a row. This is the **next fix**:
  1. drop apexes that coincide with the start or end point;
  2. merge consecutive apexes on the same vertex and side, keeping the
     largest radius;
  3. then look again at the "actual -0.8" overlaps (a wire through a pad),
     which may be a side error in the funnel.

  Unit tests for `tangent`, `arc` and `tighten` on hand-made channels
  would pay for themselves.

### Still missing for a faithful, useful TopoR

In rough order:
1. Correct realization (above), then measure it on the corpus against the
   lattice engine.
2. **Steiner/T-junctions.** Wires may attach to their net's existing wires,
   not only pad to pad. Today power nets make long spanning trees.
3. **Via placement.** Choose a spot in the face with room, check it
   against obstacles and other vias, and move vias (TopoR's "nudging").
4. **Per-layer capacity** during routing, instead of all layers merged.
5. **Exact 2-layer layer assignment** by planar max-cut (Chen, Kajitani &
   Chan 1983; Barahona 1988), as TopoR claims to do.
6. **Variant archive** on the (length, vias) convex hull; randomized
   restarts from archived variants.
7. **Native KiCad arcs**, which need `Segment` or `NetRoute` to carry arcs
   and `verify` to handle them. Today they are polylines.
8. **Pours** (connect to planes), and the engine in `layout-kicad-board`
   (moves that keep topology: the big speed-up).
9. Performance: `Mesh::locate` is a linear scan, and path search copies
   are small but unprofiled.

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
