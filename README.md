# pcb-maker

**Automatic placement and routing for KiCad boards, judged by KiCad itself.**

`pcb-maker` takes a KiCad project, places the footprints that are free to
move, routes every net, and hands the board back to KiCad's own ERC and DRC
before it calls the job done. It is a research project, written in Rust, with
one goal: the best open-source PCB placer and router.

<p align="center">
  <img src="docs/images/routing-interf-u.gif" alt="Negotiated-congestion routing of the KiCad Interf-U demo board" width="720">
  <br>
  <sub>Routing the 110-net Interf-U demo from a bare board: every net is routed at once, conflicts are priced up, and only the losers move. The last frame is what KiCad accepted.</sub>
</p>

## Status

Not finished, but already useful.

- **Open-source boards.** On the 617 open-source boards of
  [PCBench / PCBWorld D3](benchmarks/pcbench/README.md), prepared the way
  PCBWorld prepares them, pcb-maker routes 601 (97 %) clean. PCBench's
  Freerouting run succeeded on 526 (85 %).
  - On the hardest tier (D3-C, 248 boards): 95 % against 75 %.
  - Vias are about half and copper 93 % of the designers' boards.
- **Agent tasks.** Boards reduced to what an agent starts from (parts
  stacked, no outline) are laid out from a `constraints.json`
  ([agent tasks](benchmarks/agent-tasks/README.md)).
  - On the 114 D3 test boards pcb-maker places and routes 110 (96 %)
    clean. The other 4 have no legal placement.
  - The median task takes 9 s. The layouts use 0.80 × the designers'
    copper and 0.52 × their vias.

On the [18-board corpus](docs/benchmarks.md) built from the KiCad demos,
Olimex and generated ESP32 boards:

