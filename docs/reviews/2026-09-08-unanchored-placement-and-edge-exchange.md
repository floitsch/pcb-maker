# Unanchored placement and edge-rule exchange

The most consequential finding is that the larger all-free benchmark was
using random placement followed by legalization, despite selecting a harmonic
initializer. A first net-aware seed passes native placement checks and is now
ready for cold whole-board routing. No completed routing improvement is claimed.

[Dashboard and combined views](../../build/edge-exchange-2026-09-08/index.html)
· [Summary](../../build/edge-exchange-2026-09-08/summary.json).

## Why net attraction was ineffective

The harmonic implementation deliberately leaves connected movable blocks with
fewer than two distinct positional anchors inactive: an unconstrained Laplacian
is singular and a singly anchored block collapses. The larger benchmark makes
all 68 components free and fixes their original rotations. Its entire graph
therefore uses the configured random fallback. Connected-pair spacing is
disabled, so the later legalizer contributes no net-dependent spacing either.

A controlled ablation retains all 68 components, 165 pads, 50 nets, widths,
clearances and constraints, but sets every net's `tension_weight` to zero.
Every final component pose is identical to the existing baseline, including
the selected random attempt. The initializer's generic
`embedding_guidance_used` flag does not establish effective positional guidance;
its underanchored-block evidence is essential here.

This is not a failure of the anchored harmonic method. It is a missing
net-aware initialization strategy for the all-free case we are benchmarking.

## First net-aware seed

`probe_unanchored_spectral.py` constructs a weighted component graph. Each net
uses declared width times tension weight, divided by its other component
count, so high-fanout rails do not gain attraction merely from clique expansion.
It computes two nonconstant normalized-Laplacian coordinates. Two experiments
use the continuous coordinates or their ranks, followed by the existing
production harmonic legalizer with the generated positions retained as the
underanchored seed. No artificial fixed anchors are introduced.

The continuous version fails legalization and is retained as a rejected
proposal. The rank version succeeds:

| Placement surrogate | Random fallback | Spectral rank |
| --- | ---: | ---: |
| Connectivity-distance estimate | 12,321.361 mm | 6,535.585 mm |
| Estimated cross-net proper crossings | 1,597 | 107 |
| Squared cell demand | 15,782.835 | 4,592.109 |

These are placement surrogates, not routed trace length or actual copper
crossings. The script currently supports wholly movable, fixed-orientation,
connected problems; disconnected blocks and partially constrained placement
require separate handling. NumPy version and eigenvalues are recorded.

`materialize_placement_probe.py` checks that the probe changes only initial
positions, applies the accepted poses using the existing native adapter,
restores original project settings, verifies the board, audits every native
pad/footprint, and runs the protected annotation repair. The result has the
same 8,057.413 mm² area, component/net/pad inventory, source rules and original
orientations. There are no physical DRC findings; three annotation findings
remain. All 112 initial open items remain because routing has not run.

An initial manual materialization missed the existing runner's project-restore
step and failed its inventory audit after KiCad migrated the project. The
reusable helper now follows that established contract. The failed attempt
remains under `spectral-native`; only `spectral-native-final` passes admission
as a provisional cold placement.

## Edge-rule translation

The new opt-in `--translate-edge-rule` comparison path loads the pinned
Freerouting representation before and after translation. A dedicated boundary
clearance class encodes the native global edge requirement minus the loaded
outline's geometric half-width. The adapter checks:

- the exact DSN tree changes only at the boundary class and added edge rule;
- geometry, net classes and pin inventory remain intact;
- all loaded non-edge clearance matrix entries remain identical;
- the loaded nominal edge requirement matches the native project;
- unrepresentable values and custom-rule files are rejected explicitly.

On the larger input the effective requirement changes from 0.31 to 0.01 mm,
and all 14 cold-input edge findings disappear. Freerouting's new run completes
normally after 315.260 seconds of process time and imports with exact source
poses, unchanged project settings and matching copper dimensions. KiCad
reports 66 opens and seven unchanged annotation findings. The earlier run
left 65 opens. Neither run completes this placement; the correction alone
does not establish a routing improvement or prove placement infeasibility.

The strict 0.5 mm control retains real native and external edge findings.
Values below the external 0.01 mm outline half-width and values off its 100 nm
grid fail instead of being silently approximated. All three controls retain
native renderings and unchanged source copper/placement.

ECC83's nominal edge rule also matches after translation, but two layer-specific
findings remain on P3.2. Its native lower edge is at 131.631172 mm; the DSN
export rounds that boundary to 131.631 mm, while the imported pad reaches
131.6212 mm. This leaves a 0.0002 mm overlap with the external outline obstacle.
It is a separate geometry-precision discrepancy, not a reason to waive the
findings. Full exchange equivalence remains unproven.

## Next implementation decision

Route all 50 nets from zero copper on `spectral-native-final/native-silk`,
using the same source classes and comparable bounded policies. Generate demand
from the new placement rather than copying the earlier spatial forecast.
Compare against independent cold random and fixed-reference controls. If the
net-aware seed improves complete-board outcomes, integrate a general
underanchored placement policy and test another board family. Do not spend the
next iteration merely increasing nested repair depth on the random placement.

The successful 34-net/24-open incumbent is unchanged. The spectral placement
and edge adapter are opt-in experiments; neither is promoted as a complete
layout solution.

## Commands

```sh
python3 experiments/whole-board/compare_native_router.py \
  --sequence build/nested-recovery-2026-09-08/finish \
  --binary build/restoration-coverage-2026-09-08/pcb-maker \
  --output build/edge-comparison-NEW --translate-edge-rule

python3 experiments/whole-board/probe_unanchored_spectral.py \
  build/general-layout-2026-09-08/complex-offset/area-00 \
  build/restoration-coverage-2026-09-08/pcb-maker build/spectral-seeds-NEW

python3 experiments/whole-board/materialize_placement_probe.py \
  build/general-layout-2026-09-08/complex-offset/area-00 \
  build/spectral-seeds-NEW/rank \
  build/restoration-coverage-2026-09-08/pcb-maker build/spectral-native-NEW
```
