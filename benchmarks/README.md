# Benchmark ladder

Cross-tool cold-input manifests and their execution contract are documented in
[`competitive/README.md`](competitive/README.md).

## Active declaration-to-KiCad ladder

`esp32-c3-ladder/declaration.json` is the system pipeline ladder. Rung 0 keeps
the generated ESP32-C3 board's 36 footprints, outline, rules, and graphics but
marks every schematic pin intentionally unconnected and emits no tracks or
vias. The ordered declaration contains all 42 multi-pad electrical nets.
Every prefix through rung 42 (`GND`) passes. Each output includes KiCad's
exported netlist, ERC report, DRC report with schematic parity, and a
machine-readable completion decision.

`dual-esp32-ladder/declaration.json` is the active harder-board restart. It is
generated from the semantic dual fixture, retains 43 component poses and 52
electrical conductor identities, and orders large shared VCC/GND nets last.
Independent cold prefixes through connection 14 pass the same native gate.
Connections 2, 5, and 11 select shared-copper-tree growth, while connection 8
selects rooted-star routing. Prefix 5 is the first
substantial routing-pressure case: its new three-terminal tree uses four
well-spaced vias and 25,908 A* expansions. Each prefix prunes later semantic
connections before placement and starts routing from zero copper. The promoted
prefix-8 board has 298.564054 mm of copper and eight non-clustered vias.
Topology-equivalent portfolio entries on two-terminal connections are removed
before search, saving 19.91% of portfolio expansions and 31.25% of native gates
without changing the selected PCB. Prefix 9 reaches 304.472117 mm and 10 vias;
its two new vias replace a native-complete zero-via detour that is 12.37×
longer. Prefix 10 reaches 330.096386 mm and 12 vias. Its new 25.624269 mm
connection uses a long back-layer trunk between two front-layer escapes; the
new vias are 20.804146 mm apart. A fixed-parent 125×-via-cost diagnostic
retains the same path while increasing search work 68.89×, evidence that the
layer change is structural under the fixed prefix-9 copper rather than a via
cluster. The first prefix-11 result was complete but placed two connected vias
exactly 1.0 mm apart. A two-source shared-tree attachment portfolio considers
the same coordinate on both layers, removes one via without adding copper,
and promotes a cold 396.988939 mm/17-via board. That selected connection takes
76,690 rather than 39,446 expansions; the full portfolio takes 233,159 rather
than 176,494. Prefix 12 adds a 28.186238 mm front-only route with no vias and
reaches 425.175177 mm/17 vias.
Prefix 13 reaches 457.016841 mm/19 vias. Its new 31.841664 mm route uses two
vias 18.681542 mm apart. A fixed-parent high-via-cost control finds a
native-complete 37.023645 mm zero-via route; the two-via route wins by only
1.181981 mm under the current 2 mm/via selection penalty, with a 2.590990
mm/via break-even.
Prefix 14 is the first local-insertion failure: rooted and shared policies both
prove branch 0 unreachable at 0.25 mm, and a 0.125 mm control also exhausts
without a path. Of 13 single-net yielding counterfactuals, only removing
`SIG01_AFTER_LINK` opens the route. Transactional repair adds the new
46.784931 mm front-only tree, reroutes the yielded net as 63.268446 mm/four
vias with 10.75 mm minimum spacing, and native-passes a cold 509.522335 mm,
112-segment, 23-via board. Total cold-prefix work is 12,681,015 expansions,
mostly serial negative counterfactuals. No later prefix is claimed.

KiCad represents internally common ESP module ground pins as one logical
no-connect group, so its raw ratsnest reports four same-module groups even
when every member is explicitly marked `no_connect`. The completion gate
accepts only that narrow form; an unconnected selected net fails. Reconstructed
library metadata warnings are also retained separately and never hide a
geometric or electrical finding.

```sh
cargo run -- materialize-kicad-rung \
  benchmarks/esp32-c3-ladder/declaration.json 0 artifacts/esp32-c3-ladder
cargo run -- verify-kicad-rung artifacts/esp32-c3-ladder/00-empty esp32-c3

cargo run -- materialize-kicad-rung \
  benchmarks/esp32-c3-ladder/declaration.json 42 artifacts/esp32-c3-ladder
cargo run -- verify-kicad-rung \
  artifacts/esp32-c3-ladder/42-gnd esp32-c3
```

