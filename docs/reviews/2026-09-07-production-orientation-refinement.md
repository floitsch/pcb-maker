# Production orientation refinement after legalization

The diagnostic identified orientations that became poor choices after component
legalization. Initial placement now has an opt-in production refinement stage
that revisits rigid quarter-turns at the final component centers. It runs before
placement evidence is computed and before archive deduplication and ranking, so
both ordinary placement and placement archives consume the refined poses.

Enable `orientation_refinement` in an initial-placement configuration. Omission
preserves existing behavior and serialized evidence; the default-off full-board
result matches the previous result exactly. A complete example is
`experiments/configs/harmonic-ports-orientation-refinement.json`.

The algorithm visits components in stable ID order. For each freely rotatable
component, it scores the three other quarter-turns from the current pose. It
validates every qualifying improvement against the original movement, rotation,
board, region and relational constraints and pad-extended placement exclusions.
It never projects a rejected trial into feasibility. The best legal improvement
for that component is committed, then subsequent scores use the updated poses.
Fixed centers remain unchanged, including for components whose movement would
otherwise permit translation.

Default limits are four sweeps and 512 scored trials. A partial search returns
the legal result found within its budget and reports the limiting budget. A
sweep with no accepted changes reports no improving legal quarter-turn **above
the configured and numerical improvement thresholds**. This is coordinate
convergence within the quarter-turn orbit, not a global placement optimum.

The attraction objective uses physical pad centers and declared width/tension
weights. Legacy branches contribute their squared minimum endpoint distance;
explicit electrical nets contribute the mean squared minimum distance over all
terminal pairs. Physical pads on permitted layers are endpoint alternatives.
Pins without such pads use their logical offsets. The mean avoids quadratic net
size scaling, but still costs quadratic work in a multi-terminal net's terminal
count. Pairwise minima may choose inconsistent pad alternatives across pairs;
this remains a geometric surrogate without a routing or layer-change cost claim.
Electrical geometry and clearance are not weakened by a low attraction weight.

Evidence includes all trial scores, accepted and unselected legal alternatives,
rejection reasons, per-net costs before and after, termination, and the number
of candidate legality queries. The legality count excludes input/final checks;
initial projection work remains in its existing separate evidence fields.

## Retained full-board experiment

The corrected dual-ESP32 placement accepts nine rotations across eight
components, with `D_LED` updated a second time after its neighbor changes.
Thirteen qualifying alternatives are rejected. Three sweeps score 297 trials
and issue 29 candidate legality queries. The attraction score falls from
6216.820769 to 6182.043950. No centers move.

Accepted final rotations change `C_E_A`, `C_E_B`, `C_W_A`, `C_W_B`, `C_W_EN`,
`D_LED`, `R_E_EN`, and `R_LED`. Per-net costs retain tradeoffs such as longer
GND connections; a lower weighted score is not electrical validation.

A matched native probe routes `LED_SERIES` at 0.35 mm width and 0.20 mm clearance:

| Poses | LED copper | Vias | A* expansions |
| --- | ---: | ---: | ---: |
| Corrected harmonic, refinement disabled | 5.441 mm | 0 | 750 |
| Production orientation refinement enabled | 2.027 mm | 0 | 60 |

Both cases use the same frozen executable and routing/template settings. Native
problem comparison proves that only the eight component rotations differ.
All 43 centers are unchanged and materialization/routing preserve the proposed
poses. Both have zero ERC, design DRC, parity and selected-net open findings.
Metadata warnings are twenty and twenty-one, respectively.

The seven new regressions bring the placement suite to 33 passing tests. They
cover the planted optimum and rejected perpendicular turns, zero attraction,
trial/sweep budget behavior, explicit nets with alternative physical pad centers,
a three-terminal objective and permutation stability, opt-in/archive integration,
and invalid input. An independent Python audit recomputes all 297 full-board
trial scores, checks best legal choices and budgets, and exactly replays the
final poses. SVGs capture the input and all nine accepted changes; twelve of the
thirteen rejected trials also have previews, with every rejection retained in
JSON. Viewer JavaScript syntax passes; real browser playback was not tested.

The release build passes. Native executable SHA-256:
`ea403fddec48ec1b1333a6c65d9996fc56fd78df75788bc6dcebb9167e147a16`.
The native probe covers only the LED connection. The changed orientations still
need the dense routing comparison before considering default enablement. The
existing nineteen-connection results apply to the pre-refinement orientation
checkpoint, not this result.

- Implementation: `crates/pcb-placement/src/orientation.rs`
- Integration: `crates/pcb-placement/src/placement.rs`
- [Accepted-change animation](../../build/orientation-refinement-2026-09-07/refined/refinement.html)
- [Native refined LED route](../../build/orientation-refinement-2026-09-07/refined/native-preview.svg)
- [Trial and per-net evidence](../../build/orientation-refinement-2026-09-07/refined/placement.json)
- [Independent score audit](../../build/orientation-refinement-2026-09-07/refined/independent-score-audit.json)
