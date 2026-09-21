# Mechanism route-only corpus

Date: 2026-09-03.

This experiment moves the competitive harness beyond prefixes of one ESP32
board. The first twelve independent semantic inputs are regenerated from zero
copper and held at their declared placement. A thirteenth case regenerates a
constrained autonomous placement. Both routers receive the same resulting cold
board under matching rules and are admitted by native KiCad
ERC/DRC/parity/connectivity checks.

Run it with:

```sh
target/release/pcb-maker benchmark-route-corpus \
  benchmarks/competitive/mechanisms/corpus.json \
  build/sequence-218-m0-mechanism-twelve-timeout-aware
```

## Retained diagnosis

Sequence 212 exposed two template defects before pcb-maker ran: compact
schematic symbols reused coordinates and unrelated labels became electrically
joined; long generated captions crossed the board edge on silkscreen.
Sequence 213 fixed both and exact-completed the initial crossing and shared-tree
cases.

Sequence 214 added four imported layout-trace mechanisms. It retained three
failures rather than disguising them as router results:

- legacy pins with a point but no pad geometry became padless KiCad symbols in
  `decoupling` and `squeeze`;
- a board-only keepout reference crossed the board edge in
  `placement-perturbation`;
- the paired runner incorrectly treated Freerouting completion as a prerequisite
  for running pcb-maker, which would censor pcb-maker-only results.

The adapter now gives legacy point terminals an explicit, manifest-controlled
1.5 mm circular F.Cu pad, puts board-only reference text on fabrication
documentation, and always runs both competitors when Freerouting produced the
source/result evidence needed for comparison. Sequence 215 is the corrected
six-case run.

## Sequence 215 result

All six cases are infrastructure-finished and both routers are native-exact on
all six. The run records 8 valid warm cache hits, 16 cold misses, no bypasses,
48 sequence-prefixed front/back images, and 177.129 seconds of aggregate paired
work.

| Case | Rules (width/clearance, mm) | Freerouting segments / vias / physical mm | pcb-maker segments / vias / physical mm | Layer observation |
| --- | --- | ---: | ---: | --- |
| Forced crossing | 0.25 / 0.20 | 7 / 0 / 94.929 | 7 / 2 / 81.276 | Freerouting detours on F.Cu; pcb-maker takes a direct two-via B.Cu excursion. |
| Multi-terminal tree | 0.25 / 0.20 | 4 / 0 / 54.870 | 5 / 0 / 48.199 | Both use F.Cu; pcb-maker's shared tree is shorter. |
| Decoupling | 1.00 / 0.60 | 10 / 0 / 120.858 | 9 / 0 / 115.634 | Both use F.Cu around two mechanical holes. |
| Movable blocker | 0.80 / 0.40 | 3 / 0 / 36.960 | 4 / 0 / 37.182 | Both route around the declared keepout on F.Cu. |
| Placement perturbation | 0.80 / 0.40 | 3 / 0 / 36.943 | 4 / 0 / 37.182 | Both find the same available side of the fixed declared wall arrangement. |
| Squeeze | 1.00 / 0.60 | 6 / 0 / 93.652 | 4 / 0 / 92.485 | Both keep two wide routes separated on F.Cu. |

Completion is tied, so these small copper differences may describe strategy
but do not establish product superiority. In particular, the forced-crossing
geometry does not force a layer transition: the open board permits a long
single-layer detour.

## Scope boundary

This is route-only evidence. Movable/free-rotation metadata survives the
semantic conversion, but the comparison deliberately fixes the declared pose;
it does not test autonomous placement or coupled movement. The `squeeze`
fixture's unpinned movable body was not declared as a routing-body keepout in
the imported source, so its native form is a mechanical NPTH/courtyard obstacle,
not yet a rectangular-body squeeze acceptance case. Those distinctions must be
covered by later placement and exact-geometry mechanism entries rather than
inferred from these images.

Authoritative aggregate:
`build/sequence-215-m0-mechanism-legacy-pad-fix/competitive-corpus.json`.

## Twelve-case expansion and timeout semantics

Sequence 216 adds six ESP32-derived mechanism slices: two pad-escape/fanout
cases, power/capacitor interaction, a resistor turn, a ten-net resistor-link
bundle, and an eight-net header breakout. Ten cases both-solve. The two larger
cases hit pcb-maker's unchanged 60-second end-to-end limit after committing
exact rung 5, while Freerouting solves them. The old report called those
infrastructure failures because a killed child had not yet written its final
`cold-prefix.json`.

