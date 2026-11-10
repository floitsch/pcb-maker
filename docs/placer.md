# The placer

The placer puts a board's movable footprints where the router can connect
them. It has two layers:

- `crates/pcb-placer` is the core. It knows bodies, pins, nets, an outline
  and constraints, and nothing about KiCad. Its entry point is
  `pcb_placer::place` (`lib.rs`).
- `crates/pcb-kicad/src/board_placer/` lowers a KiCad board to the core's
  `Problem`, runs several seeds, picks one, writes the placed board and
  `board-placer.json`. Its entry point is `place_kicad_board` (`place/`).
  The layout (`board_layout.rs`) adds the race: it ranks the seeds'
  placements, probes the best ones with the router and routes on with one.

Two commands use it:

```sh
# Placement alone: the placed project, placement.html (an animated playback)
# and board-placer.json.
pcb-maker place-kicad-board <source-dir> <board-id> <out-dir> [layout.json|placer.json]

# Placement, the race and routing.
pcb-maker layout-kicad-board <source-dir> <board-id> <out-dir> <router.json> [layout.json]
```

Given a layout config, `place-kicad-board` places exactly as the layout
would (`placement_check_config`: the budget share, the seeds, the router's
rules), so its yes is the layout's. Every key of `placer.json` (the
`placer` object of `layout.json`) is in the
[configuration reference](config-reference.md); the constraint language
is in [constraints.md](constraints.md). After the first route the coupled
loop nudges congested parts within the placement's level
([router.md](router.md)), pin swaps run once if the layout names them
([pin-swap.md](pin-swap.md)), and the polish places the reference and
value fields (`labels.rs`, `silk_fields.rs`).

Terms used throughout:

- A **held** (fixed) part keeps its pose; a **movable** part may move and
  turn. A part's **body** is the box or disc it occupies (courtyard and
  pads); its **halo** is room kept around the body for its pins' escapes.
- A **level** is one step of the relaxation ladder: the halos, spacing,
  grid and body shapes the legaliser keeps. Level 0 is the most generous.
- **Work** is the placer's unit of effort; every budget is in work, not
  wall time ("Placement work against the CPU").
- The **passives** are two-pad resistors, capacitors, inductors, ferrites
  and fuses, and resistor or capacitor arrays (`skeleton_passive` in
  `board_placer/skeleton.rs`); the **active parts** are the others.

## The pipeline at a glance

In the order the code runs, for one board:

1. **Prepare and lower** (`prepare_board` in `board_placer/place/prepare.rs`,
   `lower_placement` in `board_placer/lower/`): read the board in
   canonical order, resolve constraints and the project's rules, build
   bodies, pins, held parts, halos, net weights, obstacles and relations.
2. **Panels and cost terms** (`place_kicad_board`): detect copies or
   sub-boards; attach escape demand and ratsnest crossings.
3. **Seeds** (`race_seeds`, `board_placer/seed_race.rs`): place
   `placement_seeds` seeds in a pool, each by `pcb_placer::place` (plain),
   `pcb_placer::skeleton::place_skeleton` (the race over skeletons) or the
   panel placers (`panel::place_copies`, `sub_boards::place_sub_boards`).
4. **Inside `place`** (`pcb-placer/src/lib.rs`): fit the halos, start from
   canonical angles, global placement (`global.rs`, from the netlist start
   in `initial.rs`), walk the relaxation levels (anneal in `anneal.rs`,
   legaliser in `legal.rs`), then refine, repair relations and align the
   best level's placement (`finish`).
5. **Pick**: sort the seeds by the placer's key; write the first, return
   the other legal ones as alternatives.
6. **Race** (`layout_kicad_board`, layout only): rank the placements by
   RUDY and crossings, probe the best three with the router, route on with
   one.

## Work, budgets and determinism

### Placement work against the CPU

