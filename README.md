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

- **Boards harvested from GitHub** (109 finished KiCad designs, 2 to 8
  layers, 44 with four or more; [the harvest](benchmarks/github/README.md)).
  Reduced to what an agent starts from - parts stacked, the designer's
  placement gone, a `constraints.json` of fixed connectors and edges -
  pcb-maker places and routes **31 of 109** to a clean KiCad verdict
  (0 unconnected, no copper error beyond the designer's own board) within
  30 minutes each, every board placed from stacked parts. Most of the
  rest are complete but for a few connections or a few starved thermal
  reliefs; a handful are panels or boards whose rules no track can satisfy
  ([results](docs/benchmarks.md#layout-sweep-v13-2026-10-10-every-board-placed-from-stacked-parts)).
- **Against Freerouting** on the same 109 boards with only the copper
  removed (the designer's placement kept, 1500 s each): Freerouting finishes
  70 and times out on 34; where it finishes, pcb-maker leaves fewer
  connections open on 49 boards and Freerouting on 6, with 50 boards clean
  against 8
  ([table](docs/benchmarks.md#freerouting-against-our-cold-route-on-the-harvested-boards-2026-10-09)).
- **Open-source boards, PCBench / PCBWorld D3** (617 boards, prepared the
  way PCBWorld prepares them): pcb-maker routes 601 (97 %) clean,
  PCBench's Freerouting run 526 (85 %); vias about half and copper 93 % of
  the designers' boards. On the 114 D3 agent tasks 110 are laid out clean,
  median 9 s.
- **Not there yet.**
  - Pours: our fill model is more fragmented than KiCad's, which shows as
    starved thermal reliefs on dense boards and as pads routed by tracks
    that KiCad would have joined through the fill.
  - Placement: crowded boards (ESC-class four-in-one controllers, a dense
    SNES motherboard) stay 30-80 connections short; the legaliser gives
    back a large part of the global placement's quality.
  - Features: no differential pairs or length matching; the placer does
    not choose sides by itself (constraints put parts on the back); pin
    swapping is opt-in.

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

Everything is measured by KiCad's own DRC and connectivity on the result,
with the designer's own findings subtracted. The harvested GitHub boards are
the main corpus ([docs/benchmarks.md](docs/benchmarks.md)):

| Sweep | Boards | pcb-maker | Freerouting |
| --- | --- | --- | --- |
| Layout from stacked parts and constraints, 1800 s | 109 | 31 clean (v13), 3586 connections open in all | - |
| Routing the designer's placement cold, 1500 s | 109 | 50 clean, 206 connections open in all | 8 clean, 70 finished, 34 timed out, 913 open where finished |

```sh
python3 benchmarks/github/harvest.py fetch                      # the harvested boards
python3 benchmarks/agent-tasks/run.py build/layout --tasks build/github-tasks-v5/tasks.json
python3 benchmarks/github/freerouting.py build/freerouting --ours   # both routers, headless
```

## Repository

| Crate | What it is |
| --- | --- |
| `crates/pcb-router` | the format-independent routing core: geometry, lattice, negotiation, pours, verifier |
| `crates/pcb-placer` | the electrostatic placer, annealer, legaliser and playback |
| `crates/pcb-kicad` | KiCad S-expression parsing and writing, project rules, the board adapters, native verification |
| `src/main.rs` | the `pcb-maker` command line |
| `benchmarks/corpus` | the KiCad-demo corpus runner (route and layout) |
| `benchmarks/github` | the harvested open-source boards: harvest, manifest, Freerouting comparison |
| `benchmarks/pcbench` | 617 open-source boards (PCBench through PCBWorld's D3 preparation): fetch, manifest, runner, triage |
| `benchmarks/agent-tasks` | layout tasks as an agent poses them: stacked parts, constraints, a finished board |
| `docs/` | design notes, results, the hand-over with the open problems |

The other crates (`pcb-engine`, `pcb-grid-router`, `layout-trace-*`, …) are
earlier approaches kept for their tests and experiments; the
[trust audit](docs/reviews/2026-09-21-trust-audit.md) explains what was
kept and why.

## License

MIT, see [LICENSE](LICENSE).
