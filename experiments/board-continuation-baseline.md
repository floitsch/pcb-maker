# Oversized-board continuation experiments

The first continuation coordinator treats board shrinking as a sequence of
transactions. It scales the target outline, component declarations, movement
regions, and seed points about the board center. At each stage a replaceable
producer proposes a complete candidate, and exhaustive geometry/electrical
validation decides whether the smaller board becomes the new retained state.
The run stops at the first rejection, so the result always identifies the last
valid board rather than claiming progress beyond a broken stage.

The default schedule is `1.5, 1.4, 1.3, 1.2, 1.1, 1.0`.

## Clear control

`benchmarks/small/board-continuation-clear.json` has one trace routed around a
central keepout. All six stages exact-pass, including the original target at
scale 1.0.

## Bottleneck control

`benchmarks/small/board-continuation-bottleneck.json` lengthens the central
keepout so the target outline has no legal top or bottom passage. Stages 1.5,
1.4, 1.3, and 1.2 exact-pass. Scale 1.1 is rejected, the scale-1.2 candidate
remains selected, and validation of that candidate against the original target
correctly fails. The command exits unsuccessfully after writing all evidence.

This is the requested “where does shrinking get stuck?” evidence: the accepted
stage image shows the trace squeezed over the wall; the rejected stage shows a
red dashed unrouted intent. See `build/progress/004-*` and
`build/progress/005-*` for front/back PNGs at every stage.

```sh
cargo run -- continue-board \
  benchmarks/small/board-continuation-clear.json \
  run/continuation-clear.json \
  experiments/configs/board-continuation-default.json \
  build/progress/009-continuation-clear
```

## Retained-copper/local-repair control

`benchmarks/small/board-continuation-local-repair.json` adds a second, straight
`UPPER` route to the clear wall problem. At each shrink, affine deformation
makes `SIGNAL` violate wall clearance while `UPPER` remains exact-valid. The
`retained-affine-local-grid-repair-v1` producer removes and reroutes only
`SIGNAL`; `UPPER` becomes fixed context and spends zero new routing expansions.
All six stages and the target board exact-pass.

| Strategy | Searched branch-stages | Expansions | Final `SIGNAL` | Final `UPPER` |
| --- | ---: | ---: | --- | --- |
| Full reroute | 12 | 47,202 | 19.727922 mm, 64 segments, 2 bends | 16 mm, 64 segments |
| Retain + local repair | 7 | 46,679 | 19.727922 mm, 64 segments, 2 bends | 16 mm, 96 segments |
| Retain + repair + collinear compaction | 7 | 46,679 | 19.727922 mm, 64 segments, 2 bends | 16 mm, 1 segment |

The local producer searches 41.7% fewer branch-stages but saves only 1.1% of
total A* expansions because the expensive `SIGNAL` route still needs repair at
every shrink. The raw retained result also exposes a real penalty: it carries
the larger board's 0.25 mm sampling count into a smaller board, leaving 96
collinear segments instead of 64. The selectable compaction ablation removes
that representational debt without changing route length, bends, vias, search
work, or exactness. Its cumulative count of 488 removed points includes points
on invalid deformed `SIGNAL` candidates which were subsequently replaced; it
is work evidence, not the final segment delta.

The front/back sequences are:

- `build/progress/006-*`: full-reroute control.
- `build/progress/007-*`: raw retained/local-repair result.
- `build/progress/008-*`: retained/local repair with compaction.

The retained producer affinely maps component poses, junctions, copper, and
vias, then snaps terminal nodes to exact body-local pin positions. It computes
a deterministic branch hitting set from exhaustive geometry findings, requires
all non-selected copper to pass exact geometry, and uses selective grid reroute
for only that set. If a routed failure names fixed copper as a blocker, bounded
repair rounds can enlarge the set transactionally.

## Continuous component/trace correction before local reroute

The retained producer now has a replaceable motion-processor seam between
affine deformation and discrete grid repair. The first processor compiles
exact trace/trace and rectangular-body findings into the generic continuous
copper engine, runs a bounded CPU projection pass, materializes only selected
trace points and movable component poses, and commits only if the whole
candidate exact-passes. Unsupported or incomplete proposals retain the input
candidate and continue into selective grid repair.

`board-continuation-force-motion.json` is the component-motion control. Its
horizontal `SIGNAL` remains valid through scale 1.2. At scales 1.1 and 1.0 a
thin, vertically movable body enters clearance; it—not the trace—must yield.

| Strategy | Searched branch-stages | A* expansions | Local reroute stages | Exact motion stages | Final `SIGNAL` |
| --- | ---: | ---: | ---: | ---: | --- |
| Full reroute | 12 | 1,174 | 0 | 0 | 16.207107 mm, 64 segments |
| Retain + compact + grid repair | 4 | 418 | 2 | 0 | 16.207107 mm, 64 segments |
| Retain + compact + continuous motion | 2 | 194 | 0 | 2 | 16 mm, 1 segment |

The continuous result keeps the initial topology and avoids all later A*
search. At scale 1.1 it moves `YIELDING_BLOCKER` by 0.040999 mm; at scale 1.0
it moves the newly deformed pose by 0.063727 mm. The two corrections use ten
recorded frames, 1,280 constraint projections, and 1,792 scalar-row visits.
Those projection counts are additional CPU work, so the lower A* expansion
count is not by itself a runtime or GPU-speedup claim.