Every legality test, anneal move and wirelength evaluation books work
(`pcb_placer::work`, `add_work`, thread-local). Budgets are work limits:
`Config::work_limit` stops new levels and anneal stages, and `out_of_work`
lets the legaliser, eviction, refinement and repair passes stop inside a
pass. Work turns into seconds at `work_per_second`: 150 M units a second
on one thread of the reference machine (`REFERENCE_WORK_PER_SECOND`),
scaled to the machine at start-up unless `PCB_WORK_RATE` pins it (see
[router.md](router.md) for the machine's rates). Because budgets count
work, a busy or slower machine places the same way; the wall clock only
guards the layout (`wall_guard`).

The layout gives the placement 40 % of `total_seconds`
(`PLACEMENT_SHARE`): 600 s of work at the default 1500. Each seed gets that
budget (`KiCadBoardPlacerConfig::work_seconds`). `place-kicad-board` uses
the same unless `PCB_PLACER_WORK_SECONDS` sets another.

`PCB_PLACER_DEBUG` prints each phase's work against its thread's CPU
time (near 1 when calibrated). Check mode (`PCB_PLACER_CHECK_OVERLAP`, and
debug builds) checks every cache against its direct computation without
booking work or drawing random numbers, so it places the same way.

### Order independence

A placement must not depend on the file's order or on the angles the
parts held when read (two saves of one design once placed 103 of 130
parts elsewhere). So every command that places or routes reads the board
through `canonical_order::parse_board` (footprints by reference, their
items sorted); the orientation search starts every movable part at the
first of its sorted angle options (`orientation_search`, on;
`orient::canonical_start`), so an import at 0 degrees and the designer's
angles place alike; and which seeds run and are kept depends on their work
alone (`race_seeds`: seed k is kept while the seeds before it used less
than the budget, three at least), not on threads or finishing order. A
layout is deterministic in its input, binary, settings and work rates.

## Lowering a KiCad board

`prepare_board` resolves `constraints.json` and fills what the config
leaves unset from the project's rules: the clearances (copper, a custom
rule's pad-to-pad, copper-to-edge, hole, mask), and whether courtyards may
overlap (`tight_bodies`), holes in a courtyard are an error and silkscreen
over a part is an error. The layout passes the router's clearances
instead (`layout_placer_config`). Outline, keepout and side constraints
change the board before it is lowered.

`lower_placement` (`board_placer/lower/`) then builds the
`pcb_placer::Problem` (`problem.rs`):

- **Bodies.** A body is the courtyard and pad extent, a box or a disc
  (`local_body`, `bodies.rs`). A part whose courtyard covers little of
  that box is **hollow** and blocks with its pads and courtyard pieces
  only (others may sit inside a shield's outline). A through-hole part
  blocks the far side with its holes and pads, a part with a courtyard on
  both sides (a hot-swap socket) with that courtyard too. The **tight
  body** (fabrication outline and pads) is used only where the project
  lets courtyards overlap.
- **Angles.** A movable part may take any quarter turn (`keep_rotation`
  false) unless a `rotation` constraint restricts it. Only `front` and
  `back` constraints change a part's side.
- **Held parts.** A part is held when it is locked, has no net, or sits
  where the designer put it on purpose: its body leaves the outline, it
  reaches within `edge_keep_mm` (3 mm) of a rule area that forbids
  footprints (an antenna module at its keepout), or it is a through-hole
  part with at least four pins (a connector, a header) within
  `edge_keep_mm` of the outline. `fixed` and `fixed_patterns` add parts,
  `free` releases them. The placer, the keepout constraints and the free
  dimension share one decision (`placement_constraints::held_parts`).
- **Halos.** Each part keeps `ceil(pins / pins_per_halo_track)` tracks of
  `track_pitch_mm` free around its body (8 pins a track, 0.65 mm), at most
  `maximum_halo_mm` (4 mm), times `halo_scale` (1);
  `halo_overrides_mm` replaces the rule per reference. Halos count as body
  for spacing and density. A PCB routes in the plane it places in, so the
  room for a part's escapes has to be reserved at placement time.
- **Spacing and grid.** Bodies keep `spacing_mm` (0.5) apart, never less
  than the largest copper clearance, on a `grid_mm` (0.635) grid,
  `edge_margin_mm` (0.5) from the outline.
- **Obstacles.** Cutouts keep copper an edge margin away; bodies may reach
  over them (a connector's pegs). A footprint's own Edge.Cuts loops (a key
  switch's peg holes) move with it (`Component::slots`). Rule areas that
  forbid footprints are obstacles, those forbidding only pads keep copper
  out. Board copper graphics are obstacles; a footprint's own copper and
  mask graphics are part of its body (`mask_openings`, on: an RF module's
  mask opening once took parts whose pads KiCad flagged as mask bridges).
- **Footprints without a courtyard.** Two such parts keep the boxes of
  their pads and outlines apart (`Component::courtless`) unless one is at
  most a quarter of the other (a diode under its key switch); against a
  part with a courtyard they block with their pads. Without this, rows of
  courtless key switches interleaved, pads in each other's peg holes.
  `solid_pin_rows` (on) makes a courtless QFP, SOIC or header block with
  its whole pad box.

### Net weights

A net's weight in the wirelength is `min(1, 3 / (pins - 1))`
(`board_placer::net_weight`): 1 up to four pins, 0.16 for a 20-pin rail,
so that power nets do not collapse the layout. Nets designers keep short
count three times (`keep_short`: names with a token for a switch node,
feedback, bootstrap, RF or antenna feed, or a crystal pin). Nets named
`unconnected-(...)` pull nothing (`placer_net`). The same weights hold in
every stage.

### The board's own texts and silkscreen graphics

A text or silkscreen graphic the input placed is an intent (header pin
names, a logo): it stays, and parts keep off it (`keep_board_marks`, on;
`board_placer/marks.rs`). Each **mark** is an obstacle on its side: texts
on silkscreen and copper by their box, silkscreen graphics by their
strokes in clusters (one spanning most of the board is left out); parts
meet silkscreen marks without their halo. A mark on a movable part at its
input pose belongs to that part (`attached_marks`). A part that finds no
room off the marks may lie on them (`seat_on_marks`; `parts_on_marks`),
which ranks below a placement without. Where the project does not rate
silkscreen over a part an error, marks are no obstacle at the last level.
The polish later moves only the parts' own reference and value fields.

### The far side of a via in a BGA's pad

Where the router will put a via in an enclosed BGA pad (the designer's
vias in its pads, or the router's `via_in_pad`), the via goes through the
board, and parts on the other side keep off it as off a through-hole pin
(`via_in_pad_far_side`, on; `far_side_of_vias_in_pads` in `lower.rs`).
The same holds for the via sites of a prepared BGA fan-out
(`far_side_of_fanouts`). A board without such a footprint lowers as
without the setting.

### Room around grid arrays

Around a grid array whose inner balls the router's fan-out serves with
dog-bones (`bga_fanout`), other parts keep off a band on the array's side
for the surface escapes of its outer rings and off the far side under and
around it for the via ring, where only the array's own decoupling
capacitors may sit (`bga_room`, on; `room_around_arrays` in `lower.rs`;
`Problem` field docs in `problem.rs`). Held parts in the band stay and are
reported (`array_rooms`). On Glasgow's BGA-121 this took KiCad's
unconnected from 343 to 262 over three seeds; no other task board moved.

### Length maxima

For a net with a `length` rule maximum (the project's `.kicad_dru`),
every two of its pins on different parts are kept within 0.8 of it
octilinearly by a soft penalty, and within the maximum by one no
wirelength pays for (`length_maxima`, on; `length_windows`): the router
cannot make a net shorter than its pads lie apart. The rules come from the
router's configuration (`router_rules`: layout and place check only).

### Length minima

For a net with a minimum on two parts, their pins are kept at least half
the minimum apart (`length_minima`, off; `LENGTH_FLOOR_SHARE`): meanders
cannot make up a minimum when the pads are too close. Off: the lengths
improved on both measured boards, but one lost connections.
`board-placer.json` reports the held rules (`length_rules`).

## Constraints and automatic relations

Constraints come from `constraints.json` (see
[constraints.md](constraints.md)); `apply_constraints` in
`placement_constraints/` lowers them into `pcb_placer::constraints`:

- **Hard** constraints (edge, region, overhang, rotation, rows as
  followers of a leader) are part of legality: the global placement's
  clamp, the anneal, the legaliser and the refinement keep them.
- **Relations** (`near`, `relative`, `apart`, length windows) are soft: a
  penalty in millimetres of violation times `constraint_weight` (50 mm of
  wire per mm missed), and a force in the global placement. The report
  says per constraint whether it holds and by how much it is missed.

Before placing, `placement_room::outline_holds` refuses what cannot fit
at all as an input error (exit 2): a row longer than the outline in every
orientation it may take, parts held at one edge that need more length
than the edge has.

The lowering adds **automatic relations** (`auto_decoupling`, on;
`board_placer/decoupling.rs`) unless a user constraint names the part: a
capacitor between a rail and ground within 2.5 mm (`decoupling_mm`; bulk
capacitors of 4.7 uF and more 5 mm) of a supply pin of an IC on its rail;
a crystal within 3 mm of the IC pins it drives; a switcher's inductor
within 3 mm of a pin whose function says it switches; ESD protection
within 3 mm of the connector pins it guards.

### Decoupling at the pins

`decoupling_near` (off) anchors each capacitor at the nearest of every
supply pin of its rail, not each IC's first, at 1 mm, and legalises a near
part right after its anchor. An RP2350 board's capacitors within 3 mm of a
pin went from 14 % to 50 %, but the dense boards routed no better and the
OpenESC boards' capacitors moved away.

### Pair rows

`pair_rows` (off; `board_placer/pair_rows.rs`, `facing.rs`) places a
differential pair's two series parts (AC-coupling capacitors, series
resistors: two free two-pad parts of one footprint on the two nets of a
pair KiCad pairs by name) as a `row` of two, so that the router can couple
the pair through them. The pitch is the pair's track width plus gap, at
least the courtyards' width; after the refinement each row turns to face
its pair's source and destination (`facing::face`). On the measured boards
the pairs without two parts side by side fell from 14 to 3, but the pairs
coupled barely changed and the connections moved both ways.

### Following diff_pairs

`PAIR_ROWS_FOLLOW_DIFF_PAIRS` (false) keeps an unset `pair_rows` off
where the router plans pairs (`pair_rows_for`): following coupled no more
pairs and cost connections on two boards.

## Global placement

`global::global_place` follows ePlace/RePlAce. A body's area (halo
included) is charge; the potential solves Poisson's equation on the
board's box spectrally (`Field`, a cosine transform), so the spreading
force acts at a distance towards free space. The area outside the outline
and the held bodies is fixed charge; a two-sided board has a field per
side. Wirelength is the weighted-average model on true pin positions.
Nesterov's method with a Barzilai-Borwein step minimises `wirelength +
lambda * density` while `lambda` grows until the overflow is below
`stop_overflow`. Every `rotation_interval` iterations, once the overflow
is below 0.75, each part takes the angle that minimises its nets' half
perimeter (`choose_rotations`).

Every part on a PCB is large next to a density bin, so the density target
is 1 (a lower target is never met; the overflow plateaued at 0.24 to
0.40): bodies exclude each other, and the halos and filler charge decide
the whitespace between them.

`GlobalConfig` (`global.rs`) defaults:

| field | default | meaning |
| --- | --- | --- |
| `bins` | 64 | density grid per side |
| `whitespace_fill` | 0 (a placer.json key) | share of whitespace taken by fillers; 0 spreads the parts, so the spare room lies between them (a 0.6 fill left 20 open on a board where 0 left 11) |
| `stop_overflow` | 0.04 | stop once the overflow is below this (after 20 iterations) |
| `max_iterations` | 1500 | cap |
| `stall_iterations` | 0 (100 under the skeleton race) | stop when the overflow has not fallen below `stall_factor` (0.98) of its least for this many iterations |
| `fit_turns` | false (true under the skeleton race) | never turn a part to an angle at which it is longer than the board |
| `lambda_growth` | 1.03 | density weight growth per iteration |
| `rotation_interval` | 15 | iterations between rotation updates |
| `rotation_net_limit` | 24 | larger nets do not vote on rotations |
| `stacks` | `Stacks::BANKS` | which netlist stacks are glued (below) |

### The global placement's start

The descent starts where the netlist puts the parts
(`initial::quadratic_start`, RePlAce's initial placement): the centres
that minimise the nets' quadratic wirelength in the bound-to-bound model
with the held pins as anchors, by conjugate gradients, linearised again up
to 12 times. A part an edge or region holds is projected there and
anchored, so its neighbours follow. A tenth of the old start noise
(`NETLIST_NOISE`) breaks ties. A part with its own start (a panel copy's)
begins there. Started from one point with noise, the descent had spread
the parts in the noise's order and the wires never rearranged them (see
"Where the arrangement is lost"). The netlist start passed the full
protocol: KiCad unconnected -6.8 % (p 0.020), no strict pass lost.

### Banks on a wide bus glued through the descent

The quadratic solve puts parts on the same nets on one spot: it cannot
tell eight memory modules on one bus apart. `spread_stacks`
(`global.rs`, `stacks.rs`) finds such **stacks** at the solved centres
(parts on one side whose nets have enough in common and whose bodies
overlap) and spreads each into a **bank** of cells at its pitch, in the
shape that adds the least wire (`stacks::bank_columns`). A bank is
**glued**: it moves as one body through the descent (its members' forces
summed) and keeps its members' angles. The default rule
(`Stacks::BANKS`) glues banks of three or more parts with three or more
nets that share nine tenths of their nets and at least 24 of them: a
memory's address and data lines. On the video board this took KiCad's
unconnected from 92 to 0 by the median.

The rule is narrow on purpose: a bank spread only at the start is undone
by the descent where no wire holds it; the anneal takes glued banks of
capacitors on a rail and ground apart; glued pairs only redraw the board;
the broad multi-net rule lost on several boards. After the descent a
bank's parts are ordinary parts.

## Relaxation levels, anneal, legalisation and refinement

`pcb_placer::place` (`lib.rs`) first fits the halos (`fit_halos`): they
shrink in eighths, the small parts' first, then those of parts with at
least `MANY_PINS` (10) pins, until each side's movable bodies, halos and
spacing take at most `maximum_utilization` (0.6) of the area its held
parts leave free. A crowded board cannot afford full halos, and the
many-pin parts' escapes need the room most. After the canonical angles and
the global placement it walks the levels from `Config::first_level` until
one seats every part or the work runs out, finishes the best placement
and reports `unplaced`, `illegal`, the constraints and the level.

The relaxation levels, loosest comfort first (`relaxation_levels`):

| | halos (many-pin / other) | spacing | grid | other |
| --- | --- | --- | --- | --- |
| 0 | 1 / 1 | full | `grid_mm` | |
| 1 | 0.5 / 0.5 | full | `grid_mm` | |
| 2 | 0.5 / 0 | full | `grid_mm` | |
| 3 | 0 / 0 | full | `grid_mm` | |
| 4 | 0 / 0 | copper clearance | `grid_mm` | |
| 5 | 0 / 0 | copper clearance | 0.1 mm | |
| then | | | 0.1 mm | bodies closer to the edge, as far as their copper allows (if any part has an inset) |
| then | | | 0.1 mm | tight bodies (where the project lets courtyards overlap) |
| then | | | 0.1 mm | the edge margin down to the rules' copper-to-edge clearance |
| then | | | 0.1 mm | parts held at an edge need only their copper on the board (a card edge in its tab) |
| last | | | 0.1 mm | courtyards may touch where their copper keeps the clearance, judged by KiCad's legality |

The level a placement ends at is reported, and every later move (the
coupled loop's) keeps to its rules (`Relaxation::apply`).

**At each level**, a fresh anneal runs from the global placement's poses
(`anneal::anneal`), pair rows turn to face their pairs
(`facing::turn_to_face`), and the legaliser seats every part
(`legal::legalize`). Parts that found no room go first in a second pass
(`legalize_first`); a few still without room (at most a fiftieth of the
parts, three at least) take a spot from movable parts, which find new ones
(`evict_for`); the rest may lie on the board's marks (`seat_on_marks`).

**A looser level keeps what the level before placed**: what was legal
stays legal, so only the parts without room look for a spot
(`legalize_keeping`), then eviction, then (at the edge and courtyard
levels, where nothing else is left to give) up to four passes with the
stuck parts first from the anneal that gave the kept placement
(`legalize_stuck_first`, `STUCK_FIRST_PASSES`). A level with a finer grid
or tighter bodies is a new problem and gets a fresh anneal too, which may
beat the kept placement.

**Shortcuts**, each for a case that used to waste the budget: a level
whose bodies need more area than a side has is skipped (`overfull`); where
even the last level is overfull or the parts cannot lie side by side in
strips of the board (`strips_overfull`: 34 keys of 18 x 17 mm on 80 x
40 mm, 96 % by area), no packing exists, the parts are seated without an
anneal and the report says `overfull` with a suggested outline, in seconds
instead of ten minutes; with many parts without room and too little work
for every level before the tight bodies, the walk jumps to them
(`skip_to_tight`); with few parts without room and the same grid and
bodies, the next level does not anneal again.

### The anneal

`anneal::anneal` lets all parts negotiate the global placement's
overlaps: greedy largest-first legalisation exiled the small passives and
doubled the wirelength. Moves, quarter turns (15 %) and swaps (12 %) are
judged on weighted wirelength, relations, 1000 times the hard-constraint
violation, the ratsnest crossings and an overlap penalty growing from 0.5
to 400 mm of wire per mm². It starts cold (temperature 0.3, times 0.975 a
stage) so the global result is refined, not scrambled. Defaults
(`AnnealConfig`): 160 stages, 40 moves per part a stage, a move radius
from 6 % of the board down to 1.5 grid steps, seed 7. `anneal::objective`
gives the objective split by term.

### The legaliser

`legal::legalize` moves each movable part, largest first, to the nearest
legal grid position and angle (`is_legal`: in the outline, clear of every
body by the level's rules, hard constraints kept), looking up to 2 mm
farther for fewer crossings (`CROSSING_REACH`) and paying escape-demand
overflow. On a fine grid it searches only near the part's wanted spot.
`legal::conflict_kind` says why two parts may not touch.

### The legality model against KiCad

The levels before the last keep bodies a full spacing apart, which is
stricter than KiCad. At the last level the KiCad layer turns on KiCad's
own legality (`Config::kicad_last_level`, `Constraints::pad_clearance`):
courtyards may touch where the pads keep the copper clearance pad to pad,
an unheld part's pads keep the edge margin, marks the project does not
rate an error over a part are no obstacle, and a lead through the board
keeps the copper clearance from the far side's pads. This is how
designers pack a crowded side; the stricter model called most designers'
parts illegal where KiCad found nothing. A placement that needed this
level ranks below every one that did not, in the placer's key and in the
race. `legality-kicad-board` judges a designer's placement level by
level.

### Refinement and finishing

`finish` (`lib.rs`) runs on the best level's placement:

- `legal::refine_with`: `refine_passes` (8) passes of legal moves, turns
  and swaps that strictly lower weighted wirelength plus the crossings and
  escape-demand costs;
- `legal::center_edge_copper`: parts held at an edge by their copper with
  less than 2 mm of play move to the middle of their stretch;
- `legal::repair_relations`: parts that miss a near or relative relation
  search legal spots around what the relation wants, and the refinement
  runs again, up to five rounds;
- `legal::align_orientations`: parts of one kind take their kind's most
  common angle where that costs at most 0.5 mm of wire each;
- `facing::face`: pair rows turn to face their pairs' ends.

### Pads without a way out

`pad_exits` (off; `pcb-placer/src/exits.rs`, `board_placer/exits.rs`)
asks the router whether each pad has a way out on the static obstacles at
its class width (`ExitProbe` in pcb-router). A pad another part, a
keepout or the edge closes is a placement defect: that part or those that
close it are moved or turned (within `reach_mm`, the cheapest of
`candidates` poses that leave fewer pads closed), and pads left closed are
reported and rank right after illegal parts. Needs the router's config
(layout or place check). Stage 1 on its four boards: -3.0 % (p 0.228);
the full protocol is pending.

### Bus-group moves

`bus_groups` (off; `groups.rs`) moves the units a designer places as one:
**banks** (parts of one footprint sharing at least `min_shared` small bus
nets) and **clusters** (a part with its passives). After the global
placement each bank takes its best order and arrangement (`arrange`), and
the anneal and the refinement shift, turn, swap and reorder whole groups.
The placer's objective falls with it (8.5 % on the placed boards) but
routing does not improve, so it stays an option.

## Cost terms shared by the stages

Wirelength alone cannot see two things that decide routability on a
board of few layers: whether a part's pins can leave its body, and the
order of parts along a bus. Two terms add them, each in every stage that
moves parts, so that a later stage does not undo what an earlier one
built (a routing term in the anneal alone was undone by the legaliser and
refinement, which then optimised wirelength only).

### Escape demand

`escape_demand` (on; `pcb-placer/src/escape.rs`, `escape_fact` in
`board_placer/config.rs`). A pad's net leaves its body through the side
facing the net's nearest pad on another part; a part's demand on a side is
the nets leaving there. Two facing bodies form a channel holding the gap
over the track pitch times the routable layers; demand above that is
overflow. The global placement inflates a part's charge on its dense
sides (`inflate`, 1), the rotations prefer dense sides facing free space
within `turn_tolerance` (5 %) of the best wirelength, and the legaliser
and refinement pay `weight` (2 mm) per track of overflow. In a layout the
refinement corrects the estimate by the global route's overflow per part
(`measured`, every 20 kept moves; `board_placer/guidance.rs`).

Guard: where at least `fixed_share` (0.6) of the footprints are held,
escape demand is not applied. On such boards the movable parts are mostly
passives among held parts, and their nets leave towards the parts they
belong to whatever the estimate says. Measured: the dense boards' KiCad
unconnected fell 7 %; the guard recovered the fixed-heavy boards it lost
(katia 162 to 38).

### Ratsnest crossings

`ratsnest_crossings` (on; `pcb-placer/src/crossings.rs`, `crossing_nets`
in `board_placer/config.rs`). Per counted net (two pins or more, not
carried by a pour) the minimum spanning tree of its pins; the cost is the
number of pairs of tree lines of different nets that cross. On a board of
few layers each crossing is a via pair or a detour. `CrossingState` keeps
the count incrementally through a bucket grid: a move changes only the
trees of the moved part's nets.

| key | default | where |
| --- | --- | --- |
| `weight` | 3 mm of wire per crossing | calibrated on the placement ablation: 2 to 3 mm ordered the variants best |
| `legalise` | on | the legaliser's spot cost |
| `refine` | on | the refinement's move cost |
| `anneal` | on | nets of up to 16 pins (`ANNEAL_NET_PINS`), until the term's work reaches half the anneal's own (`TERM_SHARE`); half the swaps trade a part with a peer (same footprint, or both with many pins) |
| `rank_seeds` | on | the race ranks seeds by RUDY rank plus crossings rank |
| `per_layer` | off | scale the weight by two over the routing layers |

Bus orders are decided in the anneal: the term there took the dense
boards' KiCad unconnected down 10.8 % (p 0.002). Without the work share it
cost several times the anneal and left overlaps the legaliser undid.

## The seeds and the placer's pick

`place_kicad_board` runs `placement_seeds` seeds (3 by default; the layout
asks for `congestion_candidates`, 16, when it pre-ranks). Seed k uses
global seed `seed + k` and anneal seed `7 + k`. The seeds are sorted by the
placer's key, a tie to the earlier seed:

1. unplaced plus illegal parts;
2. pads other parts close (`pad_exits`, when on);
3. whether the placement needed the last (courtyard) level;
4. parts lying on marks;
5. summed constraint violation;
6. final wirelength.

The first is written, tracks and vias removed, and checked: every pad
must land exactly where the placer's model put it. The other legal
placements go to the race as `alternatives`. `board-placer.json` reports
the level, the constraints, unplaced and illegal parts with what holds
them, the room per side with hints, the seeds and the work
(`pool_work_seconds`, the kept seeds on eight reference threads, is what
the layout's clock charges).

## A race over skeletons

`skeleton_race` (on where a seed's work reaches `min_work_seconds`;
`pcb-placer/src/skeleton.rs`, the class rule in `board_placer/skeleton.rs`)
places each seed in two phases:

1. **The skeleton**: every part but the free passives (movable, named by
   no user constraint) is placed alone; the passives are absent. Each
   skeleton part's body grows by the area of the passives it owns (`room`
   1 times their tight bodies). A two-pin passive's two nets are one net
   where both have at most `join_pins` (8) pins and no pour carries either,
   so a series resistor keeps its ends' parts together. The skeleton
   starts at the first level where the whole board fits by area
   (`first_fitting_level`), takes at most `work_share` (0.5) of the work,
   and its global placement uses the stall stop and fitting turns.
2. **The passives** are placed around the held skeleton with the rest.

Where either phase leaves parts without a legal place, the seed is placed
plain with its full work, charged on top (with only the work left, video's
fallbacks left memory modules out). The layout's race ranks the results as
any. Not on panels of copies or held sub-boards; `plain_share` (0) mixes
plain seeds in.

Why: the designer's active parts pinned recover most of the designer's
advantage (see "Which placement decisions matter"), so placing them first,
undisturbed by the passives, attacks the coarse arrangement directly. The
full protocol: KiCad unconnected -12.1 % on the dense boards (p 0.002;
video -33 %, protoesp -40 %).

### The passives' first level

The passives start at the skeleton's level, but no looser than level 2
(`PASSIVES_FIRST_LEVEL`: many-pin halos halved, none on the others). At
full halos the passives found no room around a skeleton grown by their
tight bodies (26 to 109 parts without room on two boards), and
`skip_to_tight` sent them to the tight levels with the seed's whole work.
A skeleton that already skipped past level 2 keeps its level. A region the
lowering gives a part (an outline piece) does not keep it in the skeleton
(`Constraints::automatic_regions`). Choosing the level per board was
tried and found no rule (see "Tried and rejected").

### The skeleton's budget

Below `min_work_seconds` (500 s, `SKELETON_MIN_WORK_SECONDS`) every seed
places plain. At the short tier's 200 s the skeleton's half of the work
made a small board's skeleton skip to the tight levels on a quarter of its
seeds, the race kept those, and the board lost its strict pass; from about
475 s the skeleton walks its levels on every seed.

## Panels

All on by default, and no change on a board without pieces or copies:
`keep_on_pieces` keeps the parts a net joins to held parts on one outline
piece on that piece; `keep_copies` gives each tabbed copy's parts a region
between the copies' held parts (`keep_copies_apart` in `lower.rs`);
`panel_copies` and `sub_boards` follow.

### Panel copies

Designers place one copy and replicate it; placing parts independently
interleaves the copies. `panel_copies` (`board_placer/panel.rs`) does as
the designer does. `panel::detect` finds groups of parts with the same
footprints whose nets map one to one, told apart by net islands (KiKit's
`Board_N-`), numbered references (U101, U201) or the nearest held part;
the copies' held parts give each copy's transform and region.
`panel::place_copies` places the first copy in its region, replicates it,
then places the copied parts that are not legal where they land and the
parts in no copy. A copy's constraints anchor in its own copy
(`anchor_in_copy`), and parts keep off the frames and tabs except
connectors and the designer's overhangs (`sub_boards::keep_off_frames`).

### Sub-boards in a panel

A panel of different boards shares net names (GND, the sheets' nets)
though no copper joins its boards; placed as one, its parts interleave and
the router joins same-named nets across the tabs. `sub_boards`
(`sub_boards.rs`) cuts the outline at necks narrower than 2 mm, mouse-bite
rows and V-cuts, assigns each part a sub-board (from held parts, the
`--reference` board, and nets), splits same-named nets per piece for the
router, and keeps parts off tabs and frames.
`sub_boards::place_sub_boards` places each sub-board alone in its region
with a budget share by its parts (its own fullness decides its levels),
then the unassigned parts. With every part stacked and no `--reference`,
the pieces are only reported.

## The race

The shortest placement is often not the one that routes (the most
compact placement of an early board had the best wirelength and did not
route). The layout (`layout_kicad_board`, `board_layout.rs`) races:

1. **Pool**: the kept placement and the other legal seeds that miss no
   more constraints, duplicates removed.
2. **Pre-ranking** (`congestion_score` `"rudy"`, with one router seed and
   no pin swaps; `pre_ranks`, `congestion_rank.rs`): RUDY, each net's
   spanning-tree length times its track pitch spread over its box, summed
   per 16 x 16-node router tile above a quarter of the tile's free
   capacity. With `rank_seeds` the order sums the RUDY and crossings ranks;
   placements at the courtyard level or on marks go last. `"none"` races
   plain seeds; `"global"` (an experiment) scores the global route.
3. **Probes**: the best `congestion_probes` (3) are routed side by side
   for the router's `probe_seconds` (75 s of work), all charged to the
   clock; the kept placement's probe starts its first route. To leave the
   final route `RACE_MIN_FINAL_SHARE` (45 %) of the budget, every probe is
   shortened alike down to a third (`RACE_SHORTEST_PROBE`), then the
   lowest-ranked are dropped (`race_probes_that_fit`), decided before any
   probe runs.
4. **Pick** (`race_pick`): the probe with the fewest open terminals after
   the rip-up routes on where it beats the first-ranked placement by
   `RACE_RIPPED_MARGIN` (2 %); below that the probes do not separate
   placements reliably and the first-ranked routes on.

Pre-ranking 16 seeds instead of racing three took the quick tier's KiCad
unconnected from 98 to 64. Every probe counts: a former gate that dropped
probes past 40 % of the budget, uncharged, cost a board 43 connections.

### The race ranks its probes by the ripped count

`race_count` (`"ripped"`, the default; `"unfinished"`): after its
negotiation, each probe's router strips its nets in conflict of their
conflicted branches (the hard repair's rip-up without its searches,
`Router::rip_up_standing`) and counts the board's open terminals; the
router is then put back, and the kept one routes on unripped. A net in
conflict counts once in the unfinished nets but counts every terminal its
conflicted branches held once ripped, which ranks placements far better
(Spearman with the final KiCad count +0.42 to +0.49 against +0.11 to
+0.41). By the protocol at nine seed pairs: -5.6 % (p 0.014).

### The race's route probe

`race_probe` (layout option, off; `{}` for `seeds` 3, `share` 0.08) gives
each probe `share` of `total_seconds` instead of `probe_seconds`, never
drops one, and ranks by the probe's open terminals, then unfinished nets,
then nets in conflict. With `repair` each probe is a whole first route.
Off because the dense medians stayed flat.

## Outline sizing and the free dimension

The placer places within a given outline; two layout features call it
at several sizes. An outline sized from the parts (`size_outline` in
`board_layout.rs`) starts at the size designers use, grows by a quarter
while the parts do not fit or a quick route leaves connections open, and
with `shrink` shrinks while it routes. A free dimension (`free_dimension`,
`free_dimension.rs`) moves one edge of a rectangular outline, carrying the
held parts within its band, and lays the board out at each size as an
ordinary board to find the smallest clean one.

## Diagnostics

| switch | what it does |
| --- | --- |
| `PCB_PLACER_DEBUG` | per level and phase: wirelength, parts without room, work against CPU |
| `PCB_PLACER_TRACE=<file>` | per kept seed: poses after each stage (start, global, annealed, legal, final; the skeleton's before them), the global descent's history, the level walk (`board_placer/trace.rs`) |
| `PCB_PLACER_DESIGNER=<board>` | the designer's placement judged by the placer's model, with `PCB_PLACER_DESIGNER_LEVEL` |
| `PCB_PLACER_START=<board>` | start the stages from the designer's poses: `PCB_PLACER_START_STAGE` `global` (default), `anneal` or `legalize`; `PCB_PLACER_START_PARTS` names the parts that start there (`Config::start`) |
| `PCB_PLACER_SKIP_ANNEAL` | no anneal at any level: each level's legaliser seats the global poses (`Config::skip_anneal`) |
| `PCB_PLACER_CHECK_OVERLAP` | check mode (above) |
| `PCB_PLACER_WORK_SECONDS` | `place-kicad-board`'s work budget |
| `PCB_PLACER_PAD_EXITS` | `pad_exits` on (`1`) or off (`0`) |

Scripts in `experiments/placement-gap/` use them: `stages.py` (each
stage against the designer's), `partialcopy.py`, `screen.py` (stage 1).

### Placement ablation

`pcb-maker ablate-kicad-placement <designer.kicad_pcb> <placed-dir>
<board-id> <out-dir>` (`lab/placement_ablation.rs`, see
[agents.md](agents.md)) writes hybrids of the designer's placement and
ours, one group of parts swapped at a time, and the signals of each
(crossings, objective, wirelength, blocked exits, overflows), so that the
cost of a group of placement decisions can be routed and measured. Its
crossing count is the placer's (`crossings::spanning_tree`,
`lines_cross`). The ratsnest crossings term came from it: on the boards it
studied, the costly decisions were part orders along buses.

## Where the gap to the designer comes from

About two thirds of what our layouts leave open is placement: the nightly
routes the designer's placement of each target board with our router and
compares (the gap line of the nightly report; see
[benchmarks.md](benchmarks.md)). What is known about that gap:

### Which placement decisions matter

Pinning some of the designer's parts and placing the rest
(`partialcopy.py`): with the designer's active parts pinned (a quarter of
the parts), our layouts recover a median 0.92 of the designer's
first-route advantage on seven boards; the connectors alone -0.16, the
three largest ICs 0.15. Our wirelength there is lower or within 2 % of
our own on six of seven. So the gap is the active parts' arrangement as a
whole (banks of repeated parts its sharpest case), which our objective
accepts and our search does not reach; the passives around a good
skeleton are not where the advantage is.

### Where the arrangement is lost

Each stage started from the designer's poses (`PCB_PLACER_START`) and
compared with our own pipeline (`stages.py`, four boards, two seeds):

- **The global placement lost the coarse arrangement.** From the old
  one-point start the active parts ended a shuffle's distance from the
  designer's, the seeds as far from each other. Started from the
  designer's poses it kept most of the advantage (video: 69 first-route
  open terminals against our 304). The netlist start and the race over
  skeletons answer this in part.
- **The anneal loses fine arrangement on dense boards** while it lowers
  our objective, and our legality adds to it (the designer's poses seated
  at our levels cost 28 and 34 open terminals on the 4in1 and Glasgow).
  Yet without the anneal seven of eleven dense boards route worse; the
  loss holds only where small passives crowd many ICs (the 4in1,
  DSP-ADAU1452), packed by the decoupling relations (88 % of the 4in1's
  objective).
- **The legaliser and the refinement** keep the arrangement they are
  given (the designer's poses are a fixed point of the legaliser).
- **The race** cannot pick an arrangement no seed holds.
- **The objective is not what is wrong.** Legalised and refined, the
  designer's arrangement scores 0.59 to 1.02 of our own placement's
  objective, lower on the boards where the objectives differ (by crossings
  and the two-pin and large nets' wire).

What stays open: the OpenESC 4in1-mini loses under the netlist start
(its 22 bulk capacitors on one rail solve to one spot, and the anneal
takes a glued bank of them apart); SNSP-CPU-01 and DSP-ADAU1452 want
different passive levels and no rule tells them apart. Untested: a lower
`constraint_weight` on the dense boards, seeded orientation starts in the
race. More seeds do not close the gap.

## How a placement change is measured

A placer change is judged by routing: placement metrics agree with the
routed verdict only where the metric is the change's own objective. The
protocol and its statistics are in [benchmarks.md](benchmarks.md) (the
placer-default rule, the ladder of placer measurements). A change climbs
three stages and stops where it fails:

- **Stage 0**: placement alone (place checks, `stages.py`,
  `ablate-kicad-placement`): does the change move what it meant to move?
  A check, not a filter. A placement is about a quarter of a layout's
  wall time.
- **Stage 1**: the eleven dense boards, five seed pairs (placer seeds 1,
  101, 201, 301, 401: the race uses seeds N to N + 15, so seeds must lie at
  least 16 apart), layouts stopped after the first route
  (`stop_after_first_route`), judged by `placer_ab.py compare --metric
  first_open`. Passes at the noise file's p <= 0.25. About half the cost
  of stage 2.
- **Stage 2**: the same seeds in full layouts, judged by KiCad's
  unconnected (`placer_ab.py compare`): better at p <= 0.03, extended to
  nine pairs between 0.03 and 0.25; the short tier must lose no strict
  pass.

A default changes only by stage 2. Before measuring, place checks show
that the switch off places byte for byte as before.

## Tried and rejected

One line each: what it was and why it failed. Code for removed switches is
in git history. The switches still in the code that stay off (bus groups,
`decoupling_near`, pair rows, length minima, pad exits, the race's route
probe) say why in their sections above; so do the density target, the
greedy legaliser and `whitespace_fill`.

Global placement and start:

- `routing_demand` (October): each net's wire area as charge over its box;
  no change within the seed noise.
- Crossings in the global placement (`ratsnest_crossings.global`,
  October): large parts traded places after the global placement; the
  placed board came out the same.
- Stacks spread into banks at the start only (November): the descent
  undoes them where no wire holds the parts.
- Glue for every multi-net stack (November): video won, three other boards
  lost beyond their seed spread and one lost its strict pass.
- The stall stop and fitting turns as defaults for plain seeds (November):
  +2.7 %, p 0.62 on the three boards they move.
- Turning off the stall stop for the passives' descent (November): -2.2 %
  at stage 1, too small to reach stage 2's bar; the stall costs nothing.

Halos, legality and levels:

- `routing_room` (October): halos rescaled by the RUDY demand around each
  part; within the seed noise.
- `shared_halos` (October): the larger halo between neighbours, not the
  sum; shorter wire, a strict pass lost and many more vias.
- Far-side gaps in the anneal (October): the anneal kept the legaliser's
  hole and far-copper gaps; worse on video and the 4in1.
- KiCad's legality at every level with the least spacing (`legality:
  "kicad"`, October): the dense sum fell but two of three seed pairs
  rose.
- A guard bounding the skeleton's level by a plain probe's (November):
  kept a small board's strict pass less often than off and gave back
  protoesp's gain.
- The passives' first level chosen per board (November): the objective
  is blind to it, area and seating rules see both contested boards alike,
  a twin probe changed nothing (+0.1 %).
- A rule keeping far-side via pads off fine-pitch fields (November): the
  failing pad failed as often away from the fine-pitch part.
- `join_pins` 0 in the skeleton (November): +3.0 % against the race.
- Skipping the anneal (`PCB_PLACER_SKIP_ANNEAL`, November): +1.4 % at
  stage 1, worse on seven of eleven boards.

Objective terms:

- Net weightings `rail`, `flat`, `sqrt` (November): `rail` +5.6 %, `flat`
  +4.0 % at stage 1; pulling rails together crowds the signal nets.
- Rows of repeated parts as an anneal cost (`align_rows`, November):
  +4.2 % at stage 1; rows bought with wire and crossings.
- The crossings weight scaled by layer count (`per_layer`, October): lost
  a strict pass, dense rows moved both ways.
- The global route inside the anneal (`global_guidance`, an experiment,
  October): its gain was undone by the legaliser and refinement; see
  [global-route.md](global-route.md).
- The move descent after the first route (`move_descent`: single parts
  nudged, turned or swapped where the incremental global route and a
  local reroute said fewer terminals stay open): the dense boards 973 to
  988 KiCad unconnected; removed 2026-11-10.

Race and measurement:

- A trouble-maker probe (October), routing only an earlier run's trouble
  nets: not representative without an earlier run, which a layout lacks.
- A coarse whole first route as the race's probe (October): ranks worse
  than the real first route and saves only a fifth of the probe's work;
  what it found was the ripped count.
- Condensed boards as a fast benchmark (October): most dense boards'
  difficulty is not local to a window.
- A quarter budget as a cheap protocol stage (November): the placement
  itself changes and the rows do not rank the full ones.

## Known limits

- Bodies are rectangles or discs (a part standing at another angle keeps
  a turned rectangle); the search turns parts by quarter turns and never
  changes a part's side.
- The board's top-level items (graphics, zones, tracks) are taken in file
  order; only footprints and their own items are sorted.
- `row` (with `columns` for a grid) is the only alignment constraint.
- The coupled loop's congestion moves single parts; congestion does not
  feed the global placement's density.
- A board text labelling a part the placer moves stays where it is; the
  part keeps off it until the last level.
- The spectral solve is a direct O(n³) cosine transform on 64 bins; an FFT
  matters only for boards with thousands of parts.
