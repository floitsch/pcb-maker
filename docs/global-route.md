# The global route (`pcb-global`)

Status (2026-10-21): **phase 1, a routability measure.** Built (a crate,
a command, an opt-in race score) and validated against final results on
two sets of placements. It tells our placements from the designers'
reliably where RUDY cannot, but it does not rank the placer's own seeds
of one board better than RUDY, and well below the router's 75 s probe.
Details under "Validation"; the prototype in the placement race under
"Prototype".

## What it computes

`crates/pcb-global` reads the lowered `pcb_router::Board` (the router's
own problem: outline, obstacles, nets and terminals, rule classes, pours)
and builds no lattice.

**Tiles.** Squares of `tile_mm` (1.5 mm: about four track pitches) over
the box of the outline and the terminals. Each boundary between two
neighbouring tiles has, per copper layer, a capacity in tracks of the
**reference class** (the class most nets with two or more terminals
use); each tile has a via capacity.

**Boundary capacity** (`capacity.rs`). Samples every `sample_mm`
(0.05 mm) along the boundary line; a sample is free on a layer where a
track centreline of the reference class may run: inside the outline by
the edge clearance plus half a track, and away from every obstacle on
the layer by its clearance plus half a track (copper: the larger of the
class's and the obstacle's clearance, as `Board::copper_clearance`;
keepouts: their own; holes: the hole clearance). Obstacles are pads of
every net (the capacity is what is left for passing nets), copper of no
net, keepouts and rule areas, holes. A run of free samples between two
obstacles holds `floor(length / pitch) + 1` tracks (exact for a gap
between two pads), a run along the whole boundary `length / pitch`, a
run from one end to an obstacle `length / pitch + 0.5` (a gap the tile
corner splits shares its `+ 1`); the sum is rounded to whole tracks.
**A pad the boundary cuts adds one track**: its own net's escape crosses
there through the pad. Without this a pin row lying on a boundary closed
it to its own pins: the designer's placement of video, whose QFP rings
sat on boundaries, overflowed 320 tracks against our placement's 226
(its final route is complete, ours leaves 199 open); with it 0 against 7.
`boundary_lines: 3` measures on the boundary and on the lines a quarter
tile into either tile, and takes the mean (obstacles inside the tiles
count; not the default, see the sensitivity table).

**Via capacity.** Sites every `via_sample_mm` (0.25 mm) inside the tile
where the reference class's via keeps every obstacle on every layer
(holes: the hole clearance from the via, and hole to hole from its
drill); the free share of the tile over the via pitch squared.

**Pours.** A connected pour (`Plane::connect`) owns a tile on its layer
where it covers at least half the tile (4 x 4 samples, higher-priority
zones cut out). Its net's pads connect to it: a pad on a tile the pour
owns needs nothing; another pad searches to the nearest owned tile (a
via under an SMD pad over an inner plane). Other nets crossing a pour pay
the router's plane cut cost, scaled per layer by its pour share as the
router scales it (`layer_cut`).

**Nets** (`negotiate.rs`). Every net with two or more terminals or a
pour. A terminal is its anchor's tile on each of its layers (a through
pad on all of them). A signal net grows a tree along its terminals'
rectilinear spanning tree (Prim order): each terminal searches back to
the tree, A* guided towards its spanning-tree parent, within the
terminals' box grown by `window_tiles` (6) or a quarter of its larger
side, then the whole board. Nets are routed in the router's order (pour
nets first, then nets of up to eight terminals, by span). A net of
another class demands its pitch over the reference pitch in tracks.

