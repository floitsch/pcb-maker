# Pressure, topology, continuous motion and placement

The experiments favor a composition: discrete routing with congestion
feedback, topology choices informed by passage capacity, continuous geometric
relaxation, and substantially more placement exploration. They do not justify
replacing the router with a fluid simulator. Production defaults were not
changed in this round.

The [visual dashboard](../../build/algorithm-exploration-2026-09-07/index.html)
links all experiments, native board images and an animation of the actual
engine. Scripts and retained data make the experiments extendable.

## 1. Pressure fields

The CPU prototype solves a separate resistor/diffusion potential per net on
4/8-neighbor grids. Two layers connect through explicit via transitions. It
tests sequential routing, independent simultaneous routing, historical
congestion feedback, and negotiated current-pass occupancy. This is a first
test of the pressure idea, not a Navier–Stokes fluid implementation.

Across 150 trial rows on five small fixtures, the useful effect comes mainly
from coordination. Both shortest-path and field methods resolve the known
route-order trap with congestion prices. On a two-layer crossing, negotiated
shortest path needs two passes and the field three. A feedback-only control
initially made the field look better because shortest paths oscillated; adding
current-pass occupancy removes that apparent advantage. On the wall detour,
the four-neighbor field produces 42 cells of route versus shortest path's 28.

The field takes 1,173,270 active cell updates on that detour, plus reachability
work, versus 255 Dijkstra expansions. These are different operations with
different parallelism; this is not a GPU performance comparison. Full array
work is separately counted. Every trial has playback and a rendering; saved
routes, three feasible witnesses and nine deliberate corruptions were checked.

**Recommendation:** keep pressure as congestion prices and diagnostic maps.
Before implementing fluid dynamics, compare batched field solves with batched
distance/wavefront methods on real routing masks. Copper must be extracted as
separate nets and native-verified either way. Actual fluid inertia and
simultaneously moving copper remain untested.

