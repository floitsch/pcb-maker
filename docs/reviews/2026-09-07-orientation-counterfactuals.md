# An analysis tool for stale component orientations

The corrected harmonic placement still chooses port orientations before the
final component legalization. A fixed-center counterfactual diagnostic now
exposes rotations that become attractive at the final positions. It answers
three concrete questions: which individual quarter-turn improves the declared
attraction objective, which branch distances improve or worsen, and whether the
rotation is legal without moving any component.

On the complete corrected dual-ESP32 placement, 22 individual rotations reduce
the weighted squared pad-distance objective. The twelve highest-ranked trials
contain eight legal rotations and four rejected ones. The four rejected trials
would conflict with component placement envelopes. Every trial has a retained
SVG/PNG and the production rejection reason; alternatives are evaluated
independently, not accumulated into an unchecked combined proposal.

The leading candidate is `R_LED` at 180 degrees. It shortens the straight pad
separation on `LED_SUPPLY` from 4.272 to 2.355 mm and on `LED_SERIES` from 5.002 to
3.317 mm. A native probe of `LED_SERIES` confirms a routing benefit:

| Corrected placement | Native LED copper | Vias | A* expansions |
| --- | ---: | ---: | ---: |
| Original orientation | 5.441 mm | 0 | 750 |
| Only `R_LED` rotated 180 degrees | 3.441 mm | 0 | 73 |

Both probes use the same executable, routing configuration, 0.35 mm trace width,
0.20 mm clearance and 43 component centers. Exact input comparisons establish
that the only pose change is `R_LED`'s rotation; native audits check the supplied
poses survive materialization and routing. Both selected boards have zero ERC,
design DRC, parity or selected-net open findings. Metadata warnings are 20 and
21. This is a one-connection routing test; it does not validate the supply or
ground routes or the full board.

The per-branch breakdown matters. For example, rotating `C_E_A` can improve the
weighted objective while lengthening its GND connection. The tool reports that
tradeoff rather than treating a better scalar score as electrical validation.
Attraction weights express the input objective; they do not replace electrical
placement constraints or routing checks.

A planted two-terminal resistor fixture recovers its known optimal rotation.
Its narrow allowed region rejects both perpendicular alternatives. Setting
both branch attraction weights to zero produces no positive counterfactuals.
The controls and rejected alternatives are rendered automatically.

The current experiment intentionally accepts legacy two-terminal branches and
one geometric pad center per pin. It rejects unsupported explicit multi-terminal
nets and multiple pad centers. It uses production placement validation, but the
ranking is a geometric surrogate and changes no production placement defaults.
The next implementation step is a bounded, legality-checked orientation
refinement after legalization, with explicit scoring and retained alternatives.

- Tool: `experiments/placement-locality/orientation_probe.py`
- [Local before/after views with pad labels](../../build/placement-orientation-diagnostic-2026-09-07/details.html)
- [All twelve counterfactuals](../../build/placement-orientation-diagnostic-2026-09-07/index.html)
- [Native rotation result](../../build/placement-orientation-native-2026-09-07/native-preview.svg)
- [Planted controls](../../build/placement-orientation-controls-2026-09-07/planted/index.html)
- Reproduction: `experiments/placement-locality/README.md`