**Costs.** A step to a neighbouring tile costs the tile side times the
layer's direction factor (even layers prefer horizontal, odd vertical;
`against_direction` 1.6 as the router's), times the pour factor of the
tile entered, times `(1 + history) * (1 + present * over)`, where `over`
is how far the boundary would be over capacity with this net. A via (a
layer change in a tile) costs `via_cost_mm` (8, the router's), priced
alike on the tile's via sites. A boundary without capacity is closed,
except next to the net's own terminals (a net can always leave its
pads' tiles, overflowing if need be). A terminal that a search over the
whole board does not reach is unreachable for good (closed boundaries do
not open) and not searched again.

**Negotiation** (PathFinder). Every net routed; then up to `iterations`
(16) rounds: boundaries and via sites over capacity gain
`history_increment` (1) of history, the present price grows by
`present_growth` (1.8, from 0.5), and only the nets crossing an
overflowing boundary or via site are ripped up and routed again. The
iteration with the least overflow is kept.

**Output** (`GlobalRoute`, `Score`). Per boundary and layer the usage
and the overflow (`GlobalRoute::overflow`), per net its route (states,
boundaries crossed, vias) and its corridor (`GlobalRoute::corridor`, the
tiles it crossed), and the score:

| Field | Meaning |
| --- | --- |
| `total_overflow`, `max_overflow`, `overflowed_edges` | tracks over capacity summed, on the worst boundary, boundaries over |
| `via_overflow` | vias over the tiles' via sites, summed |
| `overflow_per_layer`, `usage_per_layer`, `capacity_per_layer` | per layer |
| `soft_overflow` | usage above 0.5 and 0.75 of the capacity, summed (continuous where nothing overflows) |
| `ace` | mean utilisation of the 0.5, 2 and 5 % most used boundaries (ISPD's ACE) |
| `overflowed_nets`, `unrouted` | nets crossing an overflowing boundary or via site; nets with an unreachable terminal |
| `detoured`, `detour_fraction` | signal nets crossing more boundaries than their terminals' spanning tree times 1.25 plus 2 |
| `wirelength_mm`, `vias`, `iterations`, `best_iteration` | |

**Command.** `pcb-maker global-route-kicad-board <dir> <board-id>
[router-config.json|auto] [--global <global.json>] [--probe
<work-seconds>] [--corridors <out.json>]` lowers the board as the layout
routes a placement (tracks stripped, planned pours added, copper texts
removed), routes it globally and prints the score, the race's RUDY score
of the same lowering, the times and, with `--probe`, the race's router
probe (`Router::probe` with the race's configuration); `--corridors`
writes the overflow per layer and tile and every net's corridor.
`global.json` holds `GlobalConfig` (the defaults above; all keys
optional).

**Time** (x1440, one thread, load 40-60 from other agents): median 0.26 s
a placement over the 218 of the validation, 90th percentile 2.5 s; katia
2.6 s (2.2 s of it the capacities: footprint polygons of thousands of
points measured per sample), Batch_1 3.2 s, the laptop motherboard
(6 layers) 0.24 s. Lowering the board takes 0.1-0.6 s more. RUDY takes
0.1-0.3 s.

## Validation

`experiments/global-route/validate.py` (run with `python3 -I`).

**Set 1: our placement against the designer's**, every board of tasks
v7: ours is the v18 layout's final placement
(`build/github-layout-v18{c,b,}/<task>/layout/placed`, the placement the
final route ran on), the designer's the designer tier's input
(`build/designer-x1315{b,}/<task>/placed`); final results from
`build/github-layout-v18c/results.json` and `build/designer-x1315b/
results.json` (KiCad's unconnected items, the strict pass). 218
placements, 209 with a final result; 43 boards whose two finals differ
in open connections; 18 boards that pass the strict rule on one
placement only (13 the designer's, 5 ours).

```sh
python3 -I experiments/global-route/validate.py run build/global-route-dev2a --binary build/bin/pcb-maker-x1440 --jobs 6
python3 -I experiments/global-route/validate.py run build/global-route-probes --binary build/bin/pcb-maker-x1440 --probe 75 --jobs 6
python3 -I experiments/global-route/validate.py analyse build/global-route-dev2a --probes build/global-route-probes
```

| Statistic (default variant) | pooled Spearman (209) | pair accuracy (43 pairs) | as a within-board Spearman | strict-discordant right (of 18) |
| --- | ---: | ---: | ---: | ---: |
| **soft overflow at 0.5** | 0.381 | **0.791** | 0.581 | 12 |
| soft overflow at 0.75 | 0.392 | 0.767 | 0.535 | 13 |
| ACE 5 % | 0.369 | 0.744 | 0.488 | 12 |
| overflowed nets | 0.498 | 0.651 | 0.302 | 10 |
| detour fraction | 0.537 | 0.640 | 0.279 | 8.5 |
| total overflow | 0.424 | 0.581 | 0.163 | 9 |
| max overflow | 0.409 | 0.558 | 0.116 | 10.5 |
| the global route's wirelength | 0.251 | 0.663 | 0.326 | 10 |
| RUDY over capacity (the race's) | 0.309 | 0.523 | 0.047 | 12.5 |

(Ties count half. With two placements a board, a within-board Spearman
is +1 or -1 per pair: its mean is twice the pair accuracy less one.
Pooled Spearman mixes boards and mostly measures board size.)

The race's 75 s probe on the same placements (`--probe 75`, one thread,
x1440-x1442's router, which is x1385's): 149 of the 218 placements, the
boards whose v18 layout peaked below 3 GB first; the rest were not
probed (memory was exhausted on the machine, and the tier lock had a long
queue). On the 141 placements with a probe and a final, and the 23 pairs
among them whose finals differ:

| Statistic | pooled Spearman (141) | pair accuracy (23) | strict-discordant right (of 11) |
| --- | ---: | ---: | ---: |
| soft overflow at 0.5 | 0.330 | **0.783** | 7 |
| overflowed nets | 0.420 | 0.717 | 6 |
| the 75 s probe | **0.811** | 0.739 | 6 |
| RUDY | 0.294 | 0.543 | 7.5 |

Across boards the probe tracks the scale of the open count (it routes);
within a pair (ours against the designer's) the global route is as good
or better, in a hundredth of the time and without a router's memory.

**Set 2: the placer's seeds of one board** (the race's question): the
historical cohort where the probe's 0.55 and RUDY's 0.30 were measured
(docs/congestion-model.md, "Which probe statistic predicts the final
result"): 23 held-out boards, the two best and two worst of 16 placer
seeds by one probe, each laid out for 1800 s by x401; 82 with a KiCad
result. The placements as the race probed them are the reconstructed
ones (`build/label-audit/initial-features/<name>/source`); 73 of them
match the probed input exactly. Within-board Spearman (mean over boards
whose finals differ), pair accuracy over pairs of one board with
different finals, and the regret of the seed the statistic picks
(finals.py's measures, same code):

```sh
python3 -I experiments/global-route/validate.py finals-run build/global-route-finals-a --binary build/bin/pcb-maker-x1440
python3 -I experiments/global-route/validate.py finals build/global-route-finals-a
```

| Predictor (all 82) | within-board Spearman | pair accuracy (81 pairs) | regret |
| --- | ---: | ---: | ---: |
| the selecting probe | 0.451 | 0.679 | 2.47 |
| one independent probe | 0.526 | 0.735 | 2.60 |
| mean of two independent probes | 0.546 | 0.741 | 1.73 |
| RUDY over capacity | 0.300 | 0.630 | 7.33 |
| the placer's wirelength | 0.234 | 0.593 | 7.80 |
| **global: soft overflow at 0.5** | **0.327** | **0.642** | 13.47 |
| global: overflowed nets | 0.312 | 0.636 | 12.87 |
| global: total overflow | 0.230 | 0.580 | 21.07 |
| global: max overflow | -0.026 | 0.500 | 23.27 |
| global: ACE 5 % | 0.152 | 0.556 | 15.07 |
| global: detour fraction | -0.105 | 0.463 | 9.60 |
| global: wirelength | 0.092 | 0.519 | 14.60 |

On the 73 matched placements: soft overflow 0.332 / 0.627, overflowed
nets 0.370 / 0.649, the selecting probe 0.561 / 0.716, an independent
probe 0.464 / 0.746, RUDY 0.201 / 0.612. Within one board the global
score agrees with the probe at 0.29 (Spearman, mean over 20 boards),
RUDY at 0.25, the wirelength at 0.27, one probe with another at 0.56.

**Sensitivity** (soft overflow at 0.5; set 1 pair accuracy and strict
right of 18, set 2 within-board Spearman and pair accuracy over the 82):

| Variant | set 1 pairs | set 1 strict | set 2 Spearman | set 2 pairs |
| --- | ---: | ---: | ---: | ---: |
| default (tile 1.5 mm, boundary line, full capacity) | 0.791 | 12 | 0.327 | 0.642 |
| `capacity_share` 0.7 | 0.744 | 12 | 0.224 | 0.580 |
| `tile_mm` 1.0 | 0.698 | 12 | 0.092 | 0.543 |
| `tile_mm` 2.0 | 0.733 | 13.5 | 0.263 | 0.593 |
| `boundary_lines` 3 | 0.791 | 13 | 0.015 | 0.469 |

The default was fixed (and the soft overflow at 0.5 named the primary
measure) after the first two rows of set 1 and before set 2 was scored;
the other variants were measured afterwards and are reported, not
chosen from.

**What it says.** Two different questions:

- *Is our placement less routable than the designer's?* The global
  route answers it well and robustly: 0.70-0.79 of the 43 pairs ordered
  as their finals at every tile size and capacity measure, against 0.52
  for RUDY (whose demand-over-capacity cannot see the layers, the pads
  between which tracks must pass, or that a designer's compact placement
  has short nets) and 0.66 for the global route's own wirelength. On the
  18 boards that pass on one placement only it is right on 12 (13-14 in
  variants); 7 of the 18 differ only in starved thermals or other
  findings with no open connection on either side, where no routability
  measure can be right but by chance; on the 11 decided by open
  connections it is right on 8.
- *Which of the placer's seeds routes best?* The race's question. Here
  the global route is at RUDY's level (0.33 against 0.30 within-board
  Spearman, 0.64 against 0.63 pair accuracy), far below one probe (0.53,
  0.74), and unstable: the variants move it between 0.0 and 0.33 while
  they leave set 1 at 0.70-0.79. Seeds of one placer differ in details
  that 1.5 mm tiles with three or four tracks a boundary do not resolve:
  which pin row a boundary cuts, a part a millimetre over. The probe
  differs from seed to seed for reasons a coarse model cannot see (pin
  access, thermal spokes, the router's own search) and those decide the
  final few open connections.

So the decision rule of phase 1 (a Spearman above 0.55, or the paired
ranking right on most of the 18) holds on the second clause only. The
measure is fit for what the placement gap needs (telling a routable
structure from an unroutable one, cheaply), not for picking among
similar seeds; for that the probe stays.

## Prototype: the placement race ranked by the global route

`congestion_score: "global"` in layout.json (x1442, opt-in): the race's
pre-ranking scores the placer's legal seeds by the global route's soft
overflow at 0.5 instead of RUDY, and probes the best three as before
(`congestion_rank.rs`; one hook in `board_layout.rs`, which let the
pre-ranking run for this score). Cost: 0.1-0.4 s for all candidates on
these boards. Layouts of the eleven dense boards at the task budgets
(tasks v7, 2 x 2, run.py `--layout '{"congestion_score": "global"}'`),
x1385 (main 043c247) against x1442 (043c247 plus this package; the
router and the default layout are unchanged): build/global-dense-x1385,
build/global-dense-x1442 (4in1-mini, 4in1) and -x1442b (the other nine,
under the tier lock). No row cut, no row lost to memory.

| Board | seeds kept | probed: RUDY / global (best race probe) | KiCad unconnected x1385 / x1442 | vias |
| --- | ---: | --- | ---: | ---: |
| video | 3 | all three, no pre-ranking | 196 / 196 | 1670 / 1670 |
| OpenESC 4in1 | 3 | all three | 111 / 111 | 145 / 145 |
| Glasgow | 3 | all three | 170 / 170 | 221 / 221 |
| SmartSpin2k | 3 | all three | 39 / 39 | 1139 / 1139 |
| MegaDrive | 5 | 1, 2, 0 / 0, 1, 2 (313 both) | 201 / 201 | 1864 / 1864 |
| SNSP-CPU-01 | 16 | 0, 9, 1 / 6, 0, 1 (321 both: seed 0) | 133 / 133 | 1686 / 1686 |
| split72_left | 7 | 2, 6, 5 / 1, 2, 3 (180 both: seed 2) | 40 / 40 | 872 / 872 |
| protoesp | 11 | 1, 4, 2 / 1, 0, 2 (84 both: seed 1) | 35 / 36 | 381 / 350 |
| 4in1-mini | 4 | 0, 3, 2 (113) / 2, 1, 3 (86) | **57 / 74** | 165 / 131 |
| DSP-ADAU1452 | 10 | 0, 1, 3 (207) / 7, 3, 2 (178) | **49 / 56** | 268 / 279 |
| 0xCB panel | 10 | 2, 0, 7 (61) / 6, 8, 0 (173) | **65 / 84** | 247 / 234 |
| sum | | | **1096 / 1140** | |

Strict passes 0 / 0. Four boards keep only three seeds within the
placement budget, so there is nothing to pre-rank (the same rows to the
via). Three keep the same seed and the same row. protoesp keeps the same
seed, but the race's other two probes spent other work, so the first
route's continuation had 583 s of work instead of 579 and ended one open
connection worse. On three boards the global route probed other seeds:
on 4in1-mini and DSP-ADAU1452 it found seeds whose race probe was better
(86 against 113, 178 against 207 unfinished nets) that finished worse
(the probe's own misjudgement: 0.55 against the finals); on the 0xCB
panel it ranked out the seed RUDY found (61 unfinished, the best by far)
and kept one at 173. As set 2 predicts: among seeds of one board the
global route is no better a pre-ranker than RUDY. Not a default; the
option stays as an experiment.

## Next

- Use the measure where it is strong: in the placer's search (an anneal
  term re-evaluated every few thousand moves, or moves of the parts whose
  nets cross the most used boundaries), where placements differ in
  structure, not between seeds.
- Corridors for the detailed router: `GlobalRoute::corridor` is ready;
  the router's own tile planner (`router/planner.rs`) and its older
  in-router global plan (`pcb_router::global`, `global_routing`, off)
  could take them.
- Capacity: pin access inside a tile (pads not on a boundary cost
  nothing now), vias taking track room, the fine-pitch classes (neck
  width) instead of the reference class everywhere; the capacities of
  boards with large footprint polygons (katia) could be measured per
  edge interval instead of per sample.
