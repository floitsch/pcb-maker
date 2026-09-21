# Interf-U medium-real cold baseline

## Scope

Sequences 240--244 admit the pinned KiCad `interf_u` project as the third
executable real source and the first executable `medium_real` source. It has 25
through-hole footprints, 110 routable named nets, 200 native connection items,
and a non-convex T-shaped outline of 12,191.58852 mm².

The upstream routed board is electrically connected, but current KiCad reports
two inherited starved-thermal errors and 23 obsolete footprint-link warnings.
Those findings are source debt, not a router result. The reproducible
`benchmarks/real/interf-u/prepare.sh` pipeline copies the pinned source,
synchronizes 22 schematic footprint links to the board's project-local
footprint library, and removes 731 tracks, 84 vias, one copper zone, and six
top-level copper graphics. The placement digest remains
`c4f4e090...b3cb579`. The cold board has zero ERC, DRC, or parity findings
apart from its expected 200 unrouted connection items.

## Exact non-convex outline support

The old adapter accepted only a rectangle and would have had to reject this
board or silently route through the two notches beside its edge-connector
extension. The routing model now reconstructs one unambiguous closed loop of
`gr_line` objects, records its exact shoelace area and bounding box, and blocks
both grid states and planar transitions against the polygon plus edge
clearance. Ambiguous, disconnected, curved, and multi-loop outlines still fail
closed. A reduced T-shaped test proves that a diagonal with legal endpoints
cannot cut across a concave exterior corner.

## Cold progression

Sequence 241 routes only alphabetically first net `8MH-OUT` from the prepared
zero-copper source. It uses 14.243525 mm, two segments, one bend, no vias, and
60 A* expansions. Native route-only admission observes zero introduced ERC,
DRC, or parity findings and the exact expected connectivity reduction from 200
to 199 items.

Sequence 242 asks for 16 nets but the 0.5 mm graph stops after two admitted
nets. `AUTOFD-` is unreachable in flood-fill after 28 states, so A* correctly
does no work. A 0.25 mm diagnostic still has no path after 308 reachable states.
At 0.125 mm the same fixed board and route order succeed in 15,843 A*
expansions with 14.989309 mm, six segments, five bends, and no vias. This is a
raster-resolution failure, not an A* budget failure: 2.54 mm-pitch, 1.4 mm PGA
pads leave about 0.232 mm of physical centerline corridor, while the
conservative half-cell guard closes it on the coarser grids.

Sequence 243 runs both 0.5 and 0.125 mm entries exhaustively at every step and
native-admits all 16 requested nets. It spends 678,377 A* expansions and
235.663776 routed-step seconds. Several fine candidates win only negligible
length differences, while the `D0` fine candidate alone spends 480,105
expansions and loses. This is retained as a negative portfolio-policy result.

Sequence 244 changes only the explicit portfolio policy to
`ordered_first_admitted`: 0.125 mm is attempted only after a coarse route fails
routing or native admission. It again admits all 16 nets, uses the fine
fallback on 7, and reduces work to 84,511 expansions and 137.534192 seconds:
87.5% fewer expansions and 41.6% less routed-step time. The different valid
lineage also happens to improve from 278.893202 mm/10 vias to 270.708452 mm/8
vias, but that incidental copper change was not the selection objective.

## Evaluation

This is a meaningful corpus advance, not a claim that the 110-net board is
solved. It establishes:

- exact ordinary-KiCad routing on a non-convex medium-real board;
- reproducible legacy-library normalization without changing the board;
- cold, immutable-source progression with native admission after each net;
- evidence that fine resolution is needed for dense terminal escape; and
- evidence that exhaustive resolution portfolios waste most search work for
  immaterial score changes.

The current performance limit is no longer primarily A* expansion budget.
Every route rebuilds whole-board raster state, and every admitted rung invokes
native KiCad DRC. Sequence 245 reuses the existing layer-aware obstacle broad
phase during rasterization. The `AUTOFD-` fine-grid route remains byte-for-byte
identical at 15,843 expansions while wall time falls from about 10.5 to 5.34
seconds. Sequence 246 independently replays all 16 cold nets: the final PCB is
byte-for-byte identical to sequence 244, while routed-step time falls from
137.534192 to 94.103760 seconds (31.6%). Against the original exhaustive
portfolio, the combined policy and broad-phase changes reduce time 60.1%.

Coarse admitted steps now take about 3.3--3.8 seconds, close to the per-rung
native DRC floor; fine fallbacks take about 8.7--9.4 seconds. Incremental/cached
grid construction and batched advisory native gating are the next runtime
experiments; final native authority must remain unchanged. The next completion
experiment should continue from zero copper with a larger bound and stop at
the first real interaction failure.

Paired front/back images for all investigated states are in `build/progress/`
as sequences 240--246. Machine-readable reports and candidates are under the
matching `build/sequence-*` directories.
