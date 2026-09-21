# Pressure-directed blocker repair

This experiment ports the useful outer-loop idea from
`testing-esp32-duts/packages/pcb-router/src/placement/clearance_inflation.toit`
without treating its proposal policy as proven. A failed grid search now
retains the physical centroid of the blocked frontier attributed to each
semantic obstacle. The coordinator converts that evidence into component
pressure, proposes bounded translations, reroutes transactionally, and only
commits an independently exact-assessed improvement.

The policy is selectable as `repair-route-pressure`; the existing axis sampler
remains available as a control. Two direction producers share the same
transaction/evidence boundary:

- `frontier_centroid` uses the mean A* contact vector;
- `passage_capacity` measures axis-aligned gaps to neighboring routing-keepout
  bodies, computes the trace-width capacity deficit, and rounds the requested
  opening outward to the router's first grid before legal-region projection.

## Reduced evaluation

Fixture: `placement-perturbation-unblocks-routing.json`, starting from the
harmonic placement and using `dut-grid-adaptive.json`.

| Policy | Repair attempts | Total A* expansions | Selected wall move | Result |
| --- | ---: | ---: | ---: | --- |
| Axis sampling | 1 | 436,241 | -0.8 mm Y | exact pass |
| Frontier pressure, 0.5 mm × `[1, 2]` | 2 | 868,461 | +0.8 mm Y | exact pass |
| Passage capacity | 1 | 436,241 | -0.8 mm Y | exact pass |

The pressure experiment works, but it is worse on this control: one extra
failed reroute and almost exactly twice the search work. The evidence explains
why. The movable wall's frontier centroid is approximately `(16.820, 15.000)`
and its center is `(20.000, 15.000)`, producing pressure `(3.180, 0.000)`.
That direction is horizontal, while the wall is constrained to vertical
motion. The policy must therefore fall back to both legal vertical directions;
the first 0.5 mm proposal remains blocked and the projected 0.8 mm proposal
opens the channel. The axis baseline deliberately tries the region extreme
first and succeeds immediately.

This was not evidence against pressure-driven feedback in general. It showed
that a centroid of the A* blocked frontier answers “where did search hit this
object?”, which need not answer “along which legal degree of freedom should
the object yield?”. Passage capacity answers the latter for this control and
matches the axis baseline with one proposal.

A 0.6 mm move is physically sufficient and passes exact validation, but only
after the router falls through to its 0.1 mm grid, for 621,210 total
expansions. The 0.8 mm passage target creates a coarse-grid opening and finishes
in 436,241 with the current heading-aware heuristic. The proposal therefore
records physical gap, required gap,
discretization-aware target, and any remaining deficit rather than minimizing
movement alone.

## Asymmetric positive control

`benchmarks/small/passage-pressure-asymmetric.json` keeps one branch and one
movable wall, but makes the lower passage almost wide enough while the upper
passage is much narrower:

| Policy | Repair attempts | Total A* expansions | Selected wall move | Result |
| --- | ---: | ---: | ---: | --- |
| Axis sampling | 3 | 868,969 | -0.6 mm Y | exact pass |
| Frontier pressure | 1 | 436,739 | -0.5 mm Y | exact pass |
| Passage capacity | 1 | 436,739 | -0.5 mm Y | exact pass |

Here the frontier centroid has a useful negative-Y component. The passage
producer independently records a 1.5 mm lower gap, a 1.6 mm requirement, and a
0.5 mm grid-aware proposal. This is the positive control missing from the first
experiment: directed feedback halves total search work relative to uniform
axis sampling on a case where direction is observable.

The current passage model is deliberately limited to axis-aligned bounds of
component bodies that are routing keepouts plus board edges. Exact rerouting
and validation remain authoritative, so the approximation cannot certify a bad
candidate, but pad/keepout shapes and rotated narrow passages remain to be
ported.

## Bounded collision-chain control

The predecessor's forced-corridor experiment grew a same-vector move set when
a proposal increased overlap with another movable component. That mechanism
is now selectable independently of the pressure direction producer with
`push_propagation: collision_chain` and an explicit
`maximum_components_per_push` bound.

`benchmarks/small/passage-pressure-push-chain.json` contains one routing wall
and one thin movable package body. The package body is physical but not a
routing keepout, matching footprints whose package outline is not copper. The
only useful 0.5 mm wall move overlaps it if applied alone; translating the pair
preserves their separation and opens the upper passage.

| Policy | Repair attempts | Rejected placements | Routed-search expansions | Result |
| --- | ---: | ---: | ---: | --- |
| Axis sampling | 8 | 2 | 3,025,340 | incomplete |
| Passage pressure, single body | 1 | 1 | 432,178 | incomplete |
| Passage pressure, collision chain (max 2) | 1 | 0 | 623,298 | exact pass |

The chain is therefore a positive capability result. It does not establish
that collision propagation should always be enabled: the default remains the
single-component control. The current port translates every contacted member
by exactly the root displacement, rejects fixed contacts, movement-axis or
region conflicts, and over-budget chains before routing, then subjects the
whole candidate to the existing exact placement/routing gate. The semantic
model has no rigid movement-group relation yet, so group membership is not
guessed. Rotation, force sharing, nonuniform displacement, and chains longer
than this two-body control remain unevaluated.

Global failed-proposal caching was intentionally not added. A component
position rejected for one parent layout may work after another component
moves, so proposal deduplication is scoped to an exact parent iteration.

## Inspection

Generate the result and viewer with:

```sh
cargo run --release -- repair-route-pressure \
  benchmarks/imported/layout-trace/placement-perturbation-unblocks-routing.json \
  harmonic run/wall-pressure.json \
  experiments/configs/dut-grid-adaptive.json \
  experiments/configs/blocker-pressure-repair.json

# Select the passage-capacity direction producer instead.
cargo run --release -- repair-route-pressure \
  benchmarks/imported/layout-trace/placement-perturbation-unblocks-routing.json \
  harmonic run/wall-passage.json \
  experiments/configs/dut-grid-adaptive.json \
  experiments/configs/blocker-passage-repair.json

cargo run --release -- view-route \
  benchmarks/imported/layout-trace/placement-perturbation-unblocks-routing.json \
  run/wall-pressure.json run/wall-pressure.html
```

The route viewer now plays retained attempts. Cyan markers show classified
frontier centroids and gold arrows show proposed component motion. Rejected
pressure attempts remain in playback as projected poses over their parent
candidate; collision chains draw every moved body and expose their contact
evidence. On the original wall fixture playback shows the unusable horizontal
pressure, the still-blocked 0.5 mm move, and the accepted 0.8 mm move.
