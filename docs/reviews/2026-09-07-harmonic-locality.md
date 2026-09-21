# Harmonic placement: LED locality and GND attraction

The user's observation exposed two production defects. `R_LED` was 19.374 mm
from `D_LED` in the retained full harmonic placement.

First, harmonic relaxation allowed region-constrained components to move outside
their permitted regions. The final constraint projector returned `D_LED` and
`J_POWER` to their regions, leaving the unconstrained resistor behind. Projecting
component bounds during initialization and every Jacobi step lets those bounded
positions influence their neighbors. It also fixes initialization overwriting a
component's locked movement axis. Pairwise relational constraints and physical
non-overlap still use the common final projector.

Second, harmonic attraction and port orientation ignored `tension_weight`. This
benchmark already assigns GND branches 0.05, but the placer treated them at full
width-derived strength. Harmonic and port attraction now use width times the
declared tension weight. This applies to legacy branches and explicit electrical
nets. Zero-weight nets contribute no attraction edge or artificial anchor;
their electrical connectivity, physical demand, routing width and clearance
remain intact. Other placement policies have not changed.

A controlled full-netlist comparison uses identical inputs, seed and policy:

| Implementation | LED/resistor center separation |
| --- | ---: |
| Original | 19.374 mm |
| Bounds participating in relaxation | 3.817 mm |
| Bounds plus declared attraction weights | 3.317 mm |

All three proposals pass production legality and independent full-body,
board-containment and movement/rotation checks. The final code passes all 26
placement unit tests, including the two new regressions covering both net
representations and zero-weight behavior. Both release builds succeed.

Native KiCad probes then preserve all 43 poses and route only `LED_SERIES`,
at 0.35 mm width and 0.20 mm clearance. Original copper is 20.240 mm; corrected
copper is 5.441 mm. Both have zero vias and zero ERC, design DRC, schematic parity
or selected-net open findings. Library metadata warnings are 19 and 20,
respectively. The first baseline native runner was interrupted with exit 143;
its outcome is not counted. A fresh replay completed and supplies the baseline
measurement. All experiment records and renderings are retained.

This is evidence for the local fix, not a rerun of the nineteen-connection dense
routing benchmark. Ground's weak placement attraction is explicit input intent,
not a claim that ground geometry is unimportant. There is no hard-coded GND name
exception. The final legalizer can still alter relative port directions after
orientation selection; the result is a useful seed rather than a local optimum
of all pad-to-pad lengths.

- [Before / bounds-only / corrected view](../../build/placement-locality-2026-09-07/index.html)
- [Corrected placement](../../build/placement-locality-2026-09-07/weighted/placement.svg)
- [Native LED route](../../build/placement-locality-2026-09-07/weighted/native-preview.svg)
- Reproduction: `experiments/placement-locality/README.md`
- Implementation: `crates/pcb-placement/src/placement.rs`