Sequence 217 is a deliberate five-second timeout control. The parent now reads
the crash-safe progression checkpoint, verifies and renders its selected exact
board, records `timed_out=true`, and classifies the result as Freerouting-only.
It refuses this classification if the checkpoint, exact verification, or
matched placement evidence is missing.

Sequence 218 reruns all twelve at the normal 60-second limit. Both routers solve
all twelve. It records 64 cache hits, 16 misses, no bypasses, 96 unique images,
and 287.855 aggregate seconds. The ten-net bundle finishes only narrowly:
pcb-maker takes 59.949 seconds after 11 hits and 10 misses. This is warmed-host
evidence, not a cold-runtime parity claim.

| Added case | Freerouting segments / vias / physical mm | pcb-maker segments / vias / physical mm | Observation |
| --- | ---: | ---: | --- |
| ESP pad fanout | 6 / 0 / 75.517 | 8 / 0 / 75.774 | Both complete on F.Cu. |
| ESP multi-contact escape | 3 / 0 / 36.959 | 4 / 0 / 37.327 | Both complete on F.Cu. |
| Power/capacitor | 10 / 0 / 17.953 | 18 / 4 / 18.754 | pcb-maker spends two layer excursions where Freerouting needs none. |
| Resistor turn | 6 / 0 / 68.440 | 10 / 0 / 69.060 | Both complete on F.Cu. |
| Resistor-link bundle | 60 / 2 / 361.198 | 49 / 6 / 331.173 | pcb-maker is shorter but uses four more vias; completion remains tied. |
| South-header breakout | 33 / 0 / 146.498 | 38 / 8 / 127.353 | pcb-maker is shorter but uses B.Cu/vias unnecessarily relative to the baseline. |

The historical pre-rule-area-fix aggregate is
`build/sequence-218-m0-mechanism-twelve-timeout-aware/competitive-corpus.json`.
The 12 cases are still short of M0's 20-mechanism gate. The imported
`west-six-final-geometry` case is not admitted yet because it contains mixed
per-net widths and the competitive rule schema currently permits only one
global width; flattening it would violate the source rules.

## Autonomous-placement acceptance

Sequences 253--255 show why movable metadata alone is not an autonomous-layout
test. `movable-blocker` and `squeeze` contain movable mechanical bodies with no
electrical attachment. The resistor-rotation fixture does connect its resistor,
but the complete connected block has no two distinct fixed positional anchors.
The harmonic solver correctly retains all three declared layouts. These are
retained in `autoplace-negative-controls.json`; they do not count toward M0.

Sequence 256 adds `anchored-resistor-autoplace`. Two fixed endpoints connect a
free rectangular two-pad resistor initially declared at `(20, 15)` and 90
degrees. Harmonic placement moves it to `(20, 10)` and zero degrees, reducing
straight connectivity distance from 12.790966 to 10.000000 mm. Freerouting and
pcb-maker independently route that same regenerated zero-copper placement and
both pass native KiCad; this is the thirteenth admitted mechanism.

Sequence 257 makes that distinction executable rather than documentary. A
semantic route manifest may require minimum moved, translated, and rotated
component counts plus minimum connectivity-distance reduction. The route fails
before external routing if generated placement misses any requirement. The
replay records 1/1/1 components and 2.790966 mm reduction, then both-solves. Its
paired renders are visually consistent with the horizontal legalized resistor.
The main corpus then declares thirteen valid mechanisms. Sequence 218 remains
the last twelve-case historical aggregate, but it predates the rule-area
preservation correction described below and is no longer current acceptance
evidence for cases containing semantic keepouts.

## Mandatory layer transition

Sequence 258 connects one F.Cu-only SMD terminal to one B.Cu-only SMD terminal.
Native completion therefore requires a layer transition; both routers produce
one via, use both copper layers, and finish with zero opens or DRC findings.
The paired front/back renders make each half of the connection visible.

Sequence 259 adds executable result obligations to the manifest: each routed
board must contain at least one via and use at least two copper layers. The
Freerouting benchmark enforces its result before declaring completion, and the
paired comparison independently enforces pcb-maker's result. Both pass. This
is the fourteenth admitted mechanism; sequence 258 remains the useful discovery
run and sequence 259 is the acceptance replay.

