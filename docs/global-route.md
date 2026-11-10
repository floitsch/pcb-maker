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
- **Output** (`pcb_global::route`): the overflow per boundary and layer,
  each net's corridor, and a score: the *soft overflow* (usage above half
  of capacity, the primary measure), the total and largest overflow and
  the overflowed nets. `Incremental` re-measures the tiles around a moved
  part and routes only that part's nets again.

`GlobalConfig::default()` holds the values above. `PCB_GLOBAL_DEBUG` prints
one line per negotiation iteration.

## Where it runs

- **By default: the placer's escape demand.** With `escape_demand.measured`
  (on), the placer's refinement corrects its per-part escape estimate by
  the global route's overflow attribution (`board_placer/guidance.rs`
  `measured_factory`, `pcb_global::Incremental`); off where most parts are
  fixed. See [placer.md](placer.md).
- `pcb-maker global-route-kicad-board` (a research command) prints the
  score, RUDY and optionally a router probe.
- Layout `congestion_score: "global"` ranks the placement race by it
  instead of RUDY (default `rudy`).
- `probe-trouble-placement --background global` and
  `ablate-kicad-placement` use `Incremental` to score moves.

Why it is not the race's ranking: the soft overflow orders our placement
against the designer's of the same board correctly on 0.79 of 43 pairs
(RUDY 0.52), but among one placer's seeds of a board it ranks no better
than RUDY (within-board Spearman 0.33 against 0.30; one 75 s router probe
0.53). As the race's pre-ranker it went 1096 to 1140 KiCad unconnected on
the eleven dense boards.

## Phase 2: the global route inside the placer's search (off)

`global_guidance` (placer config, default unset, off;
`KiCadGlobalGuidance` in `board_placer/guidance.rs`, the anneal term in
`crates/pcb-placer/src/guidance.rs`) feeds the measure into the search:

- The anneal's poses are written into the board, lowered and routed by
  `pcb_global::route`. Per boundary the usage above `threshold` (0.5) of
  its capacity counts; per tile that excess is the *field*, per part the
  excess on the boundaries its nets cross is its *attribution*.
- Between two evaluations a net's wirelength is multiplied by
  1 + `weight` (1) × the mean field over its pins' box; a share `bias`
  (0.5) of the moves draws its part by attribution. An evaluation is due
  after `min_accepted` (200) accepted moves while evaluations stay within
  `work_share` (0.1) of the anneal's work.
- With `moves` (on with the guidance) the coupled loop tries parts in the
  order of their attribution.

**Work model.** An evaluation counts placement work from deterministic
counts: 3000 units per obstacle lowered, 5 per weighted distance test of
the capacities, 30 per state the negotiation pops, at the reference's
150e6 units a second (fitted on 4in1-mini, DSP-ADAU1452 and video, about
1.3 times the wall time). Escape demand and the trouble
probe use the same constants.

**Verdict: no-go as a default.** On the eleven dense boards the summed
KiCad unconnected fell 4 % (1179 to 1132), but five boards gained and four
lost by amounts a different seed moves them, and the short tier lost a
strict pass. The legaliser and the refinement undo much of the anneal's
term.