- **Routing** (designer's placement kept, tracks and vias stripped): 12 of
  the 13 two-layer boards complete and DRC-clean. Of the four-layer boards,
  OpenAir is clean, ColdFire and the video board are a few nets short, and
  Tiny Tapeout (whose source already fails DRC) is well short. Head to head
  with Freerouting on the same cold boards: pcb-maker completes 12 of 13,
  Freerouting 8 of 13, with less copper on every board and fewer vias on
  four of the seven boards both finish.
- **Placement + routing** (every movable footprint re-placed from scratch):
  11 of the 13 two-layer boards clean, one more with a single thermal-relief
  finding; the automatically placed Interf-U routes with fewer vias than the
  human layout.
- **Not there yet.**
  - Placement: very dense two-sided boards with rotated parts can leave the
    placer without a legal solution.
  - Speed: large four-layer boards take 10–20 minutes.
  - Pours: big copper pours on both layers of a two-layer board are still
    the weak spot.
  - Features: no differential pairs or length matching; the placer does
    not choose sides by itself (constraints put parts on the back). Pin
    swapping exists but is opt-in. The
  hardest board in the corpus, a 186-footprint two-layer design with a
  0.75 mm BGA and a 3.3 V pour on both layers, is DRC-clean but stops at
  153 of 180 nets with the designer's placement and 161 with automatic
  placement.

Take a look at the [benchmarks](docs/benchmarks.md) before trusting any
number here; they are re-run and rewritten as the code changes.

## Try it

You need KiCad 9 or newer on the `PATH` (`kicad-cli` runs the final
verification) and either a [snapshot build](https://github.com/floitsch/pcb-maker/releases/tag/snapshot)
(rebuilt on every push to `main`) or a Rust toolchain:

```sh
cargo build --release          # target/release/pcb-maker
```

### From a schematic to fab files

```sh
pcb-maker import-kicad-netlist myproject/myboard.kicad_sch board myboard
pcb-maker layout-kicad-board board myboard layout auto board/layout.json
pcb-maker export-kicad-fab layout/result myboard fab
```

1. `import-kicad-netlist` makes the board KiCad's "Update PCB from
   Schematic" would: footprints from the libraries, on their nets, linked
   to their symbols. Next to it go a starter `constraints.json` (every part
   placed, the board sized from the parts, USB and other plug-in connectors
   on an edge) and the `layout.json` that uses it. Say where things go in
   `constraints.json` before the next step.
2. `layout-kicad-board` places and routes the board and has KiCad check
   it: `layout/result` is the finished project.
3. `export-kicad-fab` writes Gerbers, drill files, a BOM and a placement
   file in JLCPCB's format, and the estimated price at six fabs.

### Route a board

```sh
pcb-maker route-kicad-board <project-dir> <board-id> <out-dir> auto
```

`<board-id>` is the file stem (`myboard` for `myboard.kicad_pcb`), `auto`
takes track widths, clearances, via sizes and net classes from the project's
`.kicad_pro`. Existing tracks and vias are replaced; copper zones stay and
are used: pads are connected to their pour, islands are stitched, and if
that leaves something open the pour nets are routed as tracks instead.
`<out-dir>` must not exist yet; it receives a copy of the project with the
routed board, KiCad's ERC and DRC reports, and `board-router.json` with
per-net results. The exit code is non-zero if anything is unconnected or
violates a rule.

### Place and route

```sh
pcb-maker layout-kicad-board <project-dir> <board-id> <out-dir> auto [layout.json]
```

Everything that is not locked, has no nets, or sits on the board edge (a
connector) is placed anew, then routed. The result lands in `<out-dir>/result`
together with `placement.html`, an animated playback of the placement. A
small JSON keeps parts where the designer put them:

```json
{"placer": {"fixed_patterns": ["J*", "H*"], "fixed": ["U3"]}}
```

<p align="center">
  <img src="docs/images/placement-interf-u.gif" alt="Electrostatic global placement of the Interf-U board" width="640">
  <br>
  <sub>Placement of the same board: parts start on top of each other and spread out under net attraction and density repulsion, then are snapped to a legal, routable layout.</sub>
</p>

### Say where things go

For agents and people alike, `constraints.json` states placement intent:

- **Board shape.** The board size, or leave it out and it is estimated
  from the parts.
- **Edges.** Parts on an edge: flush, with a connector opening outwards, or
  an antenna reaching beyond it.
- **Placement.** Parts put in words ("front", "top-left", "center") or at
  an exact spot, regions, keepouts, rotations, fixed parts, and the side a
  part goes on.
- **Proximity.** Parts near other parts or near a given pad, groups kept
  together, and rows of parts at a pitch.
- **Shields and modules.** Parts inside a hollow part's outline, where
  only its pads block.

```json
{"version": 1,
 "outline": {"width": 60, "height": 40},
 "edge": [{"part": "J1", "edge": "left", "flush": true, "opening_outwards": true}],
 "near": [{"part": "C1", "pin_of": "U1:3", "max_mm": 2}]}
```

```sh
echo '{"placer": {"constraints": "constraints.json"}}' > layout.json
pcb-maker layout-kicad-board <project-dir> <board-id> <out-dir> auto layout.json
```

A ground plane is one line of router config, used in place of `auto`:
`{"add_pours": [{"net": "GND", "layers": ["B.Cu"]}]}`.

`export-kicad-fab` then writes the Gerbers, drill files, BOM and
placement file a fab takes, with the estimated price at common fabs.

The result reports every constraint as kept or missed, by how much. When
parts find no place, the error says why: how full each side is, or which
constraint holds the part. When connections stay open, `board-router.json` says which pads are unreachable
and where nets fought for room. See [docs/agents.md](docs/agents.md) and
[docs/constraints.md](docs/constraints.md).

### Placement only

```sh
pcb-maker place-kicad-board <project-dir> <board-id> <out-dir> [placer.json]
```

## How it works

**The router** ([docs/router.md](docs/router.md)) is a whole-board
negotiated-congestion router in the PathFinder family. The board is lowered
to a lattice whose pitch is chosen from the pads on it; every net is routed
against the same occupancy maps, overlapping other nets' clearance at a
price that rises from iteration to iteration, until nothing overlaps. On top
of that: a coarse tile graph plans corridors before the fine search runs,
fine-pitch pads get pre-computed escapes, copper pours are first-class
(thermal guards, plane connections, island stitching), and an attempt
ladder retries with a plane skeleton, with pours as tracks, and with finer
lattices when something stays open. Finished boards go through a via
reduction pass and an exact geometric verifier before KiCad sees them.

**The placer** ([docs/placer.md](docs/placer.md)) is an electrostatic
global placer in the ePlace/RePlAce tradition: footprints are charges,
their density field is solved on a grid, nets pull with a smooth wirelength
model, and every part keeps a halo sized to the tracks that must escape it.
Global placement is followed by annealing on rotations and positions and a
legalisation step. The **layout loop** ([docs/coupling.md](docs/coupling.md))
then routes, reads the congestion back into the halos, and tries footprint
moves that reduce open connections.

**Verification** is never skipped. Both the router and the placer keep their
own exact checks, and every board that leaves the tool has been through
native KiCad ERC, DRC and connectivity. `pcb-maker` reports what KiCad says,
not what it hopes.

## Benchmarks

Route mode on cold two-layer boards, one thread, same rules and same DRC for
every router (full table and method in [docs/benchmarks.md](docs/benchmarks.md)):

| Board | pcb-maker | Freerouting 2.2.4 |
| --- | --- | --- |
| Interf-U (110 nets) | complete, 30 vias, 4654 mm, 109 s | complete, 44 vias, 5051 mm, 44 s |
| PIC programmer (34) | complete, 1 via, 1911 mm, 4 s | 1 open, 0 vias, 2101 mm, 7 s |
| Complex hierarchy (50) | complete, 0 vias, 1330 mm, 1 s | 1 open, 0 vias, 1377 mm, 8 s |
| ESP32-C6 DUT (42) | complete, 26 vias, 1274 mm, 22 s | complete, 29 vias, 1355 mm, 14 s |
| Multichannel (79) | complete, 20 vias, 2578 mm, 75 s | 180 open, 16 s (likely an adapter setup problem, [being checked](docs/todo.md)) |
| StickHub (45) | 4 open, 53 vias, 13 s | 2 open, 44 vias, 66 s |

```sh
benchmarks/corpus/fetch.sh                      # downloads the KiCad demos
python3 benchmarks/corpus/run.py build/corpus   # route and layout for every board
```

## Repository

| Crate | What it is |
| --- | --- |
| `crates/pcb-router` | the format-independent routing core: geometry, lattice, negotiation, pours, verifier |
| `crates/pcb-placer` | the electrostatic placer, annealer, legaliser and playback |
| `crates/pcb-kicad` | KiCad S-expression parsing and writing, project rules, the board adapters, native verification |
| `src/main.rs` | the `pcb-maker` command line |
| `benchmarks/corpus` | the board corpus runner and the Freerouting / tscircuit comparisons |
| `benchmarks/pcbench` | 617 open-source boards (PCBench through PCBWorld's D3 preparation): fetch, manifest, runner, triage |
| `benchmarks/agent-tasks` | layout tasks as an agent poses them: stacked parts, constraints, a finished board |
| `docs/` | design notes, results and the trust audit that reset the project |

The other crates (`pcb-engine`, `pcb-grid-router`, `layout-trace-*`, …) are
earlier approaches kept for their tests and experiments; the
[trust audit](docs/reviews/2026-09-21-trust-audit.md) explains what was
kept and why.

## License

MIT, see [LICENSE](LICENSE).