## Fixed keepout bottleneck and adapter audit

Sequences 260--270 turn a proposed 3 mm passage into an adapter audit. Sequence
260 first fails because its artificial mechanical hole is below KiCad's minimum
drill. After correcting that fixture, sequence 261 appears to pass, but its cold
source reports zero rule areas and the images show straight routes: progressive
materialization had silently discarded every netless keepout zone. This result
is retained but not counted.

The materializer now preserves rule areas while still pruning copper zones by
selected net. Component-body areas allow pads (so a body's own NPTH is legal),
whereas explicit routing keepouts continue to forbid them. The KiCad grid
adapter now lowers track-blocking rule-area polygons as fixed, layer-aware
obstacles instead of treating all zones as yielding copper. Regression tests
cover all three distinctions. Sequence 266 revalidates `movable-blocker`: the
cold board has one rule area and both routers visibly detour around its body.

Sequence 267 then exposes a separate unsupported conversion: abstract per-net
`allowed_layers` is not lowered into KiCad net rules, so pcb-maker escapes a
top-only wall on B.Cu. The final bottleneck puts the two wall halves on both
layers instead. At 0.25 mm resolution sequence 269 exhausts 5,362 reachability
cells and rolls back while Freerouting completes. At 0.125 mm, sequence 270
both-solves: Freerouting uses 22.787006 mm/three segments and pcb-maker uses
22.762742 mm/four segments, both without vias or native findings. This proves a
raster-resolution loss, not an A* budget shortage. The fine comparison is the
fifteenth corpus mechanism; coarse failure remains its diagnostic control.

Sequence 271 closes the corresponding inspection gap. KiCad's native 3D
renderer does not display rule areas because they are constraints rather than
physical board layers. The journal renderer now creates a disposable board
with outline-only F.SilkS/B.SilkS polygons for each applicable rule area,
renders it, and deletes it. The verified source/result boards are not mutated.
The retained front/back pair visibly shows both wall halves and the route
through their opening.

## Ground-plane stitching

Sequences 272--275 integrate the inherited rectangular ground-zone transaction
as an explicit competitive post-route processor rather than allowing copper in
the cold input. A four-terminal GND net has two F.Cu-only and two B.Cu-only
pads. Each router first solves that identical zero-copper board. Its routed GND
tracks are then removed, two inset F.Cu/B.Cu zones are added, existing GND vias
are retained as stitching candidates, and KiCad refills and exact-gates the
result.

Sequence 272 retains a harness failure: Freerouting's plane result passes, but
pcb-maker's admission cannot reuse the already-created cold-source baseline.
Sequence 273 fixes that lifecycle by checking the baseline board hash before
reusing its reports. Both results then have zero segments, two copper zones,
zero opens, and zero native findings. Freerouting retains one stitch via after
removing four segments; pcb-maker retains two after removing nine segments.
Executable acceptance requires at least two zones, two used layers, and one
via independently from both tools. This is the sixteenth admitted mechanism.

Sequence 273 initially proves refill-time connectivity but exposes that the CLI
DRC path does not serialize filled polygons back into the PCB. Sequence 276
therefore runs pcbnew's zone filler and saves the generated result before the
independent DRC refill. Both outputs now contain two filled polygons and measure
2,049.837046 mm2 of layer-weighted filled-zone area. Sequence 277 makes at least
1,800 mm2 of persisted area an executable result obligation in addition to two
zones, two layers, and one via; both tools pass. Sequences 274--275 and 278 add
an opt-in render-only solid outline and readable `/GND` label on both sides.

## Differential-pair result acceptance

Sequence 279 adds a deliberately easy positive control. `USB_P` and `USB_N`
are symmetric, unobstructed, and independently routed from the same zero-copper
board. Both routers produce one 26.000000 mm segment per net with no vias. The
benchmark imports each named final route, measures physical length and via
count, and requires at most 0.500000 mm length skew and zero via-count
difference. Both pass. This validates the measurement and acceptance contract;
it does not show pair-aware corridor routing, controlled pair spacing, or
length tuning.

Sequences 280--282 retain the corresponding negative experiment. The first
fixture accidentally put its dummy NPTH inside the explicit keepout and is not
valid evidence. After separating body and keepout, both routers electrically
complete the asymmetric board with zero native findings. The obstacle forces
only `USB_P` to detour: Freerouting measures 27.547585 versus 26.000000 mm
(1.547585 mm skew), while pcb-maker measures 27.839150 versus 26.000000 mm
(1.839150 mm skew). Both therefore fail the same 0.500000 mm product-level
contract despite native electrical completion. Sequence 282 also makes the
top-level pcb-maker completion flag consume result acceptance symmetrically
with Freerouting; the negative now reports neither solved and a tie.

The straight positive is the seventeenth admitted mechanism. The asymmetric
case remains a negative control identifying the next missing capability:
coupled pair routing or bounded post-route length matching. Pair spacing and
maximum uncoupled length are not yet represented and must not be inferred from
this acceptance result.

## Preserved bodies, pad ports, and rotation audit

Sequence 283 is the first full 17-case replay after rule-area preservation. It
finishes as infrastructure but only seven cases both-solve; ten legacy cases
become neither-solved because their own monolithic component-body keepout covers
the only path out of each pad. The old sequence-218 all-solved result depended
on those bodies being silently discarded and is not current acceptance evidence.

The native adapter now decomposes a body keepout into cells around explicit pad
ports rather than deleting the body. Ports are derived per connection from pad
geometry, trace width, and clearance, and are emitted only on the pad's copper
layer. Pinless mechanical bodies remain closed. Sequences 284--287 expose a
second issue: a port can align with a pad while both are mirrored relative to
the semantic problem. A 270-degree ESP pad should coincide with the declared
seed endpoint `(22.71, 30.75)`, but the initial KiCad render put it on the
edge-facing side.

Footprint angles are now serialized with the opposite sign required by KiCad's
board convention, while directly emitted rule-area points retain semantic
rotation. The generated-board regression test checks the actual ESP fixture's
pad against its semantic endpoint. Sequence 289 visibly moves the ESP pads to
the interior-facing side and both-solves. On corrected 65-degree decoupling,
sequence 290 becomes pcb-maker-only: ours completes both nets at 111.851425 mm,
10 segments, and no vias; Freerouting leaves one net open. Sequence 292 repeats
that result after top-only ports stop opening B.Cu.

Sequence 291 is the corrected-orientation aggregate before that final
layer-specific tightening. All 17 child reports finish: 13 both-solved, one
pcb-maker-only, two Freerouting-only, and one neither-solved. The two pcb-maker
losses are 60-second process timeouts on the ten-net resistor bundle and
eight-net south-header breakout; each retains an exact-clean rung-3 checkpoint.
The neither-solved placement-perturbation case is intentional under fixed
placement: its three body walls seal the channel, making it the next concrete
placer/router coupling acceptance case. A final aggregate replay under the
layer-specific port representation remains required.

## Router-to-placer passage feedback

Sequences 293--298 turn the fixed-wall negative into a separate positive
coupling mechanism without removing the original control. The declared
placement cannot route. Passage-capacity feedback attributes the failed front
to `WALL`, moves it 0.8 mm within its declared vertical region, and completes
the semantic route in one repair attempt. That route is discarded. Both
competitors then receive the same generated zero-copper KiCad board; placement
acceptance requires at least one translated component and the 0.0001 mm digest
gate proves identical fixed poses.

The first fair frozen-source run is sequence 296: Freerouting completes, while
pcb-maker rejects the 1.8 mm passage in reachability before A*. Sequence 297
shows that changing from 0.5 to 0.25 mm does not help. The cause is the old
half-cell-diagonal obstacle inflation: at 0.25 mm it requires 1.953553 mm for a
physically valid 1.600000 mm trace envelope. The router now uses exact point
clearance and checks every planar grid edge against continuous obstacle
geometry. A regression test reproduces the narrow rule-area channel.

Sequence 298 both-solves and native-passes with matching placement, no vias,
and zero findings. pcb-maker uses 37.431241 mm versus Freerouting's 38.566705
mm. This is the eighteenth admitted mechanism and the first positive corpus
case where routing failure changes placement. It proves one constrained
translation interaction, not general auto-placement, rotation feedback, or
multi-net coupled search. A full 18-case aggregate replay remains pending.
