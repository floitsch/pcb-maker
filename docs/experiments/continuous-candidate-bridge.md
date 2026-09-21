# Continuous-engine candidate bridge

## Question

Can continuous placement and trace correction produce the same durable,
independently validated candidate artifact as the discrete routers?

## Boundary

The engine now emits a representation-independent `ContinuousSolution`:

- semantic component IDs, centers, and rotations;
- semantic connection IDs, layers, widths, and ordered polyline points.

Particles, rigid-body handles, attachment records, and constraint buffers stay
inside `pcb-engine`. The `pcb-routing` continuous adapter owns the
layout-trace lowering and candidate materialization. It retains a clone of the
source problem so a solution cannot accidentally be materialized against a
different semantic input. The exact validator remains independent and is
always run after materialization.

## Reduced control

`benchmarks/small/continuous-resistor-coupling.json` contains two fixed
one-pin anchors, one freely translatable/rotatable resistor, and two trace
branches. The left seed has an overlong sampling interval while the right seed
does not. With the density field disabled, maximum-spacing projection pulls
the resistor left through its terminal attachment.

Run:

```sh
cargo run -- relax-continuous \
  benchmarks/small/continuous-resistor-coupling.json \
  run/continuous-relax.json run/continuous-relax.html preserve-seed
```

Measured deterministic CPU-reference result:

| Measure | Result |
| --- | ---: |
| Steps | 20 |
| Projection sweeps/step | 8 |
| Resistor initial x | 10.000000 mm |
| Resistor final x | 9.000045 mm |
| Coupled displacement | 0.999955 mm left |
| Final maximum residual | 0.000029 mm |
| Exact geometry findings | 0 |
| Exact electrical findings | 0 |

The persisted result includes initial/final semantic solutions, all 21 frames,
the candidate, and both exact assessments. The viewer makes the component and
trace correction visible. A separate negative control moves the analytic body
outside the board and proves materialization does not hide the exact
`component_outside_board` failure.

Both endpoint-particle and analytic-body representations pass the same coupled
motion/materialization/exact-validation test. The CLI currently chooses the
analytic control explicitly; the adapter API accepts either compile policy.

The `preserve-seed` policy is intentional in this recovery control: its
overlong seed is the injected defect. Normal semantic lowering uses
`subdivide`, which inserts enough trace particles that no initial segment
exceeds the configured maximum spacing. The five-terminal control below
originally exposed this distinction by exact-passing while retaining an 8.72
mm solver residual; subdivision reduces that residual to zero instead of
mistaking undersampling for useful component force.
Subdivision is work-bounded per connection and fails before allocation when
the configured particle ceiling would be exceeded.

## General body and multi-terminal control

`benchmarks/small/continuous-multipin-tree.json` adds a freely movable
rectangular component with three arbitrary local pin offsets and one
five-terminal electrical net. Deterministic terminal-MST lowering emits four
continuous branches and materializes one graph with five terminal nodes.

```sh
cargo run -- relax-continuous \
  benchmarks/small/continuous-multipin-tree.json \
  run/continuous-multipin.json run/continuous-multipin.html subdivide
```

The analytic rectangle owns one pose and three body-local attachments. After
20 steps the subdivided control has zero maximum constraint residual, four
materialized traces, one route graph, and zero geometry or electrical
findings.

## Supported subset and next work

The adapter supports legacy branches and deterministic terminal-MST lowering
for multi-terminal electrical nets. Analytic rectangular bodies accept any
number of body-local logical pins; the specialized two-particle representation
still requires centered x-axis terminals and matching body width. Routes are
currently single-layer branches without vias, and relational placement
constraints are still rejected before compilation.

These restrictions are explicit missing ports, not approximated behavior.
Selected same-layer family-pair and family/fixed-context copper-clearance
obligations now have a bounded continuous correction path; its evaluation and
limitations are recorded in
[`continuous-family-obligation-repair.md`](continuous-family-obligation-repair.md).
The next extensions are component-body coupling, adaptive sampling, layer/via
bindings, and relational placement constraints.
