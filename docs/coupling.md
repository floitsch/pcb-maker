# Coupling placement and routing

Status: steps 1 and 2 below are implemented (2026-09-22); 3 and 4 are open.

`layout-kicad-board` places the board, routes it once, and then lets the
router and the placer take turns on one in-memory state
(`crates/pcb-kicad/src/board_layout.rs`): the router's congestion history
names the footprints sitting where nets fought for room, the placer proposes
small legal moves for them (axis steps of 0.5-4 mm, quarter turns), ranked by
the congestion they would land in, the router reroutes only the nets the move
invalidated (`Router::update` and `Router::reroute` in `pcb-router`), and the
move is kept when the board improves (open connections first, then vias and
copper). On Interf-U two nudges take the first placement from 9 open
connections to 110/110; on the DUT boards a handful of resistor nudges remove
10-25 % of the vias. Per-board results are in [benchmarks.md](benchmarks.md).

## The idea

The router already has a coarse layer above the lattice: 16 x 16-node tiles
with a capacity (routable nodes per class and layer), a fill (claimed nodes)
and a conflict history. Every connection is planned on that tile graph before
the lattice search runs inside the resulting corridor ([router.md](router.md),
"Two-level search"). That graph is the natural shared representation between
placer and router:

- The placer can read tile capacity and fill directly instead of guessing
  room with halos, and can be told which tiles are over-subscribed.
- Routing can be *incremental*: when a part moves, only the tiles it touched
  change. Nets whose corridors pass those tiles or whose pins moved are ripped
  up; everything else keeps its copper. A placement change then costs a few
  net reroutes, not a whole-board route.
- Going further (Florian's suggestion): treat the first layer as a graph
  with per-corridor capacities, lay the connections out on that graph together
  with the placement, and map the graph onto the lattice only afterwards; the
  second layer is used only where the graph ran out of capacity.

## Steps

1. **Incremental router state** (done). `pcb_router::Router` is long-lived:
   `update(board)` rebuilds the static maps, keeps every net whose pads did
   not move and whose copper is still legal, and rips up the rest;
   `reroute()` negotiates only the pending set. Occupancy, tile fill and
   history are kept. (The static maps are still rebuilt in full; rebuilding
   only the changed tiles is a later optimization.)
2. **Router-driven moves** (done). See above. Each trial costs one
   incremental reroute plus the polish pass, 1 s on a small board and about
   20 s on Interf-U.
3. **Capacity-aware placement.** The placer's density term takes the tile
   graph's routing demand into account: a tile that must carry many corridors
   needs whitespace, not only body room. Corridor demand is estimated from the
   tile-graph plans of all nets on the current placement, which is cheap to
   recompute.
4. **Graph-first layout (experimental).** Place and plan all connections on
   the tile graph of one layer with capacities; accept when every corridor
   fits; then realize on the lattice. Use the second layer only for what the
   graph could not fit. This is the "place + route on a graph, then map to the
   PCB" experiment; it may or may not beat the current flow and is measured on
   the corpus like everything else.

Each step is judged by the corpus: boards completed, vias, copper and time in
place-and-route mode, compared with the current loop.
