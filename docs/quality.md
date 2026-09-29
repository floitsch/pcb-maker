# What makes a layout good: computable measures

pcb-maker's boards beat the designers' on track length and via count, but
those measure economy, not quality. This page collects the objective,
computable measures we use instead: to report them for any KiCad board, to
compare our boards with human ones, and to turn into placer and router
terms. Survey of 2026-09-29; sources are linked per row.

## Ground rules

- **No standard defines layout quality.** IPC-2221B, IPC-2152 and IPC-7351
  set fabrication and assembly limits. Almost all EMC, signal and power
  integrity guidance is physics-motivated rule of thumb. Hubing, who built
  one of the EMC expert systems, warns that "some of the worst PCB design
  choices are made by engineers trying to comply with a list of EMC design
  guidelines" ([Hubing 2003](https://doi.org/10.1109/ISEMC.2003.1236559)).
- **So the measures are scores, not pass/fail truth.** We report them for
  our board and the designer's side by side, and calibrate rule-of-thumb
  thresholds (marked RoT) on the distribution of human boards.
- **The core set the commercial checkers agree on**: nets crossing plane
  gaps, reference changes at vias without a return via, nets near plane
  edges, decoupling placement, filters at connectors, power trace width,
  parallel-run crosstalk, stubs, diff-pair symmetry
  ([LearnEMC list](https://learnemc.com/commercial-emc-rule-checkers),
  [HyperLynx DRC](https://www.saros.co.uk/wp-content/uploads/2024/02/HyperLynx-DRC-Standard-and-Developer-Edition.pdf),
  [Zuken EMC Adviser](https://www.zuken.com/en/products/pcb-design/cr-8000/products/emc-adviser-ex)).
- **Open-source KiCad checkers with reusable defaults**:
  [EMC Auditor](https://github.com/RolandWa/KiCAD_Custom_DRC) (decap
  within 3 mm, return via within 2 mm),
  [kicad-happy](https://github.com/aklofas/kicad-happy/blob/main/emc-precompliance.md)
  (44 rules, e.g. stitching at λ/20, switching inductor ≥15 mm from ADC,
  crystal, RF), [EMI Guardian](https://github.com/hap231/kicad-emi-guardian).
- **Academic placement still scores only wirelength, density and
  crossings**; no validated suite of layout-quality metrics against human
  boards exists. That comparison is ours to make.

## Roles come first

Most measures need to know what a part or net is for. The board file
carries enough to infer most roles: pads have `pinfunction` and `pintype`
(`power_in`), footprints have library names and values.

| Role | Inferred from | Ask the user when |
| --- | --- | --- |
| Ground | net names `GND`, `AGND`, `VSS`, `0V`; the largest zone's net | chassis vs signal ground |
| Rail | `pintype` power_in/power_out; names VCC, VDD, 3V3, 5V, VBUS, VIN, VBAT | |
| Decoupling cap | a capacitor with one pad on a rail and one on ground; ≤1 µF high-frequency, ≥4.7 µF bulk; assigned to the nearest power pin on its rail | a cap shared on purpose |
| Switcher | net joining an inductor to an IC pin named SW, LX, PH; its input cap; the FB net | unusual topologies |
| Fast net | crystal nets; names CLK, SCK, XTAL, USB D±/DP/DM, SDIO, QSPI; `_P`/`_N` pairs; net classes | edge rates |
| Analog | pins AIN, ADC, VREF, IN±, MIC; op-amps, codecs, audio jacks | sections of an audio board |
| Hot part | SOT-223, DPAK, TO-220 regulators, power transistors, exposed pads | the current |
| Edge connector | USB, barrel, RJ45, audio jack, SD, cable headers | the intended overhang |
| Antenna | ESP32/WiFi/BLE modules, chip antennas; the footprint's keepout area | |

## The measures we implement first

Ranked by value per effort. P: usable as a placer term, R: as a router
term.

| Rank | Measure | Definition and threshold | Term |
| --- | --- | --- | --- |
| 1 | Reference coverage (return paths) | Every fast-net segment has ground copper under it on the reference layer (the other side on two layers) over w/2+2h; count uncovered runs and length, and the detour ratio around gaps. 0 crossings ([TI SPRAAR7A](https://e2e.ti.com/cfs-file/__key/communityserver-discussions-components-files/171/USB-2.0-Board-Design-and-Layout-Guidelines.pdf) §2.1, [Ott](https://hott.shielddigitaldesign.com/techtips/split-gnd-plane.html)); detour ≤1.5 (RoT) | R |
| 2 | Return vias | A ground via within 2 mm of every fast via that changes reference layer (EMC Auditor; [Bogatin](https://www.informit.com/articles/article.aspx?p=2916283&seqNum=14)) | R |
| 3 | Decoupling proximity | Every IC power pin has a decoupling cap whose copper path to the pin is ≤3 mm (target 2) | P, R |
| 4 | Switcher hot loop | Input-cap path to VIN and PGND ≤1-2 mm, loop area ranked, switch-node copper small, FB ≥ a few mm from the switch node and inductor ([TI SNVA021](https://www.ti.com/lit/pdf/snva021)) | P, R |
| 5 | Antenna keepout | No copper and no courtyard in the antenna area on any layer; antenna at or over the edge ([Espressif](https://docs.espressif.com/projects/esp-hardware-design-guidelines/en/latest/esp32/pcb-layout-design.html)) | P, R |
| 6 | Connectors face outward | The mating face at the edge, pointing out | P |
| 7 | Ground integrity and stitching | No floating ground islands; islands with ≥2 vias; stitching ≤λ/20 (≈8 mm at 1 GHz, ≈3.4 mm near 2.4 GHz antennas; RoT) | R |
| 8 | Current capacity | Narrowest width along a supply path ≥ the IPC-2152 width for its current at ΔT 10 °C | R |
| 9 | Separation | Inductor and switch node ≥15 mm from ADC, op-amp, crystal, RF (RoT); no digital copper in the analog region ([Ott](https://www.edn.com/partitioning-and-layout-of-a-mixed-signal-pcb-3/)) | P, R |
| 10 | Crystal | Short XTAL path (≤≈5 mm, RoT); no foreign copper under the crystal ([ST AN2867](https://www.st.com/resource/en/application_note/an2867-oscillator-design-guide-for-stm8afals-stm32-mcus-and-mpus-stmicroelectronics.pdf)) | P, R |
| 11 | I/O protection | ESD diode or filter ≤≈3 mm from the connector pin and on the path; interface IC ≤2 cm from its connector ([TI SLVA680](https://www.ti.com/lit/pdf/slva680)) | P, R |
| 12 | Stubs and pairs | USB ≤100 mm, stubs <5 mm, pair matched and coupled | R |
| 13 | Tombstoning and via-in-pad | On ≤0603 parts, conductive width into one pad ≤2× the other; no vias in non-exposed pads | R |
| 14 | Hot parts | Exposed-pad via array per datasheet (≈0.3 mm drill, 1-1.2 mm pitch, solid); copper area around hot parts ([TI SLMA002](https://www.ti.com/lit/slma002)) | P, R |
| 15 | Assembly | ≥90 % of polarized parts at one orientation per side (IPC-2221B §8.1.3); spacing per the assembler's table; bodies ≥2.5 mm from the edge (else rails); SMD on one side ([JLCPCB](https://jlcpcb.com/help/article/minimum-spacing-for-smd-components)) | P |

Almost free: KiCad's own DRC covers most manufacturability (slivers, mask
bridges, starved thermals, annular ring, courtyards, silkscreen); report its
counts by category.

## Further measures (lower priority)

- Fast nets near board and plane edges: ≥2h from the reference edge (≈3 mm
  on a 1.6 mm two-layer board).
- Crosstalk: coupled length of aggressor-victim pairs closer than 3w (RoT).
- Heat-source separation from sensitive parts (electrolytics, references,
  sensors): RoT ≥5-10 mm.
- Acute angles, copper balance between layers, teardrops: low weight.
- Test access, fiducials, mounting-hole keepouts, reachable buttons and
  LEDs.

## Validation

Compute every measure on the designers' boards and on ours for the same
netlists, and report per-measure deltas and "no worse than the human"
rates rather than absolute pass/fail. Human boards break many rules of
thumb too; calibrate those thresholds on their distribution.
