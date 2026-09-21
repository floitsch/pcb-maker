# GPU execution contract

GPU work is a first-order design constraint rather than a later port.

## Hot representation

Hot state uses flat structure-of-arrays buffers and integer handles. Particle
field weight is distinct from inverse mass and sample count. Constraint
families have separate dense buffers so one dispatch executes one uniform
kernel. Strings, hash maps, component classes, and electrical policy stay on
the cold side of the representation compiler.

The CPU reference now has separate particle, analytic-body, and body-local
attachment buffers. `ProjectAttachments` is an explicit GPU dispatch family;
it is a contract placeholder, not a GPU implementation. Body translation and
angle are updated through translational inverse mass and inverse inertia.

The planned pass graph is:

1. apply commands and discrete transaction results;
2. scatter particles and primitives into layer-local grid cells;
3. build density, capacity, and congestion fields;
4. run stencil/multigrid field passes;
5. gather field forces and integrate predicted positions;
6. build a spatial broad phase;
7. generate bounded contacts;
8. project each constraint family in colored/Jacobi batches;
9. update velocities, residuals, health, and compact viewer evidence.

Candidate batches add one offset table; the kernels do not change. A board may
be too small to occupy a GPU well, while tens or hundreds of independent
coordinator candidates provide natural parallel work.

## Particle count policy

Particles are cheap when they remove divergent special cases or turn awkward
rigid-body coupling into uniform constraints. They are expensive when they
inflate broad-phase pairs, require global reductions, or create ill-conditioned
stiff constraint graphs. Representation choice is therefore benchmarked, not
decided by object-oriented convenience.

The active ablation compares a two-terminal passive represented as:

- two terminal particles with a distance constraint;
- an analytic rigid body with body-local pads;
- a small shape-matching particle cluster.

The first two variants now have matched reduced CPU controls and viewer
evidence. The shape-matching cluster, representation-independent copper
occupancy, exact-candidate materialization, and GPU measurements remain. The
engine reports both constraint records and scalar attachment rows, but not byte
traffic; these counters alone are not suitable for GPU performance claims.

## CPU/GPU parity

The deterministic CPU backend is the semantic reference. GPU floating-point
results need not be bit-identical, but both backends must agree on constraint
activation, bounded work accounting, health classification, and exact final
verification within declared tolerances. No GPU backend may weaken validation
because readback is inconvenient.

The initial repository contains the GPU-shaped buffers and dispatch boundary,
but not a pretend GPU implementation. A real `wgpu` backend will be added only
with device execution, parity tests, and measured end-to-end evidence.

## Current scheduling decision

GPU support remains an architectural requirement, but it is not yet the
shortest path to faster dual-board iteration. In cold prefix-14 sequence 129,
the mixed semantic placement/routing feedback completes in 1.552 seconds,
while native KiCad progression takes 98.069 seconds of the 100.190-second
run. Sequence 113 independently showed the same shape: ordinary local route
calls were a small minority of total time. A GPU port of the current scalar A*
would therefore leave the dominant loop untouched.

Prefix-15 transactional fallback strengthens that result. Its one-iteration
semantic feedback takes 1.417 seconds, the rejected native trial takes
110.064 seconds, and the independently regenerated fallback progression takes
160.433 seconds. The correct 272.910-second transaction is slow because two
KiCad-gated candidate histories are evaluated, not because placement-force or
semantic-route arithmetic dominates.

The first useful device experiments should batch naturally parallel work:
placement density/demand fields, force gathering, collision/contact rows, and
counterfactual reachability across candidate states. Native ERC/DRC remains a
CPU/process-side admission authority. The host NVIDIA/Vulkan stack was checked
outside the filesystem sandbox on 2026-09-02: Vulkan 1.4 sees the discrete
GeForce GTX 1650 (4 GiB, driver 610.57.04). Device work is therefore testable,
but is deliberately deferred. When resumed, each kernel needs CPU/GPU parity
plus end-to-end timing that includes transfer and readback.
