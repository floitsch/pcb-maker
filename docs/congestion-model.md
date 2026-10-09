# The congestion model

A small neural network that predicts, from a placement alone and in
milliseconds, where the router's negotiation will overflow and how many
nets a placement-race probe leaves unfinished. It is meant to let the
layout's placement race judge many candidate placements instead of three,
and later to give the global placer a routability term.

Status: RESULTS_PLACEHOLDER

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

LABEL_CHOICE_PLACEHOLDER

Probe budget: on a pilot of 18 boards x 7 placements, the unfinished
count after 25 s of work ranks placements of one board like the count
after 75 s (Spearman 0.994 within boards; 10 s: 0.92). Most probes end on
the stall rules long before the budget (mean 11.2M expansions at 25 s,
13.9M at 75 s), so samples use the race's 75 s and the label is exactly
the race's criterion.

## Data

DATA_PLACEHOLDER

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

MODEL_PLACEHOLDER

## Results

RESULT_TABLES_PLACEHOLDER

## Integration

`KiCadBoardLayoutConfig` (layout.json) has `congestion_model` (path to an
ONNX file; off when absent), `congestion_candidates` (16), `congestion_probes`
(3) and `congestion_score` (`open` or `overflow`). With a model, the placer
makes `congestion_candidates` seeds; the kept placement and every other
seed's legal placement that keeps the constraints as well are lowered,
rasterized and scored (`crates/pcb-kicad/src/congestion_rank.rs`); the
race then probes only the best `congestion_probes`, as before.
`board-layout.json` records the scores and the probed placements
(`congestion`). Latency: LATENCY_PLACEHOLDER

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

LIMITS_PLACEHOLDER
