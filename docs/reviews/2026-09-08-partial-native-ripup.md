# Restoring blocking nets during partial native routing

The retained 68-component complex-board route stopped at `Net-(U301A--)`,
with 23 routed nets and 42 native open items. The target has two pad centers.
Its reachability preflight explored only 337 cells before proving that no path
exists on the configured grid. Increasing the A* expansion limit would not
address that disconnection.

## Diagnosis and native restoration

The existing yielding diagnosis tries removing each of the 23 routed nets
individually, preserving its pads, holes and all other obstacles. Five removals
allow a target route: `+12V`, `-VAA`, `GND`, `Net-(D304-K)` and
`Net-(U102-CAP+)`. These are alternative ways to release space, not a claim
that all five must move. All single-net combinations were evaluated.

The first diagnostic control kept the full forecast set from the selected
pass configuration. A second removes the forecasts for the 23 completed nets,
matching the actual failed step's demand context. Both identify the same five
yielding options. The restoration experiments use the second configuration;
they also remove the target's forecast when rerouting the yielded net.

Restoring `+12V` fails after routing the target. Each of the other four repairs
connects the target and restores the yielded net. Native open items decrease
from 42 to 41, with the original seven annotation findings and no added design
findings. Native checks of the remaining unconnected-item identities prevent
trading one completed net for a different broken one. These are partial-board
repairs; the board is still incomplete and earns no area credit.

## Program change

`repair-kicad-single-connection-ripup` now supports explicit
`allow_partial_progress` and `allow_existing_annotation_findings` flags. Both
default to false. Partial mode requires a discovered target with no existing
copper, and computes its expected connectivity reduction from its pad count.
Every retained repair must:

- Pass native ERC, parity and physical DRC, with only explicitly allowed,
  unchanged source annotations remaining.
- Reach the exact expected number of open items, with every remaining native
  open finding present in the baseline, including affected object identities.
- Preserve the project, ERC inputs, fixed board objects, and every copper
  object outside the target and yielded net.

An accepted incomplete board is labelled `progress`; `complete` still means
full native completion. The repair CLI can exit successfully for its explicitly
requested partial action and prints `native-complete=false`. This does not
change the complete-layout CLI or area-admission gates. Declared-rung insertion
rejects partial mode, since that workflow requires a complete child.

The program retains a separate provisional directory before restoring the
yielded net. Its native reports, combined copper render and labels cannot be
overwritten by the final repair. All experiment comparisons hide component
bodies and show both copper layers together.

Partial-mode selection compares the entire resulting board's physical length
plus its configured via penalty. Comparing only the target and rerouted net
would bias the choice toward yielding short nets, because each alternative
removes a different amount of original copper. The legacy complete-board
mode retains its previous scoring policy.

## Checks and remaining integration

All 123 KiCad unit tests pass. Added regressions reject a same-count replacement
disconnection, changed unrelated copper, changed component position and
incorrect native counters. The strict native control rejects the annotated
partial source. The already-routed-target control rejects duplicate insertion.
An older dual-ESP32 repair still produces a fully native-complete board.
The final executable selects a board byte-identical to that legacy control.

The final partial repair selects `Net-(D304-K)` as the yielded net. All four
restorations remain admissible. The program keeps the original annotations
visible and reports `native-complete=false`. Its independent audit validates
48 SVG files as well as the native geometry and selection checks.
See the [combined comparisons and complete evidence](../../build/complex-conflict-2026-09-08/partial-final/index.html).

The independent audit uses isolated native readback processes to check component
poses, unchanged copper UUIDs and geometry, source project bytes and requested
track/via dimensions. It recomputes the native progress gate and selection from
the stored evidence. Every source, temporary removal and restored state has a
render. Evidence lives under
[`build/complex-conflict-2026-09-08`](../../build/complex-conflict-2026-09-08).

This command now supplies a reusable repair action for an incomplete board.
The subsequent [automatic recovery integration](2026-09-08-sequential-ripup-recovery.md)
now invokes this action from the sequential/adaptive coordinator and continues
the retained routing prefix. It keeps the same native gates and preserves
successful copper outside the repaired nets.
