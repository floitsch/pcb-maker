# Explicit route-graph junction transactions

This experiment ports the smallest useful route-graph topology transaction
from `layout-trace` and establishes the semantic prerequisite for the shared
copper-tree search in `testing-esp32-duts`.

The unsafe shortcut would be to let a new same-net trace touch the interior of
an existing trace and infer connectivity from geometry. The durable candidate
graph does not permit that ambiguity: every branch endpoint must name a
terminal or junction, every incidence must agree in both directions, and its
physical endpoint must coincide with the named node.

`optimize-route-junctions` therefore performs a bounded transaction:

1. find a terminal with at least two routed incident branches;
2. split one branch at its first same-layer interior point;
3. create a stable explicit junction at that point;
4. reattach the sibling branches from the terminal to the junction;
5. rebuild all branch incidences and via decorators;
6. accept only an independently exact-valid configured length improvement.

The topology core consumes and returns `CandidateArtifact`; it does not depend
on the DUT grid router. The CLI wrapper routes a convenient baseline first,
but corridor/family and future GPU strategies can submit their own durable
candidates through the same exact-gated operation.

Endpoint vias and split-point vias are rejected. Route-basis fingerprints are
cleared on changed branches. Every candidate is derived from the same routed
baseline, so a failed or merely insufficient proposal cannot contaminate the
next trial.

## Reduced evaluation

Fixture: `benchmarks/imported/layout-trace/multi-terminal-tree.json`, declared
placement, and `dut-grid-adaptive.json`.

| Candidate | Search expansions | Physical branches | Junctions | Copper length | Exact result |
| --- | ---: | ---: | ---: | ---: | --- |
| Terminal MST baseline | 148 | 2 | 0 | 49.426407 mm | pass |
| First adjacent-junction proposal | no additional search | 3 | 1 | 49.219300 mm | pass |
| Search-time shared-copper tree | 144 | 3 | 1 | 46.355339 mm | pass |

The 0.207107 mm reduction is intentionally modest: this first port considers
only the first interior point next to the branching terminal. Its importance
is semantic, not the headline percentage. Terminal `C` changes from degree two
to degree one, the new `junction:BUS:0001` has degree three, and the split tail
becomes a third explicit physical branch. The exhaustive geometry and
electrical validators report zero findings.

A control requiring at least 1.0 mm improvement evaluates both available
proposals and retains the exact baseline. This proves that “proposal applied”
and “proposal committed” are separate states.

The selectable `shared_copper_tree` router policy now performs the search-time
version of the transaction. It starts from the lexicographically stable root
and its nearest terminal. For every later growth step it compares every legal
current tree vertex with every pending terminal using predecessor-style grid
Manhattan distance plus layer-change cost. After search, it discards the
prefix through the last state already in the tree. Growth dependencies remain
ordered even when the general route-order policy changes.

On the original control, after `BUS#branch0` is routed, the policy chooses
`(19,19)` on that branch and terminal B, splits the branch, and records both
the selected target and attachment in evidence. The result saves 3.071068 mm
relative to the terminal MST and the explicit junction has degree three.

The first run exposed a search penalty despite the shorter route: 1,184 versus
874 expansions. The cause was not tree growth itself. The heading-expanded
A* charged bends while its heuristic omitted the one unavoidable bend for a
non-collinear goal, leaving many near-equal heading states. Adding that
admissible lower bound reduced the same candidates to 144 and 148 expansions,
respectively; path costs and copper geometry did not change.

A separate four-terminal control proves dynamic target selection. Although C
is next in root-distance order, the A–B trunk passes within 5 mm of D; branch 1
therefore targets D and branch 2 targets C. Shared growth uses 135 expansions
and 66.0 mm of copper versus terminal MST's 199 expansions and 67.485281 mm.
It emits four physical branches and four terminal nodes and exact-passes.

The obstructed prefix-trim control forces a search selected at `(5,15)` to
follow the existing vertical tree before escaping around a wall. The committed
junction moves to the last shared state `(5,7.5)` and 15 duplicate grid points
are removed. Without trimming, shared copper would measure 52.363961 mm;
after trimming it measures 44.863961 mm, compared with the terminal-MST
control's 45.899495 mm. Search work is 1,303 versus 1,572 expansions and both
exact-pass.

The optional terminal-junction preference is disabled by default. When
enabled, it reroutes from connected terminals whose estimated distance is
within the configured slack, compares retained branches by via count, length
slack, then bend count, and exposes an explicit maximum number of alternative
searches. On `shared-tree-terminal-junction.json`, ordinary growth uses one
interior junction, 12.414214 mm of copper, and 26 expansions. With 1.0 mm
slack, one additional search selects terminal B directly: the result has no
interior junction, 12.242641 mm of copper, and 42 expansions. Both exact-pass.
This is a quality/topology win with a measured search-work penalty, not a
blanket default change.

The separate tree-attachment portfolio generalizes source trials beyond
terminals. It distance-ranks tree/target pairs, applies a configurable spatial
diversity radius, and evaluates an explicit maximum. The selected objective can
use retained length, predecessor-style via/length/bend ordering, or A* cost.
On the asymmetric-wall control, its second source is the interior point
`(5,8)` and reduces copper from 19.242641 mm to 17.828427 mm while increasing
work from 368 to 500 expansions. Dynamic-terminal and prefix-trim controls show
work penalties with no quality gain, so the portfolio remains opt-in. Full
measurements are in
[`tree-attachment-portfolio.md`](tree-attachment-portfolio.md).

## Scope boundary

Search-time growth chooses initial sources from existing trace vertices. The
default performs one predecessor-compatible greedy search; an opt-in bounded,
spaced portfolio can evaluate more. After routing,
it recognizes the last non-parallel same-layer centerline contact even when it
falls inside both segments, inserts the contact into both polylines, trims the
already-owned prefix, and builds exact junction incidence. A reduced
three-terminal half-grid regression exact-passes this transaction. Collinear
interior overlap, contact at a via, and whole-tree lookahead remain.
Selective and negotiated rerouting reject this policy until synthetic
split-branch ownership is defined. Continuous junction movement, full tree
canonicalization, and full-tree transactional rerouting also remain.

Run and inspect the experiment with:

```sh
cargo run --release -- optimize-route-junctions \
  benchmarks/imported/layout-trace/multi-terminal-tree.json \
  declared run/multi-terminal-junction.json \
  experiments/configs/dut-grid-adaptive.json \
  experiments/configs/route-junction-adjacent.json

cargo run --release -- view-route \
  benchmarks/imported/layout-trace/multi-terminal-tree.json \
  run/multi-terminal-junction.json run/multi-terminal-junction.html
```

Viewer playback shows the terminal-MST baseline followed by the split tree;
magenta diamonds identify explicit junction nodes. Direct shared-tree results
draw a green ring around the committed attachment. If prefix trimming changed
it, an amber ring and dashed line retain the originally searched source. A
violet halo identifies a selected terminal-junction alternative.
A white X identifies an attachment inserted inside one or both segments.
