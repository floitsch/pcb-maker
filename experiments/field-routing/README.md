# Diffusion and congestion feedback experiment

This isolated CPU prototype tests whether a pressure-like field is useful for
routing. It does not modify the production router. Existing project harmonic
work concerns placement (`docs/experiments/harmonic-dual-esp32.md`); existing
`dut-grid-negotiated.json` already exposes routing congestion and history costs.
The useful question here is what the field itself adds to that coordination.

Run from the repository root with Python 3 and NumPy:

```sh
python3 experiments/field-routing/run.py --output build/algorithm-exploration-2026-09-07/field/final
python3 experiments/field-routing/check.py build/algorithm-exploration-2026-09-07/field/final
```

Choose a fresh output directory to preserve earlier trials. Every trial writes
an animated standalone HTML file and a final SVG, including incomplete or
conflicting layouts. `index.html` combines all trials with a selector and
playback slider. `results.json` retains actual intermediate potentials and
routes; `metrics.json` omits large visual arrays. `validation.json` records
rechecking, corruption probes, and script hashes. The initial 90-trial suite
and the later negotiated-control suite remain in the parent artifact folder.

## Model and controls

Each free cell is a unit-capacity routing resource. Obstacles remove cells.
Four or eight planar neighbors are allowed, with strict diagonal corner
checks. A transition between coincident cells on two layers costs 6 planar
units and occupies both layer cells. All benchmark terminals are fixed on the
front layer; routing on the back requires explicit transitions back to their
declared layer. Plated-terminal alternate layer contacts are not modeled.
This is a discrete experiment: no trace
diameters, pad geometry, via holes, electrical constraints, or KiCad DRC.

The field is an electrical resistor / diffusion analogy, **not Navier–Stokes**.
Each net gets its own potential, fixed to 1 at the source and 0 at the target;
obstacles are insulating. Conductance is the inverse of edge length multiplied
by congestion price. Damped Jacobi sweeps solve the weighted Laplace equation.
A route follows the greatest descending current. This makes a discrete path
from a spread-out field; it does not minimize copper length. Disconnected
terminals are detected with a separately counted graph traversal. A field
iteration limit or flat numerical gradient is treated as a failure, never
silently replaced with shortest-path routing.

Seven methods isolate the effects:

- Sequential Dijkstra and sequential field routing reserve each earlier path.
- Independent simultaneous fields ignore other nets until validation.
- Field and Dijkstra feedback allow overlaps, then add historical price 8 to
  multiply occupied cells and reroute all nets (24-pass cap).
- Field and Dijkstra negotiated variants additionally price cells occupied by
  earlier nets in the current pass. Order is shuffled with fixed seeds 1, 7,
  and 19. Feedback-only variants have no within-pass coupling, so their seed
  repeats are deterministic; they are not independent evidence of robustness.

Five fixtures run with both neighbor choices. Three include independently
checked valid witnesses: a wall detour, a two-net corridor order trap, and a
two-layer crossing. Two are known impossible: a solid separating wall, and a
one-cell crossing whose unique center is required by both nets. The independent
validator checks terminals, obstacle membership, neighbor movement, diagonal
corner cuts/crossings, layer transitions, and shared cells. Corruption probes
exercise nine corruption cases, including occupancy on each side of a via and
incorrect terminal layer identity. Unknown failures elsewhere would not constitute
an impossibility proof.

## Observed results

These observations come from 150 final trial rows, representing five small
fixtures with multiple policies, neighbor choices, and deterministic seeds.

| Fixture | Sequential shortest path | Independent field | Feedback only | With present occupancy |
| --- | --- | --- | --- | --- |
| Wall detour | 28 cells (4-neighbor) | 42 cells | Same respective lengths | Same respective lengths |
| Corridor order trap | Blocks second net | Both connect but collide | Both methods succeed in 3 passes | Both succeed in 2 passes |
| Two-layer crossing | Succeeds immediately, 2 vias | Both connect but collide | Field succeeds in 10 passes; Dijkstra oscillates for 24 | Field succeeds in 3; Dijkstra in 2, both 2 vias |
| Single-layer crossing | Incomplete | Collision | Collision persists | Collision persists |
| Disconnected wall | Incomplete | Incomplete | Incomplete | Incomplete |

Eight-neighbor routing shortens the wall detour to 20.971 cells for Dijkstra
and 29.314 for the field. The narrow corridor fixtures have no legal diagonals,
so their results do not change. The three negotiated seeds agree on completion
and quality in these fixtures, although work counts and some routes differ.

The four-neighbor wall field needs 3,885 sweeps / 1,173,270 cell updates plus
302 reachability visits, versus 255 Dijkstra expansions. These operations have
different costs and parallelism. Recorded CPU timings are Python prototype
timings, not a production comparison; no GPU has been tested. Final accounting
includes both active connected cells and all allocated array cells touched by
Jacobi, so disconnected array regions do not disappear from the work report.

The important follow-up was the two-layer oscillation: adding current-pass
occupancy makes the shortest-path method succeed faster in passes than the
field, without changing its path finder. The initial apparent field advantage
was therefore not good evidence to replace discrete search.

## Recommendation

Keep the pressure idea as explicit congestion prices and diagnostic maps.
Prioritize coordinated rip-up, present/history costs, and scarce-channel
feedback into placement. Retain a field backend as an experimental proposal
generator, but these cases do not justify investing first in a GPU fluid router.
A GPU can accelerate local updates; it does not remove the need to assign
separate nets, resolve shared capacity, extract paths, and verify actual widths.

The next fair GPU experiment would compare batched field solves against batched
distance/wavefront propagation on the same native-rasterized geometry, include
iteration count and transfers, and pass every extracted route through native
verification. This prototype has no inertia, moving copper, trace widening,
component motion, or continuous 3D fluid volume. Its two layers are a routing
graph with via transitions; a true fluid simulation remains untested.

Primary-source parallels: [Connolly's harmonic-field work](https://www.cs.cmu.edu/~motionplanning/papers/sbp_papers/integrated1/connolly_harmonic_application.pdf)
motivates the obstacle-aware potential; [McMurchie and Ebeling's PathFinder](https://janders.eecg.utoronto.ca/1387_2015/readings/pathfinder.pdf)
motivates current and historical resource prices. This is a small analogy and
ablation experiment, not a reproduction of either complete algorithm.
