# Competitive benchmarks

These manifests run external tools against the same cold inputs as pcb-maker.
They are executable evidence, not screenshots or vendor claims.

The [ESP32 pad-gap profiles](../esp32-pad-gaps/README.md) compare the existing
0.25/0.20 mm routing policy with a 0.15/0.10 mm policy that can pass between
adjacent pads. They include focused and full-placement cases, matched grids,
native rule settings, automatic renders, and passage/work measurements.

The route-only contract regenerates semantic connectivity and placement when
configured, materializes zero copper, applies the manifest's exact routing
rules, invokes the external router, imports its result, and admits it only
through native KiCad verification. Every run writes source/result front and
back images.

For pinned upstream projects, `cold_kicad_project` copies the complete project
and removes inherited routed copper while retaining rule-area keepouts. Native
route-only admission records both the absolute result and a frozen-source
baseline: result connectivity must be zero, and any ERC/DRC-design/parity finding
must be absent or identical to one already present in the source. This keeps
unrelated upstream warnings visible without attributing them to a router.
As in the base native verifier, `lib_footprint_mismatch` findings with severity
`warning` are counted separately as library metadata. Their changed descriptions
or affected items do not reject routing admission. All other DRC warnings and
all mismatch errors remain design findings. Raw reports and metadata counts
are retained for inspection.

Freerouting is not redistributed. Download the exact jar named by the manifest
and verify its SHA-256 before running. For the current prefix-19 control it is
expected at `/tmp/freerouting-2.2.4.jar` with SHA-256
`f5ed374182900ccc78e473518bbb9f6b869f4a07159495f663a76f52bb10523b`.

```sh
cargo run --release -- benchmark-route-freerouting \
  benchmarks/competitive/dual-esp32-prefix19/freerouting.json \
  build/sequence-196-m0-freerouting-harness-prefix19
```

The output directory must not already exist. This prevents a prior route from
silently becoming the next run's starting point. Its basename also becomes the
prefix of four mandatory source/result front/back images in `build/progress/`,
so use a new sequential experiment name for every run.

The paired runner invokes both pcb-maker and Freerouting. Semantic manifests
regenerate independently from pruned connectivity; direct real-board
manifests give both routers the same adapted, hashed zero-copper project:

```sh
cargo run --release -- benchmark-route-compare \
  benchmarks/competitive/dual-esp32-prefix03/comparison.json \
  build/sequence-201-m0-paired-prefix03
```

```sh
cargo run --release -- benchmark-route-compare \
  benchmarks/real/ecc83-pp/comparison.json \
  build/sequence-224-m0-ecc83-formal-comparison
```

The selected pcb-maker insertion or direct-sequential config is rejected
before execution unless every routing entry matches the manifest's width,
clearance, via diameter, and drill.
The report also requires the cold source and both routed boards to share a
component-placement digest at the 0.0001 mm Specctra exchange precision. Exact
unrounded pose hashes, native reports, process logs, and separate front/back
renders remain available for diagnosis.

The paired manifest also names a native-verification cache directory. Its key
covers local KiCad design inputs, the exact `kicad-cli` version, verification
policy and zone-refill mode, and a verifier digest derived from `pcb-kicad`
source plus `Cargo.lock`. Reports are restored only after their hashes and
metadata validate. External library URIs bypass caching, symlinked design
inputs are rejected, and malformed entries fail the run. Every invocation
records hit/miss/bypass telemetry in `competitive-comparison.json` and retains
the raw JSONL events beside the process log.

The corpus runner executes a versioned list of paired comparisons and writes a
checkpointed aggregate after every case:

```sh
cargo run --release -- benchmark-route-corpus \
  benchmarks/competitive/dual-esp32-growth-smoke/corpus.json \
  build/sequence-210-m0-dual-esp32-growth-smoke-corpus
```

The independent mechanism corpus is runnable with the same command:

```sh
cargo run --release -- benchmark-route-corpus \
  benchmarks/competitive/mechanisms/corpus.json \
  build/sequence-215-m0-mechanism-legacy-pad-fix
```

It currently declares sixteen route-only cases and two constrained-placement
cases. Autonomous and feedback-driven cases carry explicit
minimum movement/rotation/connectivity-improvement obligations; equal output
placement between competitors is not sufficient. Imported legacy point-only
terminals use the template manifest's explicit
`implicit_pad_diameter_mm`; this is conversion policy, not inferred source
geometry. An incomplete router result does not suppress the other competitor:
both/one/neither-solved are measured outcomes, while missing comparison
artifacts or mismatched placement/rules remain harness failures.
Topology mechanisms may likewise require minimum via, used-copper-layer, and
copper-zone counts and minimum serialized filled-zone area independently for
both routed results. A post-route
processor can replace a named routed connection with refilled front/back zones
while preserving the zero-copper source contract. Native completion remains a
separate mandatory condition. Differential-pair acceptance can name two final
routes and bound their physical length skew and via-count difference. This does
not yet constrain pair spacing or uncoupled length. The current corpus has
eighteen mechanisms.

The latest corrected-orientation aggregate is
`build/sequence-291-m0-mechanism-seventeen-corrected-rotation`. It finishes all
17 comparisons with 13 both-solved, one pcb-maker-only, two Freerouting-only,
and one neither-solved fixed-placement case. Sequence 292 subsequently makes
component-body pad ports per-net and layer-specific while preserving the
pcb-maker-only decoupling result; the full corpus still needs one replay under
that stricter final representation.

Sequences 293--298 add the first accepted tracer-to-placer mechanism. A failed
semantic route moves a vertically constrained wall, its route is discarded,
and both competitors consume the same moved zero-copper KiCad board. The first
fair run exposed half-cell-diagonal obstacle inflation in pcb-maker's native
grid adapter. Exact continuous checks on every grid edge restore the legal
1.8 mm passage; both routers native-complete with matching placement and no
findings. The expanded 18-case aggregate has not yet been run.

`competitive-corpus.json` records infrastructure/fairness status separately
from the four completion outcomes (both, Freerouting only, pcb-maker only, or
neither), then summarizes comparable segment/via/stored-length measurements
and cache telemetry. Child reports remain authoritative. Corpus image names
include both the sequence directory and case ID, so later runs do not overwrite
the scrollable experiment history.

Every board-statistics record contains the same `physical_copper` schema.
Straight same-net tracks are contact-split and overlap-deduplicated before
centerline length is counted. Circular arcs, width-weighted track area,
projected via/drill and layer-weighted annulus areas, refilled zone polygons,
rectangular board area, used-layer count, and per-layer totals are reported.
The schema explicitly marks whether centerline union is exact and records that
area is a primitive sum rather than a planar copper union. Completion remains
the primary corpus outcome.
