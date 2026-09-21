# Net-aware placement unlocks whole-board routing

The same router now connects all 50 routable nets of the 68-component
`complex_hierarchy` board in its first pass after replacing the unanchored
random fallback with a net-aware spectral rank seed. Native verification finds
zero opens, no ERC/parity findings and three unchanged silkscreen findings.
This is electrical routing completion, **not a fully admitted layout**.

[Comparison and combined copper views](../../build/spectral-routing-2026-09-08/index.html) ·
[50-net routing playback and independent audit](../../build/spectral-routing-2026-09-08/spectral/adaptive/pass-000/recovery.html).

## Matched placement experiment

Both cold sources retain 68 components, 165 pads, all 50 routable nets,
source net classes, fixed original orientations and 8057.413 mm² board area.
Movable centers are regenerated under the explicit all-free benchmark policy;
this does not recover the original product's mechanical requirements.
The sources begin with zero copper and 112 native open items. Both use the same
frozen router, configuration, initial net order and two-pass budget. Each
placement gets independently computed future-routing forecasts.

| Router / finished result | Random placement: native opens | Spectral placement: native opens |
| --- | ---: | ---: |
| pcb-maker, best of two passes | 30 (30/50 nets routed) | 0 (50/50 nets routed) |
| Freerouting, explicit two-signal-layer control | 2 | 0 |

Both two-pass jobs are terminal and independently sequence/selection audited.
Both spectral passes connect all 50 nets; random passes route 30 and 29 nets.
Both coordinators retain pass 0. The spectral run's four repair-tree audits
also pass. Frozen configurations, binaries, project files and native classes
are checked equal again after completion.
Concurrent execution and different router budgets prevent an equal-budget
performance ranking.

The successful pass commits two retained-prefix repairs, including one nested
restoration. Its sequence and both repair trees pass independent connectivity,
unchanged-copper, pose and requested-dimension audits. This shows the existing
recovery mechanism working on a better initial placement; it does not require
another expansion of the via-removal search.

Three silkscreen findings remain: U101's reference overlaps C103 silkscreen;
the D101 and D203 `K` labels overlap R301 and R206 pads respectively. Eight
library metadata warnings are tracked separately. Full native admission and
area credit remain withheld. We have not tested a smaller spectral board.

## External comparison correction

The source's front copper layer exports as `(type power)`, which the pinned
Freerouting loader disables for ordinary routing. Earlier edge-corrected
external controls therefore routed only on the back layer, leaving 66 opens
on the random placement and 26 on the spectral placement. Those are not
two-routing-layer comparisons against pcb-maker.

The new opt-in `--all-copper-signal-layers` adapter changes only exported layer
roles and checks AST equality everywhere else. The loaded external settings
confirm that both layers are signal and active. The existing edge adapter
matches the nominal native global edge rule and preserves all non-edge
clearance-matrix entries. Native re-import checks component poses, project
rules and copper dimensions. Full DSN geometry equivalence remains unproven;
the layer adaptation explicitly changes the source layer-role policy.

## Integration and next priorities

`area_probe.py --placement-seed spectral_rank` now exposes this initializer
through the general native-project pipeline. It uses connectivity, widths and
tension weights, without reading original component centers. It requires a
entirely movable graph with fixed orientations; the subsequent
[native geometry expansion](2026-09-08-native-placement-geometry.md) adds
disconnected blocks. Unsupported policies fail explicitly. Production legalization and native checks still decide
whether the seed is usable.

The integrated driver reproduces the experiment's entire seeded problem and
all accepted poses exactly. Python syntax checks pass for the changed helpers;
the native source-mode import controls pass, including real-move, rotation,
unsupported-resolution and tampered-session rejection. No Rust code changed
in this placement integration.

The next implementation targets are broader placement policies (fixed anchors
and permitted rotations), recovery from stalled legalization, routing-driven placement changes,
and full-board coverage on other families. Only complete layouts should enter
the board-area comparison. Local via/length work remains in
[deferred optimizations](../optimization-todos.md).
