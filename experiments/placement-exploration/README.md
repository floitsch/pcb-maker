# Placement exploration, 2026-09-07

These experiments call the **actual `pcb-placement` library**, with a small
experiment-only Rust adapter. No production code or defaults were changed.
The output is in `build/algorithm-exploration-2026-09-07/placement/`.

- `overview.html`: entry page linking every follow-up and native comparison.
- `index.html`: all initial proposals, including failed ones and junction seeds.
- `constraint-aware/index.html`: retained constraint-aware follow-up failures.
- `constraint-aware-with-floor/index.html`: corrected legal junction seed and reinsertion.
- `native-comparison.html`: five independently routed native layouts with copper metrics.
- `results.json`, follow-up `results.json`, and `validation.json`: raw evidence.

These standalone HTML files animate **proposal selection**, not interpolated
force motion. No interpolation is presented as a collision-free trajectory.
The separate engine experiment handles actual force trajectories.

## Protocol

The initial suite has 86 proposals: retained, barycentric, full-size harmonic,
and a new shrink/restore junction adapter, plus 16 one-shot random seeds and
eight randomized junction seeds per fixture. The movable-wall fixture adds two
hand-selected ±0.6 mm controls. Every proposal gets an SVG, even when rejected;
rejected proposals show a clearly labeled input view. Eleven follow-up
proposals retain failures and test constraint-aware reduction and the existing
harmonic connected-pair spacing floor.

The junction adapter reduces movable bodies to 0.02 mm, collapses their pin
offsets to the center, removes pad shapes and keepouts for seed generation,
calls production harmonic placement, and restores the **original complete
problem** for production projection and validation of rigid component poses.
This is a new experimental adapter, not a claim that existing harmonic
placement already does physical component removal/reinsertion.

The first version deliberately exposed a bad modeling assumption: shrinking
ESP bodies changes what their face-board-edge constraints mean. Its failures
are retained. The constraint-aware version only shrinks free components with
no region and no appearance in any relational placement constraint: 26
components in the full fixture. Other component dimensions and all represented
constraints remain intact. Reducing constrained bodies requires an explicit
constraint transformation, which is not implemented here.

The first suite disables harmonic's connected-pair spacing floor to isolate
basic body legality. On the real board that ablation fails a capacitor/ESP
maximum-distance constraint. The follow-up retains the same configuration
except for enabling the existing spacing floor. Both full-size harmonic and
constraint-aware junction reinsertion then produce legal full-size poses.
The floor is a conservative radial spacing heuristic, not an exact copper
capacity model; this observation does not justify it universally.

## Results

| Fixture | Retained / local / junction result | Random restart result |
|---|---|---|
| Movable pinless wall | Retained, barycentric, harmonic and naive junction: 0/1 routed. Both hand-selected 0.6 mm moves: 1/1. | 16/16 single random proposals legal and route 1/1. |
| Crossed resistors | Retained: 4/4, 64.0 mm. Barycentric, harmonic and junction: 4/4, 27.2 mm. | 16/16 placements legal, only 8/16 complete all four routes with the fixed route order. |
| Full dual ESP32 | Retained, corrected full-size harmonic and corrected junction proposals are legal. | Only 2/16 one-shot random proposals pass production full-size legalization. |

The wall's two centered gaps are 1.2 mm. Its 0.6 mm trace plus two 0.4 mm
clearances needs 1.4 mm. A 0.6 mm shift opens a 1.8 mm passage. The net-distance
surrogate and ratsnest demand are identical before and after: a pinless
blocker receives no connection attraction. This is a clean reason to feed
route-derived passage pressure to placement. The ±0.6 mm shifts are **manual
diagnostic oracles**, not a claimed new pressure algorithm.

The micro-fixture routing probe is deliberately small: single layer, four
neighbors, 0.2 mm pitch, fixed net order, circular pads and axis-aligned wall
keepouts. It uses trace width and clearance, checks entire edges continuously,
and checks exact off-grid pad access. Every output edge is replayed through
the same geometric predicates. This is not an independent router or a native
DRC certificate, and search failure is not an unroutability proof. Full-body
SAT, board containment and movement/rotation lock checks are implemented
independently in Python; production projection additionally checks full
pad-extended envelopes and represented relational constraints.

## Native checks of realistic seeds

All five full-board legal seed types were independently probed using the
production router and KiCad. Placement was computed from full connectivity;
then the same first three connections were selected, and those proposed poses
were preserved by the declared policy. Each native run starts from zero
copper. This is **a partial routing probe of full-connectivity placement
seeds**, not the usual benchmark that prunes connectivity before generating
placement. It cannot establish full-board completion or predict the eventual
winner.

