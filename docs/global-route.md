# The global route (`pcb-global`)

`crates/pcb-global` is a coarse global router used as a routability
measure of a placement. It reads the lowered `pcb_router::Board` (the
router's own problem) and builds no lattice: about 0.3 s per board by the
median. The router does not use it to route; the placer does, to measure.

## What it computes

- **Tiles.** Squares of `tile_mm` (1.5 mm) over the board. Each boundary
  between two tiles has, per copper layer, a capacity in tracks of the
  *reference class* (the class most nets use), measured every `sample_mm`
  (0.05 mm) between pads, keepouts and holes (`capacity.rs`). A pad the
  boundary cuts adds one track for its own net, so a pin row lying on a
  boundary does not close it to its own pins. Each tile has a via
  capacity from free via sites every 0.25 mm.
- **Pours.** A connected pour owns a tile on its layer where it covers at
  least half of it; its pads connect to the nearest owned tile, and other
  nets crossing a pour pay the router's plane cut cost.
- **Nets** (`negotiate.rs`). Every net grows a tree along its terminals in
  Prim order, each terminal an A* back to the tree within the terminals'
  box grown by `window_tiles` (6), then the whole board, in the router's
  order. A step costs the tile side times the router's direction factor
  (`against_direction` 1.6), a via `via_cost_mm` (8), both times
  (1 + history) and (1 + present × overflow). The nets negotiate
  PathFinder-style for up to `iterations` (16).
- **Output** (`pcb_global::route`): the usage per boundary and layer, the
  boundaries each net crossed, and a score: the *soft overflow* (usage
  above half of capacity), the total overflow, the overflowed nets and the
  share of nets that detour. `Incremental` re-measures the tiles around a
  moved part and routes only that part's nets again.

`GlobalConfig::default()` holds the values above. `PCB_GLOBAL_DEBUG` prints
one line per negotiation iteration.

## Where it runs

- **By default: the placer's escape demand.** With `tuning.escape_demand.measured`
  (on), the placer's refinement corrects its per-part escape estimate by
  the global route's overflow attribution: per boundary the usage above
  `threshold` (0.5) of its capacity, per part the excess on the boundaries
  its nets cross (`board_placer/measured.rs`, `measured_factory`,
  `pcb_global::Incremental`); off where most parts are fixed. See
  [placer.md](placer.md), "Escape demand".
- `ablate-kicad-placement` reports the global route's overflow and soft
  overflow per variant; `probe-trouble-placement --background global`
  weights its probe by the other nets' fill (research commands).

As a ranking of placements it ordered ours against the designer's of the
same board correctly on 0.79 of 43 pairs (RUDY 0.52), but among one
placer's seeds of a board no better than RUDY (within-board Spearman 0.33
against 0.30), and as the race's pre-ranker it lost; the race ranks by
RUDY ([placer.md](placer.md), "The race").

## Work

A measurement counts placement work from deterministic counts: 3000 units
per obstacle lowered, 5 per weighted distance test of the capacities, 30
per state the negotiation pops, at the reference's 150e6 units a second
(fitted on 4in1-mini, DSP-ADAU1452 and video, about 1.3 times the wall
time). The escape demand and the trouble probe use the same constants.
