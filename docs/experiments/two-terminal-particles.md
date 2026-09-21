# Two-terminal representation ablation

## Question

Should a movable two-terminal body use two endpoint particles joined by a
distance constraint, or an analytic rigid body with body-local terminal
attachments?

## Controls

Both representations are explicit compiler policies. They receive the same
semantic component, terminal geometry, masses, field weights, solver settings,
and initial route samples. `Unspecified` remains a compile error.

Two reduced controls are retained:

1. an 80-step asymmetric field/trace load with one projection sweep per step;
2. an isolated resistor whose second terminal is displaced downward, followed
   by 30 one-sweep relaxation steps with the density field disabled.

Run `cargo run -- resistor-ablation run/resistor-ablation` to regenerate the
JSON evidence and four self-contained viewers. The rigid-body viewer draws the
body pose and dashed body-local attachment residuals instead of inferring a
body from terminal particles.

## Measured CPU-reference result

| Asymmetric field load | Endpoint particles | Analytic rigid body |
| --- | ---: | ---: |
| Hot particles | 16 | 16 |
| Explicit bodies | 0 | 1 |
| Equality constraints | 13 | 12 |
| Attachments | 0 | 2 |
| Counted projections/step | 19 | 20 |
| Scalar constraint rows/step | 19 | 22 |
| Resistor center displacement | 1.122 mm | 1.032 mm |
| Resistor rotation | 0.0086 rad | 0.0047 rad |
| Maximum particle motion | 3.008 mm | 3.114 mm |
| Initial maximum residual | 1.720 mm | 1.720 mm |
| Final maximum residual | 1.126 mm | 1.000 mm |

The field fixture is still a poor rotation test: despite asymmetric route
samples, its net torque is small. It does show a modest residual improvement
for the rigid representation at slightly higher counted work. Neither variant
converges under the intentionally inadequate one-sweep budget.

| Asymmetric terminal perturbation | Endpoint particles | Analytic rigid body |
| --- | ---: | ---: |
| Counted projections/step | 1 | 2 |
| Scalar constraint rows/step | 1 | 4 |
| Final rotation | -1.052 rad | -1.010 rad |
| Center displacement | 3.500 mm | 1.750 mm |
| Final maximum residual | 0.00000048 mm | 0 mm |

Both variants rotate freely. This is evidence that the endpoint model's
implicit rotational degree of freedom can be an advantage, not merely a
correctness risk. The analytic body's explicit pose reduces center drift in
this perturbation, but the different response also shows that terminal/body
mass distribution is not yet a representation-independent physical contract.

## Advantages and penalties

Endpoint particles keep one uniform particle/distance kernel and transmit
trace corrections directly. They need no pad Jacobian or body buffer, and the
isolated control uses half as many counted constraint records. Rotation emerges
naturally.

The analytic model makes rotation permission explicit, gives the viewer and a
future exact-candidate materializer a stable body pose, represents physical
extent directly, and supports body-local pads. It adds a body buffer, inertia,
and two attachment records for a resistor.

The engine reports both constraint records and scalar rows because one
attachment projection solves x and y separately. Thus the field comparison is
22 versus 19 scalar rows, and the isolated resistor is 4 versus 1. Scalar rows
are still not a GPU timing or bandwidth measurement; byte traffic and kernel
occupancy remain to be measured.

## Decision

Neither representation is the default and neither is rejected. Both remain
selectable experiments. The endpoint model passed the important free-rotation
control; the analytic model supplies the missing explicit-pose control and
slightly better residual on the field fixture.

The next fair comparison needs representation-independent occupancy, calibrated
mass/inertia semantics, residual histories, memory accounting,
GPU timings, and materialization into the same exact-validated candidate. A
small shape-matching particle cluster remains a third planned representation.