| Full-connectivity placement seed | Native prefix | Copper mm | Vias | A* expansions |
|---|---:|---:|---:|---:|
| Retained | 3/3 | 92.431666 | 0 | 247,130 |
| Random seed 2 | 3/3 | 94.384655 | 0 | 1,011,518 |
| Random seed 8 | 3/3 | 119.368227 | 2 | 691,937 |
| Existing full-size harmonic, spacing floor enabled | 3/3 | **50.845383** | **0** | **45,423** |
| Constraint-aware junction reinsertion, spacing floor enabled | 3/3 | 58.948445 | 1 | 287,865 |

Every native result has zero ERC, design DRC, parity and selected connectivity
findings. Library metadata warnings are retained and vary by pose: respectively
21, 29, 28, 19 and 22 for the five rows above. Proposed semantic
poses match the generated poses; initial and final native footprint poses
match exactly. Native verification automatically captured progress SVGs.
`validation.json` measures native copper and records these checks.

These five native probes were selected from 97 placement proposals: the
retained real-board pose, both one-shot random real-board proposals that
passed legalization, the successful full-size harmonic follow-up and the
successful constraint-aware junction follow-up. Rejected placements were
not sent to native routing. The micro-fixture proposals were evaluated only
with the explicitly limited toy route probe.

The current dimension-aware harmonic policy wins this small probe. Explicit
junction reduction remains a plausible portfolio proposal, but this run does
not show an advantage over the existing implementation. Random search is
useful for escape/diversity, and should retain failed proposals and be ranked
by routing outcomes. A favorable net-length score alone is insufficient.

## Reproduce

From the repository root, with Rust dependencies cached and KiCad available:

```sh
cargo build --release --offline --manifest-path experiments/placement-exploration/Cargo.toml --target-dir build/algorithm-exploration-2026-09-07/placement/target
python3 experiments/placement-exploration/run.py
python3 experiments/placement-exploration/constraint_aware.py
python3 experiments/placement-exploration/constraint_aware.py --connected-floor
python3 experiments/placement-exploration/native_probe.py
python3 experiments/placement-exploration/native_probe.py --followups
python3 experiments/placement-exploration/summarize.py
```

The native scripts use the existing `target/release/pcb-maker`; they never
rebuild it. Native transactions require fresh output directories: preserve
or move the previous output tree before replaying the complete suite. Random
seeds, inputs, executable hashes, outputs and all failures are retained. Timing
is diagnostic only because native runs overlap and no controlled CPU-speed
comparison is claimed.

## Denser paired probes

`dense_probe.py` tests the retained, full-size harmonic and constraint-aware
junction seeds on the same first **19** connections under both pad-gap rule
profiles. It copies the executable and input files into a fresh output tree,
runs two jobs concurrently, and records authoritative subprocess outcomes.
Every native transaction captures an SVG automatically. `dense_summary.py`
audits every accepted prefix and creates an HTML slider comparing equal
completed prefixes across all six cases.

The frozen poses come from the full-connectivity proposal experiment above.
The declared placement policy preserves them after the routing netlist is
pruned. The audit checks this explicitly, checks zero inherited copper, and
compares native placement hashes at every accepted step and between profiles.
It measures physical centerline union, vias and actual ESP pad-gap crossings.
Failed insertion attempts must preserve the parent and roll back exactly.
Reachability preflight and A* work are recorded separately; their sum still
excludes mask construction, placement and native verification.

```sh
python3 experiments/placement-exploration/dense_probe.py build/my-dense-probe
python3 experiments/placement-exploration/dense_summary.py build/my-dense-probe
```

While jobs run, `dense_summary.py ... --allow-running` creates an explicitly
partial snapshot. It does not infer process completion from output files.
The runner also invokes the summary automatically after all six jobs finish.

Separate, fresh-output diagnostic helpers preserve the original comparison:

```sh
python3 experiments/placement-exploration/dense_followup.py build/my-dense-probe harmonic-blocked
python3 experiments/placement-exploration/dense_isolate.py build/my-dense-probe junction-blocked
python3 experiments/placement-exploration/dense_followup.py build/my-dense-probe junction-blocked --policy ripup --maximum-connections 1
python3 experiments/placement-exploration/dense_diagnostics.py build/my-dense-probe
```

Guidance requires an observed budget failure and changes only the A*
heuristic. Its reverse-distance preprocessing is counted separately. Isolation
routes the failed connection first from zero copper and audits physical pad
geometry, target membership, outlines, rule areas, native design settings and
poses against the original target. Rip-up requires a grid-disconnection
failure and enables the existing bounded one-net repair; a yielded net must
be restored and the complete prefix must pass native verification.

The retained run is `build/placement-dense-prefix19-2026-09-07/`, with its
matched-prefix viewer in `index.html` and separate counterfactual images in
`diagnostics.html`. One initial guided replay was interrupted (tool exit 143)
before a result; it is retained separately from the successful fresh replay.
The full comparison and interpretation are in
[the dense placement review](../../docs/reviews/2026-09-07-dense-placement.md).