The complementary fixed-wall control uses
`board-continuation-local-repair.json`. The same processor bends only
`SIGNAL` around the body at all five shrink stages while endpoints, `UPPER`,
and the wall stay fixed. It searches only the two initial branches and spends
10,066 A* expansions, compared with seven branch-stages and 46,679 expansions
for compacted local grid repair. Its 25 frames perform 5,760 constraint
projections and 7,040 scalar-row visits. Maximum point motion grows from
0.246609 mm to 0.505655 mm as the board shrinks.

## Explicit keepout-shape lowering

Sequence 013 recorded the initial negative result for a rectangular explicit
routing keepout: every stage reported `unsupported` and fell back to grid
repair. The adapter now lowers a fixed owner's rectangular keepout as a fixed
virtual continuous obstacle while retaining its semantic identity as
`keepout:WALL:0`. It is not reported as a component body and cannot acquire an
independent pose.

`board-continuation-explicit-rect-motion.json` repeats exactly that rectangle
in sequence 014:

| Strategy | Searched branch-stages | A* expansions | Local reroutes | Exact motion stages | Final route |
| --- | ---: | ---: | ---: | ---: | --- |
| Pre-lowering/discrete fallback | 6 | 46,716 | 5 | 0 | 19.727922 mm, 64 segments |
| Fixed-rectangle continuous lowering | 1 | 9,969 | 0 | 5 | 20.684615 mm, 3 segments |

The lowering removes every later topology search and reduces recorded A*
expansions by 78.7%. Its 25 frames add 3,200 constraint projections and 4,480
scalar-row visits. It also exposes a 0.956693 mm (4.85%) length penalty: pure
clearance projection pushes the inherited vertices vertically but does not
pull their x coordinates from roughly 5/15 toward the expanded rectangle's
corners. This is evidence that the next processor needs an explicit
length/tension objective or exact vertex-pull stage; fewer searches alone do
not make this the better final route.

`board-continuation-explicit-keepout-fallback.json` now contains a circular
shape. The rectangular-body engine cannot represent it exactly, so sequence
015 records `unsupported`, keeps `keepout:WALL:0` visible, and reroutes only
`SIGNAL` at every shrink. All stages exact-pass. The circle is not replaced by
a conservative square because that would change the feasible geometry and
make the experiment's result ambiguous.

## Post-motion shortening and projected tension

Sequence 016 reuses the exact vertex-pull processor originally exercised after
discrete conflict actions. This validates the new consumer-neutral route
post-processing seam, but it is a negative algorithm result here: 485 extra
exact-validation calls save only 0.011052 mm on the final route (0.016639 mm
summed over all stages). A vertex moves only toward its neighbors' midpoint;
that direction enters the keepout almost immediately and cannot slide along
the active clearance boundary.

The continuous engine now has a flat trace-tension edge buffer. Each CPU step
accumulates equal and opposite length-gradient forces per edge, then the same
hard clearance/attachment projection removes forbidden motion. The retained
tangential component slides the two inherited bends toward the keepout
corners. This buffer maps to an explicit `AccumulateTraceTension` kernel in the
GPU dispatch plan; there is still no implemented GPU backend.

| Rectangle strategy | A* expansions | Frames | Constraint projections | Tension-edge integrations | Extra exact-validation calls | Final route |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| Discrete grid fallback | 46,716 | 0 | 0 | 0 | — | 19.727922 mm, 64 segments |
| Clearance projection only | 9,969 | 25 | 3,200 | 0 | 0 | 20.684615 mm, 3 segments |
| Clearance + exact vertex pull | 9,969 | 25 | 3,200 | 0 | 485 | 20.673562 mm, 3 segments |
| Projected tension, 8 steps/stage | 9,969 | 45 | 6,400 | 120 | 0 | 19.406395 mm, 3 segments |
| Projected tension, 16 steps/stage | 9,969 | 85 | 12,800 | 240 | 0 | 18.872391 mm, 3 segments |

The eight-step result is already 1.63% shorter than grid A* at half the full
tension projection work. Sixteen steps improve that to 4.34% shorter, with
final bend x coordinates 8.074680/11.969161 rather than the clearance-only
4.997408/15.002293. This is a real quality improvement, but not an optimality
or runtime claim: convergence has not been characterized, the work is CPU
reference work, and this control has only one trace and one rectangular
obstacle.

The new front/back sequences are:

- `build/progress/009-*`: full-reroute component-motion control.
- `build/progress/010-*`: retained/compacted grid-repair control.
- `build/progress/011-*`: the body yields under continuous correction.
- `build/progress/012-*`: the trace yields around a fixed wall.
- `build/progress/013-*`: pre-lowering rectangular keepout fallback.
- `build/progress/014-*`: the same rectangle handled by continuous motion.
- `build/progress/015-*`: unsupported circle with visible discrete fallback.
- `build/progress/016-*`: vertex pull after motion (negative shortening ablation).
- `build/progress/017-*`: sixteen-step projected tension.
- `build/progress/018-*`: eight-step projected-tension ablation.

This is positive evidence for continuous squeezing, not a general fluid or
GPU result. The processor is currently CPU-only and supports via-free,
single-layer selected traces, rectangular component bodies, and fixed
rectangular explicit keepouts. Circles, movable compound keepouts, explicit
pads, vias, mixed-layer runs, body/body collisions, junction endpoints, and
relational placement constraints remain fail-closed or fall back to discrete
repair.
