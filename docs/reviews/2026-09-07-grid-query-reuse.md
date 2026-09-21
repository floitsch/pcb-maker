# Reuse symmetric routing-grid collision queries

The completed open ESP32 prefix spent 239 seconds generating routes across its
19 committed transactions. Inspection found repeated work before A*: each
planar segment was checked in both directions, and through-via clearance was
computed twice at the same x/y position. These checks ask the same geometric
question; directional search costs are applied separately.

The main native routing-grid builder now evaluates each undirected segment
once and writes the answer into both directed transition entries. It also
evaluates the combined two-layer via collision once per position and stores
that answer for both layer transitions. Point occupancy still depends on the
individual copper layer. Grid resolution, clearance predicates, search costs,
and search limits are unchanged.

The planar differential regression compares every transition with the previous
eight-direction calculation on two-layer geometry with rotated rectangular and
oval pads, a hole, local clearance, board edges and degenerate one-row/column
grids. Masks match exactly, with half the segment queries. All 93 native-adapter
library tests pass. The native via-window regression also passes and retains
its automatic renderings.

Replaying the retained ESP32 connection `SIG06_W_R` yields an exactly identical
candidate JSON, including geometry, cost, reachability and search expansion
counts. Materializing that candidate passes native ERC, DRC, parity and
connectivity checks; automatic preview output is retained. The replay ran
alongside other work, so its wall time is not an isolated speed measurement.

[Validation and executable hashes](../../build/routing-improvements/grid-query-reuse-2026-09-07/validation.json)
and [native preview](../../build/routing-improvements/grid-query-reuse-2026-09-07/optimized/preview.svg)
are retained. This change removes duplicate geometry work; it does not add new
routing options or establish full-board timing improvement.

Six subsequent [paired route replays](../../build/routing-improvements/grid-query-reuse-2026-09-07/replay-timings.json)
run each executable three times on the identical source/configuration. Every
candidate matches exactly; each replay references a retained rendering of that
verified geometry. Median process CPU time falls from 12.037 s to 6.768 s
(43.8% lower). Median wall time falls from 12.508 s to 7.218 s. The blocked
board benchmark ran concurrently, so these are local replay observations,
not a claim about isolated or full-board performance. The underlying reduction
in geometric query count is independently established by the differential test.
