# The congestion model

A small neural network that predicts, from a placement alone and in
milliseconds, where the router's negotiation will overflow and how many
nets a placement-race probe leaves unfinished. It is meant to let the
layout's placement race judge many candidate placements instead of three,
and later to give the global placer a routability term.

Status (2026-10-11): **a closed negative as a race ranker.** Built end to
end (exporter, data, training, tract inference, race integration) and
measured; the network does not rank one board's placements better than
RUDY, a hand-written demand estimate, and against the final layouts it is
worse. What came out of it and stays:

- the race's pre-ranking, now the layout's default, with **RUDY** as the
  ranker (no model): 16 placer seeds, the best three by RUDY probed; on the
  quick tier KiCad unconnected 98 -> 64 against the plain race of three
  seeds;
- the infrastructure: the tile rasterizer (`crates/pcb-congestion`), tract
  inference, the read-only tile views of the router
  (`pcb_router::router::congestion`), the sample exporter
  (`export-congestion-sample`), the training and evaluation scripts
  (`experiments/congestion`), the training-board harvest.

Why it did not beat RUDY: the race needs a ranking *within* one board,
among placements of the same parts that differ in detail, and the only
affordable judge to learn from is a 75 s probe. That judge is noisy: one
probe agrees with another under a different router seed at Spearman 0.44
(23 held-out boards; 0.61 on a first 6), and against the final layout
(1800 s, KiCad's unconnected count) a probe reaches about 0.55, the same
as a second probe or the mean of two. The network learned what is easy in
that label, the board-to-board scale of the unfinished count (Spearman
0.75-0.84 across boards), but within a board it reached 0.27-0.37 against
the probe (RUDY 0.35-0.40) and 0.13 against the finals (RUDY 0.30,
wirelength 0.23). See "Results".

A second attempt would need a better judge than the probe to learn from:
the final layouts themselves as labels (1800 s a sample, so thousands of
CPU-hours for a training set), or a probe statistic that tracks the final
result clearly better than 0.55 (none of the probe's own statistics does:
"Which probe statistic predicts the final result").

## Representation

Everything lives on the router's own tiles: `TILE` x `TILE` (16 x 16)
lattice nodes, 1.6 mm at the 0.1 mm pitch, with the lattice the router
would choose for the board (`pcb_router::router::congestion::tile_frame`:
neck classes added, then `Grid::choose`). Features and labels therefore
line up tile for tile.

Copper layers go to four *slots*, the same for every board: front, back,
the first inner layer, and the remaining inner layers averaged. A two-layer
board leaves the inner slots empty; `layer_present` says which slots
exist. The router prefers horizontal steps on even layers and vertical on
odd ones, so training augments with flips only: a 90-degree turn would hand
a layer the other preferred direction.

Input channels (`crates/pcb-congestion/src/features.rs`, 32 in all; all
computed from the lowered `pcb_router::Board`, no router built):

| Channel | Per | Meaning |
| --- | --- | --- |
| `pins` | slot | terminals (pads with a net) on the layer |
| `pad_copper` | slot | share of the tile's nodes inside net copper |
| `obstacle` | slot | share inside keepouts, holes, netless copper, or outside the outline |
| `pour` | slot | share inside a pour polygon |
| `layer_present` | slot | 1 where the slot has a layer |
| `inside` | board | share of the tile inside the outline (the model's board mask) |
| `rudy` | board | RUDY: each signal net's spanning-tree length times its track pitch (width + clearance) spread uniformly over its bounding box (at least a tile wide) |
| `rudy_mst` | board | the same per spanning-tree edge over the edge's box |
| `pin_rudy` | board | pin RUDY: per pin, (w + h) / (w h) of its net's box times the track pitch |
| `rudy_pour` | board | RUDY of nets with a connected pour (their pads reach the pour by vias) |
| `nets_here`, `through_pins`, `pins_scaled` | board | distinct nets, through-hole terminals, all terminals per tile |
| `layer_count`, `track_pitch`, `via_pitch`, `tile_mm` | board | board constants (layers / 4; the most used class's track and via pitch over the tile size; tile size / 1.6 mm) on the board's tiles |

RUDY and the pin counts are differentiable in part positions (box
overlaps and indicator sums), which is what a global-placement term would
need.

## Labels

`pcb-maker export-congestion-sample` lowers a placement exactly as the
layout does (planned pours added, copper texts removed), and runs
`Router::probe` with the race's router configuration (the layout's
`stall_drop`, `stall_at_cap`, `present_growth`, `stall_patience`) for
`probe_seconds` of work (75, the race's budget). It then reads the router
state on the tiles (`Router::congestion_labels`, a read-only child module
of `router`):

| Label | Meaning |
| --- | --- |
| `conflict` | nodes of a net's branches inside another net's clearance zone at the end (the overflow negotiation left), per layer and the via plane |
| `history` | accumulated node history (conflict hits) summed per tile |
| `tile_history` | the corridor planner's per-tile history |
| `usage` | route nodes per tile and layer; vias in the last plane |
| `claimed`, `routable` | claimed and routable shares of the most used class |
| `open_terminals` | terminals of unfinished nets |
| scalars | `unfinished` (the race's own measure, `Router::unfinished`), conflicted and incomplete nets, iterations, expansions; per-net flags |

The map label is `conflict` (in nodes per tile). `history` makes a denser,
easier map (held-out precision 0.41 and recall 0.70 for tiles above one
node, against 0.27-0.33 and 0.29-0.44), but a network trained on it ranks
placements worse (within-board Spearman of its scalar 0.09-0.29 against
0.19-0.37). The scalar head is trained on `ln(1 + unfinished)`.

**Label noise.** The same 8 placements of 6 held-out boards probed again
with `PCB_ROUTER_SEED=7` rank alike within a board with Spearman 0.86
(4in1-mini), 0.94 (d20), 0.78 (OpenAirScope), 0.67 (SNSP-1CHIP), 0.38
(Explorer), 0.02 (OpenFC): mean 0.61. On all 23 held-out boards whose
seeds differ (16 seeds each, router seed 1 against none) the mean is 0.44
(-0.24 on capybully to 1.00 on Blue_Line). No predictor can agree with a
single probe much better than that, and the race's own pick among three
probes is partly luck.

Probe budget: on a pilot of 18 boards x 7 placements, the unfinished
count after 25 s of work ranks placements of one board like the count
after 75 s (Spearman 0.994 within boards; 10 s: 0.92). Most probes end on
the stall rules long before the budget (mean 11.2M expansions at 25 s,
13.9M at 75 s), so samples use the race's 75 s and the label is exactly
the race's criterion.

## Data

Boards: the 109 harvested benchmark boards, the 617 PCBench boards
(`benchmarks/pcbench`), and 686 more GitHub boards harvested for training
only (`harvest.py fetch --training`). Whole boards are held out by a hash
of the name (15 %), plus 9 boards the race exercised in v11: 226 held-out
boards, 23 of them harvested boards whose placements differ.

Placements per board (`experiments/congestion/prepare.py`): the designer's;
designer perturbations (5/15/40 legal moves with swaps and quarter turns,
up to 2/5/10 mm, two seeds each); two shuffles (every part at a random spot,
legalized: bad on purpose); our placer from stacked parts with every
footprint free (`move_all`; 3-4 seeds) and two perturbations of seed 1; and
for the 109 benchmark boards the task exactly as `benchmarks/agent-tasks/
run.py` stacks it (with the 2026-10-10 unplace fix) at 16 placer seeds
(held-out boards) or 8 (training boards): the race's candidates. Generation
runs on spare cores at nice 19, one search thread per exporter, under the
run.py memory gate with 20 GB kept free.

Per sample (x401): lowering median 22 ms, features 33 ms, probe 55 s wall
(p90 134 s; 75 s of work = 150M expansions at most). Unfinished nets:
median 24.5, 0 in 17 % of the samples. `conflict` is nonzero in 0.2 % of a
board's tiles (median), `history` in 11 %.

Found on the way: `run.py`'s `unplace` read a pad's net only in KiCad 10
syntax, so 89 of the 109 layout tasks had every part fixed (fixed in
baa5817).

## Model and training

`experiments/congestion/model.py`: normalisation inside the graph (log1p
for count and demand channels, then per-channel mean and standard
deviation from the training boards, zero off the board), a three-level
U-Net (widths w, 2w, 4w; GroupNorm, SiLU), and two 1x1 heads: overflow per
tile and plane (4 slots plus vias, in `ln(1 + label)`), and an open density
per tile whose sum is the predicted number of unfinished nets. Loss: Huber
on the map (tiles that overflow weigh five times as much), Huber on
`ln(1 + unfinished)`, and optionally a pairwise logistic ranking loss
between placements of the same board (`--by-board --rank-weight`).

Width 24 (530k parameters) trains 30 epochs in 2-3 minutes on the
GTX 1650 (batches of one board's placements, flips). Variants measured on
the held-out boards: the ranking loss helps (without it the scalar's
within-board Spearman drops from about 0.2-0.37 to 0.07-0.27); width 16
(235k) is as good as 24; the spread between training seeds of one
configuration is as large as the differences between configurations, so
the shipped experiment is an ensemble of four seeds exported as one ONNX
graph (`export_ensemble.py`), trained without the quick tier's boards.

## Results

Held-out boards, all placement kinds (about 1000 samples, 84-93 boards;
lower score = better placement; truth: the probe's unfinished nets):

| Ranker | within-board Spearman | pair accuracy | across boards |
| --- | ---: | ---: | ---: |
| network, scalar head | 0.27-0.37 | 0.62-0.68 | 0.75-0.84 |
| network, map sum | 0.24-0.38 | 0.62-0.67 | 0.54-0.79 |
| network + RUDY (rank sum) | 0.42 | 0.69 | |
| RUDY over capacity | 0.35-0.40 | 0.65-0.66 | 0.46 |
| wirelength | 0.29 | 0.62 | 0.66 |

The race simulated on the 23 held-out boards' 16 task seeds (sum of the
best probe among those the race probes; today's race probes seeds 1-3):

| Ranker | best of its top 1 | best of its top 3 | its best in the probe's best 3 | probe's best in its top 3 |
| --- | ---: | ---: | ---: | ---: |
| today (seeds 1-3) | | 2015 | | |
| random seeds (expected) | 2414 | 1865 | 0.25 | 0.33 |
| network (one model) | 1934-2139 | 1568-1796 | 0.43-0.65 | 0.39-0.65 |
| network, 4-seed ensemble | 1997 | 1585 | 0.52 | 0.57 |
| network + RUDY | 2023 | 1553 | 0.35 | 0.57 |
| RUDY over capacity | 2043 | 1621 | 0.43 | 0.48 |
| wirelength | 2071 | 1709 | 0.48 | 0.26 |
| oracle | | 1466 | | |

Tile map (conflict nodes per tile, held out): MAE 0.23-0.48 nodes;
tiles above one node: precision 0.27-0.33, recall 0.29-0.44.

Quick tier, one binary (x402), 36 boards all runs have (31 of 37 race):

| | plain race (3 seeds) | RUDY, top 3 of 16 | network + RUDY, top 3 of 16 |
| --- | ---: | ---: | ---: |
| best race probe (sum) | 204 | 189 | 153 |
| open after the first route | 149 | 112 | 108 |
| KiCad unconnected | 98 | 64 | 64 |
| passes | 16 | 18 | 15 |
| boards better / worse (unconnected) | | 6 / 2 | 6 / 1 |
| placement wall (37 boards) | 2792 s | 3978 s | 2900 s |

Pass counts move on starved thermals and single placement findings
(items_not_allowed, courtyards_overlap) of particular seeds, not on
routing. Where the pre-ranking lost: on reCamera RUDY's race found a
better probe (10 against 14) but the board resumed from it worse (first
route 15 open against 8, KiCad 6 against 4): the probe misjudged, not the
ranker; RUDY was also 0 for 9 of its 16 placements there (no tile above a
quarter of capacity), so ties went to the placer's order. On USB_Keypad
the ranker's three probed 6/5/6 against the plain race's 6/4/4 (KiCad 4
against 2): a slightly worse pick, within the probe's noise (the same
seeds probed by the exporter: 5-13).

## Which probe statistic predicts the final result

92 final layouts (2026-10-10, x401, run.py, 1800 s each): on 23 held-out
boards the two best and the two worst of 16 placer seeds by one probe,
each laid out alone (no race). 82 have a KiCad result, 15 boards have
finals that differ. Within-board Spearman against the final KiCad
unconnected count, pair accuracy (81 pairs), and the regret of the seed
the statistic picks (final unconnected above the best of the four):

| Statistic | Spearman | pairs | regret |
| --- | ---: | ---: | ---: |
| the probe the seeds were selected by (biased by the selection) | 0.451 | 0.679 | 2.47 |
| one independent probe (router seed 1 / 2) | 0.526 / 0.568 | 0.735 / 0.747 | 2.60 / 2.87 |
| mean of the two independent probes | 0.546 | 0.741 | 1.73 |
| the probe's conflicted nets | 0.503 | 0.685 | 3.73 |
| the probe's incomplete nets | 0.486 | 0.679 | 13.2 |
| open terminals after the layout's first route (not available to a race) | 0.555 | 0.778 | 2.67 |
| RUDY over capacity | 0.300 | 0.630 | 7.33 |
| wirelength | 0.234 | 0.593 | 7.80 |
| the network (4-model ensemble) | 0.130 | 0.556 | 8.60 |

The probe's best two seeds finish better than its worst two on 11
boards, the same on 6, worse on 2: the probe is a real judge, the best
available before routing, but averaging two probes (at the same work, two
half-budget probes would do: 25 s and 75 s probes rank alike at 0.994)
does not make it a clearly better one, so the race keeps one probe a
placement.

## Integration

`KiCadBoardLayoutConfig` (layout.json) has `congestion_score` (`rudy`, the
default; `none` for the plain race; with `congestion_model`, an ONNX file:
`open`, `overflow` or `open+rudy`), `congestion_candidates` (16) and
`congestion_probes` (3). The placer makes `congestion_candidates` seeds; the kept placement and every other
seed's legal placement that keeps the constraints as well are lowered,
rasterized and scored (`crates/pcb-kicad/src/congestion_rank.rs`); the
race then probes only the best `congestion_probes`, as before.
`board-layout.json` records the scores and the probed placements
(`congestion`). Latency: features 7-70 ms, inference 5-58 ms on 24 x 24 to 107 x 79
tiles (one network; the first call on a size builds the plan, 66-125 ms;
the 4-network ensemble 18 ms on OpenFC), lowering a placement 18-440 ms
(the largest part on big boards). tract matches PyTorch to 2e-4 relative
on real samples.

## How to retrain and export

```sh
# 1. Boards and jobs (sources/, jobs/, split.json with the held-out boards)
experiments/congestion/prepare.py build/cdata --sets github pcbench training
# 2. Samples: taskset 16-31, nice 19, one search thread, 20 GB kept free
experiments/congestion/generate.py build/cdata --binary build/bin/pcb-maker-x401 --jobs 16 --probe-seconds 75
# 3. Training (GPU; venv: python -m venv build/venv-nn; pip install torch onnx onnxscript numpy)
build/venv-nn/bin/python experiments/congestion/train.py build/cdata --out build/cdata/models/NAME --width 24 --by-board --rank-weight 1
# 4. Held-out metrics, baselines and the simulated race
build/venv-nn/bin/python experiments/congestion/evaluate.py build/cdata --model build/cdata/models/NAME
build/venv-nn/bin/python experiments/congestion/evaluate.py build/cdata --model build/cdata/models/NAME --variants task-seed
# 5. tract against PyTorch on real samples
build/venv-nn/bin/python experiments/congestion/check_tract.py build/cdata/models/NAME build/bin/pcb-maker-x401 build/cdata/samples/*/designer.json
```

`train.py` writes `model.pt` and `model.onnx` (input `features`
`[1, 32, H, W]`, H and W multiples of 8; outputs `overflow`
`[1, 5, H, W]` and `open` `[1, 1]` = `ln(1 + unfinished)`). The Rust side
pads to multiples of 8 and caches one optimised tract plan per size.
`experiments/congestion/export_tiny.py` regenerates the fixed test model
of `crates/pcb-congestion` (`tiny_model_matches_pytorch`).

More boards for training only: `benchmarks/github/harvest.py fetch
--training` (relaxed criteria, into `benchmarks/real/external/training`,
origins and licences in `benchmarks/github/training-manifest.json`).

## Limits

- The label: one probe is a noisy judge (0.44-0.61 between router seeds,
  about 0.55 against the final layout), and it is what the network learns.
- The network does not beat RUDY within a board; its advantage is across
  boards, which the race never needs.
- RUDY ties at 0 on uncrowded boards (9 of 16 placements on OpenFC and
  reCamera); breaking the ties by demand above a tenth of the capacity
  changed the probed seeds but not one final row (pinned pairs on both
  boards; offline race@3 1601 against 1601), so ties keep the placer's
  order.
- 6- and 8-layer boards share two inner slots (averaged).
- Flips only, no quarter turns (the router's preferred directions).
- GroupNorm sees the zero padding of a batch; inference pads to multiples
  of 8 only (a small train/inference mismatch).