The active routing loop starts with the smallest failing prefix. The Rust
KiCad adapter rasterizes rectangular board bounds, rotated pads, existing
tracks, and vias, then calls the replaceable bounded DUT A* kernel. Returned
segments/vias, cost, and expansion count are reviewable candidate evidence;
Multi-terminal targets use an explicitly labeled deterministic rooted-star
baseline; independently searched branches reserve via-hole spacing from one
another. Persisted replacement candidates let power-net copper be exact-gated
before it replaces historical routes. KiCad remains the acceptance authority.
A rung must pass the independent gate before changes are evaluated on the next
rung.

Promoted and superseded candidate measurements are retained in
[`esp32-c3-ladder/route-evidence.md`](esp32-c3-ladder/route-evidence.md).
The first accepted route-quality experiment and its rejected prefix-only
control are retained in
[`esp32-c3-ladder/experiments/3v3-line-of-sight-shortening.json`](esp32-c3-ladder/experiments/3v3-line-of-sight-shortening.json).

## Active exact-routing rungs

| Case | Branches | Mechanism |
| --- | ---: | --- |
| `esp32-slices/esp-pad-multi-contact-escape.json` | 1 | One realistic terminal leaving a three-pad ESP32 bank |
| `multi-terminal-tree.json` | 2 | Electrical net as a graph, plus the explicit-junction topology control |
| `esp32-slices/resistor-turn-escape.json` | 2 | A freely rotatable resistor between realistic endpoint geometry |
| `esp32-slices/esp-pad-fanout-clearance.json` | 2 | Two nearby ESP32 pad escapes into freely rotatable resistors |
| `forced-crossing-two-layer.json` | 2 | Layer transition and explicit-via correctness under an unavoidable planar crossing |
| `esp32-slices/power-west-capacitor-interaction.json` | 4 | Small interacting power/decoupling slice |
| `esp32-slices/west-six-final-geometry.json` | 6 | Denser fixed-placement geometry slice |
| `esp32-slices/south-header-breakout.json` | 8 | ESP32-to-resistor-to-header breakout |
| `esp32-slices/resistor-link-bundle.json` | 10 | Five independently oriented resistor links between two modules |

All nine are enforced by
`pcb-routing::dut_grid::tests::small_board_ladder_is_exact_and_bounded`.
The ladder stops here; dual-ESP32 is not run by this test.

## Isolated mechanism controls

| Case | Mechanism |
| --- | --- |
| `squeeze.json` | Two trace chains and a movable blocker under width growth |
| `movable-route-blocker.json` | Route pressure moving a placement degree of freedom |
| `placement-perturbation-unblocks-routing.json` | Discrete placement fallback after topology failure |
| `small/passage-pressure-asymmetric.json` | Directed placement feedback when one legal passage is closer to routable |
| `small/passage-pressure-push-chain.json` | A routing blocker can yield only by translating a contacted movable package body with it |
| `small/shared-tree-dynamic-terminal.json` | Current tree geometry, rather than root order, selects the next pending terminal |
| `small/shared-tree-prefix-trim.json` | Obstacle-forced overlap with existing tree copper must be trimmed and materialized at the last shared vertex |
| `small/shared-tree-terminal-junction.json` | Optional reroute from a connected terminal removes an avoidable interior junction |
| `small/shared-tree-attachment-portfolio.json` | A bounded second interior-tree source beats the greedy distance estimate around an asymmetric wall |
| `small/continuous-resistor-coupling.json` | A trace-spacing correction moves a freely rotatable two-terminal body, then materializes an exact-valid candidate |
| `small/continuous-multipin-tree.json` | General body-local pins and a deterministic multi-terminal continuous route graph |
| `small/family-multi-run-via.{json,portfolio.json}` | Native bounded two-run/one-via discovery plus an independent file-level V5 import; both materializations pass the exact gates |
| `small/family-seeded-via-obstacle.json` | A keepout blocks the seed midpoint; exact via-clearance rejection is retained while four spatially distinct legal sites survive |
| `small/family-seeded-via-two-net.json` | Two nearby multilayer nets conflict only when their vias use the same sampled position; joint assignment selects distinct sites, while the one-site control is bounded infeasible |
| `small/conflict-action-orthogonal-crossing.json` | Electrically complete crossed seeds produce the six requested north/south, west/east, and via-on-either-net states; every state is retained and exact-gated |
| `small/conflict-action-two-crossings.json` | Bounded best-first search resolves either of two independent crossings and deduplicates final candidates reached in opposite action orders |
| `small/conflict-action-shared-trace.json` | Two conflicts share one trace; some first actions make the remaining conflict unsupported, while viable action orders exact-pass and retain the dead-end evidence |
| `small/board-continuation-clear.json` | Oversized-board shrinking exact-passes every affine stage through the target outline |
| `small/board-continuation-bottleneck.json` | Shrinking remains valid through scale 1.2, rejects scale 1.1, and retains the last valid board plus failed-stage evidence |
| `small/board-continuation-local-repair.json` | Two independent routes separate affine retention from local repair: shrinking invalidates `SIGNAL` at every stage while `UPPER` remains valid and fixed during selective rerouting |
| `small/board-continuation-force-motion.json` | Several retained shrink stages remain exact before a thin vertically movable body must yield to an attached straight trace; continuous correction avoids later topology search |
| `small/board-continuation-explicit-rect-motion.json` | A fixed rectangular explicit keepout is lowered as a virtual obstacle; retained trace vertices move through five exact shrink corrections without rerouting |
| `small/board-continuation-explicit-keepout-fallback.json` | Shape-boundary control: a circular keepout reports `unsupported` and exact-passes through selective grid fallback rather than being approximated as a rectangle |
| `small/connection-insertion-esp32-progressive/` | Five cumulative ESP32/resistor stages; each adjacent pair adds exactly one legacy branch and every adapted stage commits from an immutable exact parent |
| `small/connection-insertion-pressure-source.json` → `small/passage-pressure-asymmetric.json` | A new connection has no fixed-parent route; global fallback moves the implicated wall and commits the whole target |
| `decoupling-capacitor.json` | A movable two-terminal part with asymmetric net objectives |
| `esp32-slices/evidence/east-route-family-fixed-context-7e9e146.json` | Fixed-context fingerprint and exact family-pair regression evidence for the ported routing kernel |

