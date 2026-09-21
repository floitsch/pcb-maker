# Legalization traces distinguish slow progress from a stuck placement

The PIC failure at C2/U5 was not evidence of an impossible placement or a local
minimum. Capturing every sweep exposed a slowly shrinking network of contacts.
The unchanged solver reaches a legal placement at sweep **2,039** when given an
explicit larger work budget. Native KiCad translation and inventory checks pass
for all 63 footprints, including JP1's custom back-side pads. Six annotation
findings remain, with zero physical/ERC/parity findings and 125 cold open items.
The first two routing passes stop after four nets at a grid-access failure.
A finer-grid probe connects that blocked net; routing has resumed from the
verified four-net prefix. There is no completed-board or area credit.

[Playback and evidence](../../build/placement-trace-2026-09-08/index.html).

## Instrumentation

`run_initial_placement_traced` exposes an observer for the harmonic seed and
completed legalization sweeps, including failed proposals. Each frame contains
every rigid pose, the pair-check count, the maximum correction encountered while
processing that sweep, and body/pad contacts recomputed at the saved positions.
The saved-state contacts exclude the optional connected-pair spacing floor and
relational rows. The observer performs no file I/O in the placement library and
does not change the proposals, budgets, admission checks or default solver.

`pcb-maker trace-placement` retains the trace before returning a placement error.
`trace_placement.py` automatically generates an interactive playback, contact
table, residual plot, and initial/final SVGs. The general native-project driver
exposes the same output with `--trace-placement`. Component labels and both
assembly sides/pad layers share one view. These are geometry-model diagnostics;
native verification remains a separate gate.

## Experiments

| PIC run | Legalization sweeps | Largest final pair correction | Model placement |
| --- | ---: | ---: | --- |
| Original budget | 512 | 0.063261 mm | Rejected |
| Larger budget | 2,039 of 3,072 | No remaining contacts | Accepted; native geometry/inventory pass |
| One motion extrapolation | 256 + 256 | 0.004639 mm | Rejected |
| Two motion extrapolations | 256 + 128 + 128 | 0.000275 mm | Rejected |

The original run begins with 74 overlapping pairs. After 256 sweeps the maximum
correction is 0.398539 mm. The final 64 frames retain the same 21 pairs while
their residuals shrink. The first reported error, C2/U5, is only one of those
pairs; J1/P3 has the largest remaining correction. Repeated pair corrections
appear to propagate movement slowly through a component chain. That mechanism
is an inference from the contact and movement history, not a proof about every
placement failure.

The extrapolation probes use only the saved prefix, never the successful final
poses. At a checkpoint, two consecutive 32-sweep displacement vectors estimate
the contraction ratio `r = dot(previous, recent) / dot(previous, previous)`.
The proposed position adds `recent * r / (1-r)`, then ordinary legalization
resumes. Predictions at original sweep 256 and resumed sweep 128 reduce the
residual substantially but still fail the unchanged gate. These remain
experimental seed proposals; they are not integrated into production recovery,
and the table compares legalization sweep counts, not equal total work or time.

## Controls and next work

- All 42 placement tests pass, including traced/untraced equality for both a
  successful movable case and an impossible fixed overlap. Failed final poses
  and contacts are retained.
- The successful 68-component `complex_hierarchy` control reproduces its entire
  prior result exactly with tracing enabled (232 frames).
- Native PIC materialization independently audits geometry, identities, pad
  shapes/layers, poses, rules and source preservation under the explicit
  all-free reference-overhang policy. Original product mechanical constraints
  are not reconstructed.
- Headless Chrome checks seeking to sweep 512 and advancing playback without
  page errors. The retained screenshot was inspected. Browser file navigation
  timed out; the successful check loaded the same HTML directly.
- The separate 110-net benchmark's first pass is now terminal: 55 nets, 143
  opens, no native design findings, four committed repairs. Its independent
  sequence audit passes. The second pass remains live.

## Routing exposes terminal-grid sensitivity

Both PIC passes retain four routed nets and 69 native opens, with the same six
annotations. Independent selection and sequence audits pass under the explicit
unchanged-annotation contract. The sequential auditor previously inferred that
contract only from rip-up settings, which are absent here; it now has an explicit
`--allow-annotations` option. The default still rejects these annotations, and
controls reject a changed annotation or a physical finding even with the option.

The next net, `pic_sockets/VCC_PIC`, fails before search: its back-side terminal
snaps to an occupied 0.5 mm grid cell. The independent cold-board forecast has
the same error, so the four previously routed nets did not cause it. A 0.25 mm
probe on the retained board connects the net and reduces opens from 69 to 59,
with no new native findings. A 0.125 mm probe fails terminal access again.
Thus finer resolution alone is not a monotonic fix; the chosen grid sample
matters. This points toward better pad-contact sampling as a general input
robustness target, without proving every such failure has the same cause.

The ongoing resumed run keeps the four verified nets, tries 0.5 mm first, and
falls back to 0.25 mm only when needed. It uses existing production checkpoint
and ordered-portfolio support. The standalone probe is not credited as an
automatic coordinator commit; the resumed run must reproduce and audit it.

Next, evaluate the resumed full routing result of the new PIC placement. If placement
search needs faster recovery, use contact and convergence evidence to choose
between continuing, extrapolating with rollback, solving a coupled contact
group, and changing the seed. Do not interpret the old sweep-limit error as a
placement infeasibility proof or replace the default with an unbounded loop.
