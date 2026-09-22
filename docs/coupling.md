# Coupling placement and routing

Status: design, 2026-09-22. What exists today is the loop in
`crates/pcb-kicad/src/board_layout.rs`: place, route, read the router's
congestion history, grow the halos of the footprints in congested regions,
place again from scratch, keep the best round. It works (see
[benchmarks](benchmarks.md)), but it is a loose coupling: every placement
change restarts routing from nothing, and the placer learns about routing only
through per-footprint halos.

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

1. **Incremental router state.** `pcb_router::Router` becomes long-lived:
   `update(board_delta)` rebuilds the static maps only in the tiles whose
   fixed copper changed, recomputes terminal nodes and escapes for the moved
   footprints, rips up the nets that touch changed tiles, and reruns
   negotiation for the pending set. Everything else (occupancy, tile fill,
   history) is kept.
2. **Router-driven moves.** After a round, the router names the tiles where
   negotiation could not converge and the footprints whose pins sit in them.
   The placer proposes small legal moves for those footprints (its refinement
   step already does legal moves and swaps), the router reroutes incrementally,
   and the move is kept if fewer nets remain in conflict. This replaces the
   halo-growth-and-restart loop.
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
