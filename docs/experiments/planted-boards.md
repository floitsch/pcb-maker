# Boards routable by construction (planted solutions)

Experiment of 2026-10-13 (one agent, x510, cores 24-31, four boards at a
time, two threads each, 1800 s timeout; CPU seconds from rusage, including
the kicad-cli check). Code in `experiments/planted/`, data under
`build/planted/`. Question: can we generate boards with a known routing
(a feasibility certificate and an upper bound on the wiring), how does our
router do against that solution, and are the planted quantities (slack per
tile, corridor width) labels worth training on later?

## Three constructions

- **Maze** (the original recipe): scattered parts, winding waypoint routes
  on a 0.1 mm lattice, netless blockers dropped into the bends. Least
  realistic: detour p90 2.0-2.6, 2.1-2.3 vias per net, half the tiles
  used, up to 45 % of terminals dropped where planting fails, and the
  router finds much better routings, so the detour is never forced.
- **Grow**: a nucleus, then parts added one at a time beside the pads they
  connect to, into the space the routes leave, each addition's connections
  planted at once; the parts are the blockers. Most real-looking (easy
  level near the designers' and our tracer's statistics), 5-20 s per
  board, unlimited, every knob recorded in `planted.json`.
- **Derived** (Florian's alternative): netless blockers dropped into a
  routed real board (the designer's copper or our v14 result), checked with
  KiCad to be no worse than the source, blockers removed until it is. Real
  netlists and pours; the only set that exposed router failures. Limits:
  0-152 blockers fit (0-15 on crowded 4-layer boards), only BAGEL-MK1,
  MIDAS-MK2 and ErgoSNM have DRC-clean designer boards, and a solution from
  our own router is biased (rerouting reproduces it exactly).

All 80 generated or derived boards: 0 clearance or short findings and 0
unconnected under `kicad-cli pcb drc --refill-zones`.

## Realism (medians over boards)

| Source | Detour p50 / p90 | Length / MST | Vias per net | One-layer nets | Tile use p99 |
| --- | --- | --- | --- | --- | --- |
| Designers, 2 layers (58) | 1.10 / 1.35 | 1.19 | 0.95 | 0.71 | 0.62 |
| Designers, 4+ layers (50) | 1.18 / 1.56 | 1.29 | 1.03 | 0.58 | 0.68 |
| Our v14 passes (34) | 1.13 / 1.35 | 1.21 | 0.85 | 0.65 | 0.41 |
| Grow easy, 2 layers | 1.21 / 1.57 | 1.31 | 0.89 | 0.49 | 0.44 |
| Maze easy, 2 layers | - / 2.00 | 1.54 | 2.25 | - | - |

Detour is a net's copper length over the straight-line spanning tree of
its pads; tile use is track length times (width + clearance) over a 1.6 mm
tile's area. Gaps that remain in grow: pads per net p90 3 against 5-8
(buses, matrices); one-layer nets 0.49; peak tile use 0.42-0.52 where
designers reach 0.62-0.68; only 55-60 % of IC terminals on nets.

## Our router against the planted solution

Synthetic (`build/planted/measure2`), ratios ours over planted:

| Set | Complete | Length | Vias | CPU s median (max) |
| --- | --- | --- | --- | --- |
| grow L2 easy / medium / tight / dense | 16/16 | 0.96 / 0.81 / 0.77 / 0.88 | 0.25-0.35 | 22-41 (48) |
| grow L4 medium / tight / dense | 12/12 | 0.93 / 0.94 / 0.89 | 0.48-0.55 | 20-45 (63) |
| maze L2 easy / medium / tight | 12/12 | 0.86 / 0.87 / 0.82 | 0.45-0.53 | 22-49 (59) |
| maze L4 medium / tight | 8/8 | 0.88 / 0.82 | 0.54-0.55 | 50-63 (92) |

Ours is shorter with no more vias on 45 of 48 boards. Every synthetic
board is easy for the router, even the densest grow set.

Derived (`build/planted/measure`; per-net signal medians, since pour nets
routed as tracks make totals incomparable):

| Source, level | Complete | Per-net length | Vias | CPU s median (max) |
| --- | --- | --- | --- | --- |
| Designer, no blockers / blockers | 7/8, 6/8 | 0.98 / 0.99 | 0.71 / 0.65 | 296 (999), 191 (893) |
| Our v14 result, no blockers / blockers | 7/8, 6/8 | 1.00 / 1.00 | 0.86 / 0.91 | 252 (1556), 403 (947) |

The six "failures" on feasible boards all routed every net with 0 KiCad
unconnected and failed on 1-3 starved thermals beyond the source's own
(BAGEL-MK1 J102 pad 4 on an isolated island; ErgoSNM J2 pad 3; 1337-v4.0
two more). ErgoSNM with 126 blockers is the clearest case of blockers
making a board harder: connect rung 8 open, tracks rung 6, the 0.075 mm
rung complete with 3 starved thermals at 947 CPU s against 280 s without.
These are open problems 1 and 5 of the hand-over (fill fidelity, starved
thermals), not search failures.

## Labels

- A planted solution is an exact feasibility certificate and an upper
  bound on wiring, nothing more.
- Planted occupancy per tile is a weak congestion label: tile Spearman
  against our routing 0.05-0.22 on synthetic boards, 0.23 on
  designer-derived ones, 0.45 only when the solution came from our own
  router; our negotiation hot spots sit at the 60th-76th percentile of
  planted occupancy (50 would be no relation). Many routings exist, so one
  routing's slack is not where the router fights.
- Worth training on later: the router's own results on many grow boards
  (cheap, feasible, knobs recorded), once grow reaches real difficulty;
  today every grow board completes, so the label has no spread. Corridor
  and slack only as knob values, not targets.

## Carry forward

Grow as the generator, with pours on 2-layer boards, BGAs and fine pitch,
bigger boards and multi-pin buses (what makes real boards hard for us);
derived designer boards restricted to DRC-clean sources as the benchmark
with known feasibility. No network yet.

Regenerate: `experiments/planted/make_sets.sh <dir>`,
`derive_sets.sh <dir>`, `measure.py <work> --list <boards.txt>`,
`report.py <work>/results.jsonl`; statistics with `stats.py` and
`summary.py`, tile analysis with `tiles.py`, renders with `render.sh`.