[Experiment and reproduction](../../experiments/field-routing/README.md).
Current/history resource prices have a direct parallel in
[PathFinder](https://janders.eecg.utoronto.ca/1387_2015/readings/pathfinder.pdf).

## 2. Topology can carry width information

The useful representation is an embedded routing topology: obstacle-relative
passages and trace ordering, rather than electrical connectivity alone. The
related term is **homotopy**: paths that can deform into each other without
crossing obstacles. Practical signatures need to distinguish the relevant
classes and inter-net ordering; an arbitrary graph mutation does not provide
that guarantee. [Homotopy-constrained path planning](https://ojs.aaai.org/index.php/SOCS/article/view/18155)
is a relevant robotics parallel.

Widths need not disappear. A corridor carries capacity; its routes consume
width plus clearances. In the 0.37 mm gap, one 0.15 mm trace with 0.10 mm
clearance needs 0.35 mm; two need 0.60 mm. The prototype accepts the initial
zero-width geometry and rejects its physical-width realization.

Six fixtures cover capacity, route order, mixed widths, a capacity-safe but
geometrically invalid realization, and limited infeasibility. A twelve-pair
sensitivity sweep varies the initial assignment and label ordering on one
six-net board. Guided search evaluates 79 geometries versus 3,508 for blind
enumeration; including priority work gives 521 versus 3,508 graph checks.
Both policies find valid solutions in all twelve pairs. This demonstrates
useful conflict information, not industrial-router superiority or a runtime
speedup. The prototype materializes its tiny assignment universe in advance.

**Recommendation:** extract bottlenecks, occupants, ordering and width demand
from actual failed routes. Use these to choose a route to move or an obstacle
to displace. Retain geometric realization and native admission. A failed
continuous solve is not proof that an entire topology is impossible; only
sound capacity certificates justify permanently excluding a topology.

Width-aware topological routing is established prior art:
[Yu and Dai's cut-based encoding](https://tr.soe.ucsc.edu/sites/default/files/technical-reports/UCSC-CRL-97-07.pdf)
explicitly supports variable width and spacing. Our prototype implements a
small special case, not their full encoding or routability theorem.
[Experiment and reproduction](../../experiments/topology-routing/README.md).

## 3. The actual engine, visibly

The new harness calls the existing CPU engine and records every iteration.
Playback distinguishes tension proposals, density proposals and net constraint
corrections. It draws rigid bodies and actual copper widths, with initial
geometry ghosts, play/pause and scrubbing. GIFs work without JavaScript.

The controls explain several earlier disappointments:

- Default trace tension is zero. A legal slack trace stays unchanged; explicit
  tension reduces 30.6467 mm to its 26 mm straight-line lower bound.
- Copper-repair particles have zero density weights. Turning up field strength
  alone produces identical trajectories. An explicit unit-weight ablation
  activates pressure, but is not an area-correct occupancy model.
- Small residual indicates constraint satisfaction, not optimality. Routes
  seeded above and below the same obstacle settle at about 27.2073 and
  28.8746 mm, respectively.
- Attached traces can move a rigid component. The long-budget control reaches
  its known 23 mm empty-fixture lower bound without detaching terminals or
  moving external anchors.

Ten trajectories retain 5,530 independently checked frames. All final
geometries pass; three contact variants deliberately start invalid and repair
the violation in their first step. Four corruptions confirm rejection of
penetration and detached/moved endpoints. These are reduced single-layer
geometric checks, not native board admission or a swept-motion guarantee.

**Recommendation:** use this playback and independent acceptance gate when
changing the engine. Define occupancy by physical area before enabling field
weights in production, so resampling a trace does not change its pressure.
[Experiment and reproduction](../../experiments/engine-playback/README.md).

## 4. Placement deserves a larger role

The code already has random, barycentric and harmonic placement plus full-size
legalization. The experiment compares them with a new shrink-to-junction,
optimize and reinsert adapter. It retains 97 proposals, including failures.

A pinless movable wall blocks a route while the connectivity-distance
objective remains unchanged. Retained placement, barycentric, harmonic and
junction reinsertion leave it blocked. Some random starts open the passage;
a hand-selected 0.6 mm displacement does too. That displacement is an oracle
control, not an implemented autonomous blocker policy. The example explains
why routability pressure must reach bodies that have little or no net pull.

Naively shrinking constrained components changes the meaning of size-dependent
constraints. Restricting shrinkage to unconstrained components fixes that
mistake. Reinsertion still needs full-size geometry and the existing connected
spacing heuristic in the tested real-board follow-up. Only two of sixteen
one-shot random seeds legalize on the 43-component board.

Five selected legal placements then receive the same native three-connection
probe with the open-gap profile and unchanged router:

| Placement | Copper mm | Vias | A* expansions |
| --- | ---: | ---: | ---: |
| Retained | 92.432 | 0 | 247,130 |
| Random seed 2 | 94.385 | 0 | 1,011,518 |
| Random seed 8 | 119.368 | 2 | 691,937 |
| Full-size harmonic, existing spacing floor | 50.845 | 0 | 45,423 |
| Constraint-aware junction/reinsertion | 58.948 | 1 | 287,865 |

All five reach 3/3 with zero ERC, design DRC, parity or remaining connectivity
findings, and preserve their proposed placement. Raw library metadata warnings
vary from 19 to 29. Placement uses the full 52-connection netlist; routing then
prunes to the first three. This is neither a cold-pruned-prefix placement
comparison nor full-board completion. Search expansions exclude placement and
native verification work.

**Recommendation:** retain full-size harmonic placement as a strong baseline,
add targeted congestion-driven moves and a small archive of legal restart
candidates, and rank them using actual routing. The evidence does not yet
justify a genetic population or replacing the dimension-aware placer with
junctions. The subsequent [19-connection paired follow-up](2026-09-07-dense-placement.md)
supports harmonic's copper-quality advantage, but also exposes distinct search
budget and retained-copper failures requiring different recovery actions.
[Experiment and reproduction](../../experiments/placement-exploration/README.md).

The relevant VLSI parallel is analytic placement with routability feedback.
[OpenROAD's global placer](https://openroad.readthedocs.io/en/latest/main/src/gpl/README.html)
combines electrostatic placement with congestion estimation and inflation of
cells in congested areas. Adapting that feedback to movable PCB bodies is a
more specific next experiment than copying a complete IC placement system.

## Constraint programming and the next implementation boundary

Constraint programming should not be dismissed categorically. It can be
tested on a small discrete master problem: choose corridors, layer use,
ordering, or one of several legal placements, subject to known capacity
constraints. The geometric solver realizes that choice and returns evidence.
This is a proposed decomposition, not a tested result from this round.
Encoding every detailed coordinate and all PCB rules in one solver is a much
larger undertaking and is unnecessary for testing the idea.

The immediate product improvement to pursue is a shared failure record:
which passage is constrained, what occupies it, how much width is missing,
which objects can move, and whether failure means exhausted search, rejected
geometry, or a sound capacity certificate. That evidence can drive both
discrete rerouting and placement, followed by continuous relaxation and native
verification. Agents should diagnose and improve these mechanisms when they
stall, consistent with the autonomous placer/router goal.
