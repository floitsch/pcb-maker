# Actual engine playback

This harness calls `pcb_engine::compile_copper_repair` and the existing
`CpuReferenceBackend`. It does not implement a second force solver or change
production defaults. Every simulation automatically generates a standalone
HTML player, GIF, four SVG snapshots, and all saved solver frames. The HTML
has play/pause, iteration scrubbing, speed selection, initial-geometry ghosts,
separate motion arrows, and an optional pressure overlay.

```sh
python3 experiments/engine-playback/run.py \
  --output build/algorithm-exploration-2026-09-07/engine-checked
python3 experiments/engine-playback/run.py --settled-only \
  --output build/algorithm-exploration-2026-09-07/engine-convergence
python3 experiments/engine-playback/check.py
```

Dependencies: the existing Rust workspace dependencies, Python, Pillow and
`rsvg-convert`. No browser, GPU, image-generation service or network is needed
to run or render. Use a fresh output directory for new experiment settings.

## Measured behavior

| Fixture | Initial length | Final length | Observation |
| --- | ---: | ---: | --- |
| Slack, no tension, 240 steps | 30.646708 | 30.646708 | Legal geometry stays unchanged |
| Same slack, tension, 240 steps | 30.646708 | 26.013592 | Explicit length gradient tightens trace |
| Same slack, 1,200 steps | 30.646708 | 26.000000 | Reaches known straight-line bound |
| Above fixed obstacle, 480 steps | 30.000000 | 27.207298 | Tightens on the seeded side |
| Below same obstacle, 480 steps | 34.083191 | 28.874569 | Retains a longer route on the other side |
| Trace contact, no field | 44.472366 | 44.000256 | Initial clearance violation removed in first step |
| Same contact, field strength 6 | 44.472366 | 44.000256 | Exactly identical positions: field weights are zero |
| Same contact, explicit unit weights | 44.472366 | 44.000050 | Field activates; peak pressure 0.112403 |
| Free component, 360 steps | 28.943665 | 23.164524 | Body moves through rigid terminal attachments |
| Same free component, 1,800 steps | 28.943665 | 23.000000 | Reaches the empty-fixture lower bound |

Lengths are millimeters, including fixed copper stubs where present. These are
ten runs on five small mechanisms, not ten independent hardware designs.

For the coupled fixture, the two external endpoints are 26 mm apart and the
rigid component's terminal span is 4 mm. Triangle inequality gives at least
22 mm of connecting traces; the two fixed 0.5 mm stubs bring the bound to
23 mm. The final state attains this bound within floating-point tolerance.
The initial 360-step run had not yet settled, so it remains a separate
finite-budget control rather than being called an optimum.

The obstacle pair is especially useful for playback: both routes become
nearly stationary while staying on their respective sides. This is evidence
about these trajectories, not a guarantee that the engine preserves topology
under arbitrary step sizes or projections. No continuous swept-motion
certificate is implemented.

## Why earlier animations could be misleading

1. `SolverConfig::default()` sets `trace_tension_strength` to zero. A legal
   slack route need not move at all. Tension is a trace-length gradient, not
   a Hooke-law spring with a physical rest length.
2. Copper-repair compilation assigns zero `field_weight` to its particles.
   Increasing density strength alone cannot produce pressure. The explicit
   unit-weight variant is an activation ablation, not a correct area-density
   model: resampling a trace would change its deposited mass.
3. The engine already exposes tension and density proposals but does not emit
   the net positional constraint correction as a separate arrow. The harness
   reconstructs that correction from the before/after positions minus the
   proposed displacement. Red arrows are corrections, not physical forces.
4. Small constraint residual means the selected constraints are satisfied.
   It does not prove optimality. The obstacle controls illustrate why a
   discrete route change may still improve a nearly stationary legal state.

No production density weights or tension settings were changed on the basis
of these tiny examples. A useful occupancy model should preserve deposited
area under trace resampling and account for rigid bodies and layers.

## Validation and rendering

A separate Python checker recomputes Euclidean segment/segment and
segment/rotated-rectangle clearances, fixed geometry, board bounds, body-local
attachments, and shared external anchors from the saved geometry. It does
not accept the engine's residual as evidence. All final states pass with
1e-4 mm clearance/attachment tolerance. The three contact variants deliberately
fail only at their injected initial frame; all other saved frames pass.
Fixed geometry does not move in any saved frame.

Corruption probes reject an obstacle penetration, displaced fixed endpoint,
detached rigid-body terminal and detached shared external anchor. The two
empty-fixture lower bounds and inactive-field equality are checked separately.
These checks cover the explicit reduced model, not full PCB design rules,
unlisted electrical relationships, via geometry or KiCad admission.

Static SVG output was rasterized and visually inspected. Viewer JavaScript was
syntax checked and exercised with a mock DOM/canvas across the coupled
trajectory. Headless Firefox crashed in this environment, so an actual browser
playback check is not claimed. The GIFs provide browser-independent playback.

Earlier artifacts in `engine/` preserve the first run before explicit shared
anchor validation was added; use `engine-checked/` and `engine-convergence/`
for the complete checked results.
