# Harmonic placement locality regression

Run from the repository root after `cargo build --release`:

```
python3 experiments/placement-locality/run.py build/placement-locality-new --native
```

The output directory must be fresh. The script freezes and hashes the executable,
places the complete dual-ESP32 benchmark, and automatically produces SVG/PNG
renderings highlighting `D_LED`, `R_LED`, `J_POWER` and their allowed regions.
`--native` then preserves all 43 proposed poses and routes only `LED_SERIES` at
its declared 0.35 mm width and 0.20 mm clearance. Verification renders every
native board; the audit checks exact semantic pose preservation, native pose
hashes, selected-net completion, and ERC/design DRC/parity counts.

The retained experiment is `build/placement-locality-2026-09-07`. Its before,
bounds-only, and bounds-plus-weights proposals use the same full problem and
placement configuration. `compare.py DIRECTORY` regenerates that experiment's
three-way viewer and locality assertions without rerunning placement. This
comparison utility expects the retained artifact layout; `run.py` accepts any
fresh output directory and an optional `--executable` control binary.

The bounds-only binary was built before adding attraction weighting, rather
than introducing production switches for either bug. Final production behavior:

- Project board/region bounds and locked axes during harmonic initialization
  and every relaxation step, so neighbors see constrained positions.
- Use `width * tension_weight` for harmonic and port attraction, for both legacy
  branches and explicit electrical nets. A zero weight adds no attraction edge.
- Physical routing demand and clearance remain independent of attraction.

The fixture already sets GND branches to `tension_weight: 0.05`. There is no
name-based inference of net function. Ground geometry, return paths, and hard
local circuit constraints are separate concerns.

Rust regressions cover bounded-neighbor propagation, locked axes, known weighted
equilibria, zero-weight connectivity blocks, port weights and unchanged physical
demand. Native results cover the LED connection only, not full-board routing.

## Orientation counterfactual diagnostic

```
python3 experiments/placement-locality/orientation_probe.py \
  build/placement-locality-2026-09-07/weighted/problem.json \
  build/placement-locality-2026-09-07/weighted/placement.json \
  build/orientation-probe-new
```

This ranks quarter-turn alternatives at fixed component centers, then validates
the highest-scoring twelve with production placement constraints and exclusions.
It freezes all centers and the trial orientations for that legality query, so
projection cannot disguise a bad rotation by moving another part. Every trial,
including rejections, gets SVG/PNG; the report decomposes the attraction score
into individual branch distances before and after. Candidates are independent,
not a sequence of accepted changes.

The current diagnostic deliberately accepts only legacy two-terminal branches
and pins with one geometric pad center (which can appear on several layers).
It rejects explicit multi-terminal nets and multiple geometric centers rather
than silently picking a pad or inventing branch topology. The score is width
times declared tension weight times squared pad distance; it is not routed
length, impedance, or electrical suitability. In particular, a weak GND weight
can favor a rotation that lengthens the GND connection. Those tradeoffs remain
visible in the per-branch evidence.

`run.py --poses RESULT --native` probes a previously validated pose checkpoint
through the same focused LED native test. It checks that the declared projector
preserves the exact supplied poses. Use only validated poses; this option is a
probe runner, not a generic proposal admission interface.

Planted controls are retained in `build/placement-orientation-controls-2026-09-07`:
a reversed resistor between two fixed terminals has one known best orientation;
its narrow region rejects both perpendicular alternatives. A zero-attraction
control has no beneficial rotations. All controls include automatic renderings.

## Dense routing of a prepared placement

```
python3 experiments/placement-exploration/dense_probe.py \
  build/locality-dense-new \
  --seed corrected=build/placement-locality-2026-09-07/weighted/native
```

The seed directory supplies the full placed problem and a declared placement
configuration. This reuses the standard blocked/open nineteen-connection
benchmark, hashes and freezes its executable, and audits native previews at
every accepted prefix. `--seed` may be repeated for other prepared placements.

Run the permanent planted control with:

```
python3 experiments/placement-locality/check_orientation.py build/orientation-controls-new
```

The fixture lives at `benchmarks/small/orientation-region-control.json`. The
runner adds its zero-attraction control and verifies automatic local viewers.
`orientation_details.py PROBLEM RESULT REPORT_DIRECTORY` can regenerate local
views from retained evidence without rerunning any experiment.

## Production orientation refinement

The production initial-placement config now accepts optional
`orientation_refinement: {maximum_sweeps, maximum_trials, minimum_improvement}`.
Defaults are 4, 512 and 1e-9. Omission disables refinement. See
`experiments/configs/harmonic-ports-orientation-refinement.json` for a complete
config usable directly with `pcb-maker place` or semantic ladder generation.

To reproduce a comparison with automatic final rendering and native LED checks:

```
python3 experiments/placement-locality/run.py build/refinement-control-new
python3 experiments/placement-locality/run.py build/refinement-enabled-new --refine-orientations --native
python3 experiments/placement-locality/refinement_playback.py build/refinement-control-new build/refinement-enabled-new
python3 experiments/placement-locality/audit_refinement.py build/refinement-control-new build/refinement-enabled-new
```

The playback reconstructs every accepted rotation, renders all accepted states
and up to twelve rejected alternatives, and checks exact final-pose replay.
The independent audit recomputes every legacy-branch trial score in Python.
The production scorer additionally supports explicit multi-terminal nets and
multiple physical pad locations, covered by Rust tests. Both scoring methods
are placement surrogates; native routing remains the admission evidence for
actual copper.
