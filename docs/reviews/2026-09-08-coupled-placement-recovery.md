# Solving contacting placement groups together

PIC placement slows down because sequential pair corrections propagate movement
through crowded groups. The implementation does not increase mass or decrease
a time step: `apply_separation` divides the current overlap by the squared
movable gradients, then immediately clips each moved component to its bounds.
Later pairs can undo part of earlier corrections. A shrinking local residual
therefore does not imply that a component has reached a useful global position.

The new optional coupled recovery reaches a legal PIC placement after **128
ordinary sweeps and three joint rounds**, compared with **2,039 ordinary sweeps**.
Native geometry and inventory audits pass for all 63 footprints. Six annotation
findings and 125 cold opens remain before routing. The new placement's first
full routing pass now connects all 34 nets with zero opens and no new native
findings. Independent sequence and adaptive-selection audits pass, including
connectivity, preserved unrelated copper/poses and requested native dimensions.

[Animations, controls and measurements](../../build/coupled-placement-2026-09-08/index.html).

## Mechanism and safeguards

`coupled_legalization` in a harmonic policy enables one bounded recovery attempt.
After the configured ordinary sweep, it forms separation inequalities from the
observed body/pad contacts and solves for a minimum-norm joint translation.
Fixed components and locked axes have no free variables. Board, region, body
overhang and pad-edge limits use the existing production geometry calculation.
The optional connected-pair spacing floor remains enforced when selected.

The solver uses a bounded dual active set with small dense active systems and
sparse constraint rows. It regenerates contact geometry after each proposal,
adds newly created contacts, and only accepts after the geometry and represented
hard constraints pass. Singular or conflicting contact branches, exhausted work
limits and unsuccessful proposals restore the original poses. They are not
reported as proofs of placement infeasibility. Ordinary legalization can then
continue within its remaining budget. Relational constraints are checked before
acceptance; this is not a global solver for choosing relational or nonconvex
separation branches.

Default limits are a start after 128 ordinary sweeps, 16 joint rounds, 4,096
linear solves, 2,048 constraint rows and 256 active rows. Coupled geometry checks
consume the shared pair-check budget. Tracing and final admission checks are
additional diagnostic/verification work. The library performs no file I/O.

Early Python probes at sweeps 0, 16, 32 and 64 encounter singular or conflicting
branches; the sweep-128 probe succeeds in three rounds. These probes use only
their prefix positions and contacts, not the eventual successful placement.
The compiled Rust implementation reproduces the three-round result and keeps
every pose/frame in the first 128 ordinary sweeps unchanged.

## Measurements and validation

| Same PIC input and executable | Ordinary sweeps | Coupled rounds | Recorded routing-independent placement time |
| --- | ---: | ---: | ---: |
| Pairwise control | 2,039 | 0 | 42.975 s |
| Coupled recovery | 128 | 3 | 2.876 s |

Both timings include tracing, use the same geometry and run sequentially on the
same machine. This is one sample per mode, not a general runtime guarantee.
Both stop after placement succeeds; their configured upper budgets differ but
are not exhausted. Native materialization and routing are outside the timed
command. The coupled run performs 92 linear solves and 257,796 charged pair
checks versus 3,982,167 pair checks in the pairwise control.

- All 47 placement tests pass. Added controls cover a long contacting chain,
  removal of negative dual multipliers, fixed/axis-limited components, new
  contacts, board limits, singular systems and exhausted work limits.
- A real PIC rollback control deliberately permits only one early joint round.
  It restores the original positions; the next ordinary sweep reproduces the
  archived sweep-17 poses exactly.
- The 68-component `complex_hierarchy` control reaches legal geometry after
  sweep 128 plus two joint rounds, versus sweep 231 in its pairwise control.
  Disabling coupled recovery reproduces the entire earlier trace/result exactly.
  The changed coupled placement has not yet received a native/routing claim.
- PIC's coupled placement passes independent geometry and native translation/
  inventory audits. Original component sides, pad geometry, classes and outline
  are preserved under the explicit all-free reference-overhang policy; original
  product mechanical requirements are not reconstructed.
- Headless Chrome checks the accepted third joint round, zero displayed
  contacts and advancing playback without page errors. The retained screenshot
  was inspected.

The earlier, slower PIC placement now has an independently audited **34-net,
zero-open** routed result using resumed coarse/fine routing. Its six annotations
still prevent full-layout admission. This result is distinct from the new
coupled placement. The new placement's own first-pass result also connects all
34 nets, using the ordered 0.25 mm fallback only for `pic_sockets/VCC_PIC`.
Its six existing annotations still prevent full-layout and board-area credit.
The native-project driver now enables coupled recovery by default and offers
`--pairwise-legalization` for comparisons. Direct policy JSON remains opt-in.

The separate 110-net run is also terminal: passes route 55 and 52 nets, retaining
the first pass with 143 opens and no native design findings. Both sequence
audits and the adaptive selection audit pass. Both runs exhausted the configured
rip-up allowance before their final failed target. Broader recovery remains
necessary; this is not a completed board or an equal-rule external-router score.
