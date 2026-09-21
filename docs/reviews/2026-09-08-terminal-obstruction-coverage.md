# Distinguishing connector escape from a second blocked terminal

`interf_u` remains at 65 of 110 routed nets, with 125 native opens and no
design/ERC/parity findings. The PC-A7 failure is now characterized more precisely:
all 65 single-net removals fail; five of the 64 tested pairs containing PC-A6
permit a counterfactual PC-A7 route, but neither direct restoration order retains
all original connections. These are failed recovery experiments, not board
completion or accepted routing progress.

[Reports, endpoint overlays and restoration renders](../../build/interf-cut-coverage-2026-09-08/index.html).

## Evidence that changes the next recovery action

The new typed terminal context identifies the root at U2 `(113.665, 116.840)`,
BUS1 at `(111.125, 138.430)` and U9 at `(145.415, 85.090)`. On the retained board,
PC-A7 fails before reaching BUS1. Removing PC-A6 allows that branch to complete
inside the search, but the branch to U9 remains grid-disconnected. All three
terminal coordinates independently match the pad-access inventory; their nearest
grid samples are clear. Reached search branches have not received native admission.

| Experiment | Coverage | Result |
| --- | --- | --- |
| Single-net removal | All 65 retained foreign nets | No full PC-A7 route |
| Pair removal | PC-A6 with each of the other 64 nets | Five full counterfactual routes |
| Restore displaced nets | Both orders for all five successful pairs | No accepted improvement |
| Reciprocal repair after restoring MD2 | Yield new PC-A7, route PC-A6, restore PC-A7 | PC-A6 routes; PC-A7 fails toward BUS1 again |

Successful pairs combine PC-A6 with PC-A5, MD2, MA6, MA3 or MA11. The remaining
59 pairs all fail toward U9 after reaching BUS1. The pair experiment covers
64 of 2,080 possible pairs, not exhaustive two-net coverage. No failed pair
exhausts the search budget. Disconnection is evidence about the selected grid
and routing policy, not a proof of physical impossibility.

All five target-only boards pass native design checks. For MD2, MA6, MA3 and
MA11, routing that net first restores it, but PC-A6 remains blocked. For the
PC-A5 pair, neither net can be restored first. The separate reciprocal repair
on the restored-MD2 board confirms a cycle: PC-A7 blocks PC-A6's first branch;
after exchanging them, PC-A6 blocks PC-A7's first branch. Its transaction is
correctly rejected.

The next recovery work should generate alternative connector escape paths while
accounting for the displaced connection. Repeating single-net removal or merely
raising its trial cap cannot resolve this measured case. Candidate diversity,
remaining-net demand and bounded repeated negotiation are justified next tests;
none has been established as a solution by this experiment.

## Program changes

- Typed search failures now optionally include root, reached and pending terminal
  coordinates. CLI error strings remain unchanged; old serialized failures still
  deserialize. Both ordinary and yielding diagnoses retain the structured data.
- Yielding diagnosis schema 4 adds `minimum_yielding_connections`, default one.
  Setting both minimum and maximum to two tests pairs without repeating prior
  single-net coverage. Trial bounds and preferred ordering remain explicit.
  Production single-net recovery retains its one-net contract.
- The automatic recovery summary retains typed yielding failures.
  `report_yielding_diagnosis.py` adds compact coverage tables and deduplicated
  endpoint overlays on combined copper, with component bodies hidden. The
  overlays show retained copper, including counterfactually removed nets; they
  do not purport to identify exact blocking segments.
- `probe_pair_restoration.py` tries both restoration orders with native checks
  and combined renders at every attempted stage. It preserves the source and
  stops only if a full restoration passes its explicit progress contract. This
  is an experiment driver, not integration into the sequential coordinator.

## Validation and limits

All 141 KiCad adapter tests and eight benchmark tests pass. A planted three-pad
wall control verifies the later-branch endpoint context for both geometric and
obstacle-distance heuristics. Pair-only enumeration controls check ordering,
cardinality and bounds. The real PC-A6 single-removal replay preserves both CLI
failure strings and search work exactly while adding context.

The full-single sequential audit passes. An independent restoration audit checks
all 19 materialized stages: poses, unrelated copper, schematic/project bytes,
native dimensions and render XML. All 19 stages have no native design findings;
their remaining opens prevent an accepted improvement. The reciprocal repair
has its own independent native audit. Retained source hashes are unchanged.
Headless Chrome checks coverage counts, all 66 table rows, local links, loaded
images and expanded context; its screenshot was inspected.

The five successful pair candidates were generated with the same 0.125 mm grid,
0.4 mm PC-A7 width, 0.254 mm clearance and four-million-expansion bound as the
preceding guided single-net tests. No placement, outline or fabrication rule
was relaxed. The pair search took about 247 seconds; native restoration checks
are additional work. Full-board routing, general recovery and compact placement
remain unfinished.