The coupled placement case has its own coordinator regression test: the
initial one-branch route must fail with a
movable blocker, then the transactional repair must pass exactly.

The three-terminal rung also has two topology regressions. The router-independent
adjacent transaction saves 0.207107 mm, while selectable search-time
shared-copper growth saves 3.071068 mm and uses an explicit degree-three
junction. Both remain exact-complete. A minimum-improvement control proves
that insufficient post-route topology trials roll back to the baseline.

The following imported sources are retained as long-horizon inputs. The dual
case is now advanced only as a one-connection-at-a-time native ladder, not used
as an all-at-once tuning target:

| Case | Role |
| --- | --- |
| `dual-esp32-benchmark.json` | Source for the active 52-connection native restart; also retains the historical 83-branch geometry |
| `esp32-c3/` | Generated KiCad import/export system target from `testing-esp32-duts` |

The JSON cases originate in `/home/flo/programming/layout-trace` at
canonical commit `1a6ec18`. The KiCad pair originates in
`/home/flo/work/testing-esp32-duts/generated/esp32-c3` at commit `965f388` plus
the working tree present on 2026-08-31. See `manifest.sha256` for exact file
identity.

Imported inputs are historical data. Their embedded solver settings do not
define this repository's engine architecture.

The imported progressive insertion fixture is preserved byte-for-byte under
`imported/layout-trace/connection-insertion/esp32-progressive/`. Its shared
`SIGNAL_A`/`SIGNAL_B` electrical identities span opposite resistor pins, which
the current independent connectivity gate correctly rejects: a resistor is
not copper. The native `small/connection-insertion-esp32-progressive/` copy
removes those two aliases so every resistor side retains its default branch
identity. No geometry, rule, pose freedom, or stage delta was changed.

Run one transaction with:

```sh
cargo run -- insert-connection \
  benchmarks/small/connection-insertion-esp32-progressive/03-b-west.json \
  benchmarks/small/connection-insertion-esp32-progressive/04-b-crossing.json \
  path/to/03-parent.result.json path/to/04-result.json \
  benchmarks/small/connection-insertion-esp32-progressive/one-policy.json
```

`small/passage-pressure-asymmetric.json` is a native reduced control derived
from the imported one-wall mechanism. It deliberately changes the upper and
lower passage capacities; it is not presented as a predecessor artifact.

`small/passage-pressure-push-chain.json` adds one non-copper package body to
that mechanism. It is the bounded collision-propagation control: single-body
pressure and uniform axis sampling remain incomplete, while a two-body
same-vector push chain passes exact validation.

`smoke/placement-only.{problem,candidate}.json` is a native file-level smoke
pair for the combined physical and electrical exact-validation command. It is
not an algorithm-quality benchmark.

The imported KiCad baseline parses as 33 schematic components and 85
schematic nets. Its PCB has 36 footprints, 321 track segments, 57 vias, and 11
unrouted nets. Those are provenance facts, not success criteria; the target is
intentionally useful before it is complete.

Check the copied bytes independently with:

```sh
cd benchmarks
sha256sum -c manifest.sha256
```
