# Small-board routing ladder

This is the active feedback loop for routing changes. It deliberately stops
before the 43-component dual-ESP32 target. Each promoted rung must identify a
specific mechanism, pass independent exact geometry and connectivity checks,
and retain deterministic work evidence.

## Enforced rungs

Measured with the default DUT grid configuration on 2026-09-01:

| Rung | Branches | Expansions | Exact result | Mechanism |
| --- | ---: | ---: | --- | --- |
| ESP32 three-pad bank | 1/1 | 46 | pass | Real footprint rotation and sibling-pad avoidance |
| Three-terminal tree | 2/2 | 148 | pass | Explicit electrical route graph |
| Resistor turn escape | 2/2 | 93 | pass | Freely rotatable two-terminal component between realistic endpoints |
| ESP32 pad fanout | 2/2 | 2,838 | pass | Neighboring pad escapes into two freely rotatable resistors |
| Forced planar crossing | 2/2 | 10,672 | pass | Layer transition with explicit vias |
| Power/capacitor interaction | 4/4 | 612 | pass | Small interacting power and decoupling slice |
| West-side geometry | 6/6 | 12,051 | pass | Denser fixed-placement routing |
| South header breakout | 8/8 | 11,545 | pass | ESP32-to-resistor-to-header paths |
| Resistor link bundle | 10/10 | 11,683 | pass | Five independently oriented resistor links |

The regression test uses deliberately loose per-rung ceilings from 200 to
40,000 expansions. These are not performance claims; they make gross search
regressions visible while allowing implementation changes. No active ladder
test loads the dual-ESP32 input.

## Coupled family/continuous rung

The independent `family-clearance-obligation.json` rung exercises a mechanism
the grid ladder does not: joint family assignment deliberately retains one
topologically compatible 0.05 mm copper-clearance shortfall and hands it to the
continuous engine. Both route-family searches exhaust after one fully
certified class rather than being padded to K=3. One segment-pair row then
pushes two vertically movable endpoint bodies by 0.050940 mm and reaches zero
exact findings in nine retained frames. The matched all-fixed control rolls
back with one finding.

This is the first natural route-family → tracer → placer → exact-gate rung. It
is kept separate from the raster expansion table because its deterministic
work units are two family searches, 60 visibility states, one segment pair,
and 8 × 16 projection sweeps—not grid expansions.

```sh
cargo run -- analyze-route-families \
  benchmarks/small/family-clearance-obligation.json \
  declared run/family-clearance-obligation.json
cargo run -- view-continuous-repair \
  run/family-clearance-obligation.json run/family-clearance-obligation.html
```

The adjacent `family-clearance-body-neighborhood.json` rung adds a body
keepout exactly at the lower trace's clearance boundary. One trace-pair row
plus two oriented trace/body rows redirect almost all correction into the
upper endpoint bodies and exact-pass in nine frames. Disabling body contacts
on the same input substitutes one trace/body violation for the repaired
trace/trace violation and remains exact-rejected. This is the current reduced
control for local-neighborhood coupling; it does not use the dual-ESP32 board.

The separate `family-multi-run-via.json` control now exercises native bounded
discovery as well as the checked-in V5 importer boundary. It generates a top
run, one directed via, and a bottom run, materializes segment layers
`[top, bottom]`, emits the matching via decorator, and passes both exact gates.
The adjacent `family-seeded-via-obstacle.json` control blocks the seed midpoint:
the producer rejects that site with exact annulus-clearance evidence and retains
families at four distinct legal positions. These are single-branch mechanism
controls, not promoted board rungs. Their evidence and reproduction commands are in
[`multi-run-family-via.md`](multi-run-family-via.md).

`family-seeded-via-two-net.json` is the next isolated control. Two parallel
multilayer nets are far enough apart for their traces and via-to-trace pairs to
be legal, but vias at equal sampled positions violate clearance. Exact pair
analysis records those annulus conflicts and joint assignment selects distinct
sites. Restricting both variables to one site exhausts the admitted portfolio
as bounded infeasible. This is still not an active board rung.

The pad-bank case was also used to evaluate an alternate-grid-terminal-contact
experiment. Baseline and experiment both produced the same exact-complete
route in 76 expansions under the then-current heuristic, and the alternate
path was never invoked. That code
was removed: the fixture showed no benefit and therefore did not justify the
extra policy surface.

The three-terminal case now also gates an explicit topology transaction. The
grid baseline has two terminal-to-terminal branches and 49.426407 mm of copper.
The accepted trial splits one branch, reattaches its sibling at a degree-three
junction, and exact-passes with three physical branches and 49.219300 mm. No
additional route search is required.

The same case now has a selectable search-time `shared_copper_tree` policy.
It grows B from the closest legal vertex on the already-routed A–C copper,
materializes the contact as an explicit degree-three junction, and exact-passes
with 46.355339 mm of copper. It uses 144 expansions versus 148 for the current
terminal-MST control. Before adding an admissible unavoidable-bend lower bound
to A*, these figures were 1,184 versus 874; preserving that intermediate result
identified a generic search weakness instead of misclassifying tree growth.
Two additional shared-tree controls guard the imported semantics without
pretending to be realistic board rungs. The four-terminal case chooses D from
the current A–B tree even though C is next in root order (135 expansions,
66.0 mm, exact pass). The obstructed case searches from `(5,15)`, follows
existing copper around a wall, trims 15 duplicate prefix points, and commits
at `(5,7.5)` (1,303 expansions, 44.863961 mm, exact pass). Its terminal-MST
control uses 1,572 expansions and 45.899495 mm.

The terminal-junction control isolates the predecessor's optional semantic
junction preference. The ordinary shared tree uses 26 expansions, three
physical branches, one interior junction, and 12.414214 mm. Enabling 1.0 mm
slack runs one additional terminal-source search and selects B: 42 expansions,
two physical branches, no interior junction, and 12.242641 mm. Both exact-pass;
the option stays disabled by default because the extra work is real.

The shared-tree transaction also has a router-independent three-terminal
regression for a crossing at half a grid cell. It inserts the crossing point
into both same-layer polylines, trims the already-owned prefix, builds an
explicit degree-three junction, and passes the independent exact validator.
This isolates segment-contact semantics without requiring a larger board to
coincidentally produce the geometry.

The adjacent `shared-tree-attachment-portfolio.json` control evaluates the
cost of testing more routes without promoting the policy to a default. The
greedy source uses 368 expansions and 19.242641 mm; a bounded second source at
least 2 mm away uses 500 expansions and 17.828427 mm. Both exact-pass. On the
dynamic-terminal and prefix-trim controls, the same policy increases work from
135 to 279 and from 1,303 to 1,957 expansions without changing copper length.
The mixed result and complete attempt evidence are the intended feedback.

Run only the active routing ladder with:

```sh
cargo test -p pcb-routing small_board_ladder_is_exact_and_bounded -- --nocapture
```

The one-branch `placement-perturbation-unblocks-routing` case is the current
coupled control. Routing alone fails with `WALL`, `TOP_WALL`, and `BOTTOM_WALL`
on the blocked frontier; the existing transactional coordinator moves `WALL`
and reaches an exact-complete result. A pressure-directed alternative also
passes, but currently needs two repair reroutes and 868,461 total expansions
versus one reroute and 436,241 for axis sampling. The spatial frontier pressure
is orthogonal to the wall's legal motion. Passage-capacity pressure uses the
neighboring gap deficits instead and restores the one-repair result.

The native `passage-pressure-asymmetric.json` control supplies the complementary
positive case. Frontier and passage pressure each solve it with one repair and
436,739 expansions; uniform axis sampling needs three repairs and 868,969.
Together the two cases prevent either a blanket “pressure wins” or “pressure
loses” conclusion.

The native `passage-pressure-push-chain.json` control adds a movable physical
package body in the only useful motion direction. Its body is not a routing
keepout, so it does not change passage capacity, but exact geometry forbids the
routing blocker from passing through it:

| Policy | Repair attempts | Routed-search expansions | Result |
| --- | ---: | ---: | --- |
| Axis sampling | 8 | 3,025,340 | incomplete |
| Passage pressure, single body | 1 | 432,178 | incomplete |
| Passage pressure, collision chain (max 2) | 1 | 623,298 | exact pass |

The single-body attempt is rejected before rerouting. The axis sampler has two
such placement rejections and exhausts all remaining legal samples without
opening a sufficient passage. The chain records the `WALL -> FOLLOWER` contact,
translates both by +0.5 mm, and adds capability rather than merely reducing
search work.

Larger solved slices may be run as integration canaries, but an algorithm
change should first explain its effect on the smallest rung that exercises the
mechanism. The dual-ESP32 board remains historical regression evidence rather
than an optimization loop.
