# Automatic silkscreen repair after placement

The complete automated pipeline now admits ECC83 at **80% and 70% of the
original area**. At 70%, pcb-maker cold-places all 15 footprints, repairs the
annotations, and routes all nine nets with zero opens, ERC, design DRC or
parity findings. This is a 30% reduction under the explicit all-free benchmark
policy; it is not an enclosure-compatible replacement claim.

| Area ratio | Native area | pcb-maker track / vias | Freerouting track / vias | Native completion |
| --- | ---: | ---: | ---: | --- |
| 80% | 1930.963866 mm² | 362.705 mm / 3 | 352.394 mm / 2 | Both complete |
| 70% | 1689.593432 mm² | 386.308 mm / 10 | 340.647 mm / 0 | pcb-maker complete; Freerouting has one open |

Original outline area is 2413.704850 mm². Freerouting's remaining 70% open is
P1 pad 2 on `Net-(P1-PM)`. Its incomplete result does not qualify for a route
quality comparison. These are two development cases with one seed, not evidence
of broad superiority or an optimal area frontier.

The final executable SHA-256 is
`d1ced13d38b5efcbc2c0fbdf6aba30f65f766f3b93b27263a15ce5e60290bc57`.
Both comparisons finish and pass the shared placement and basic routing-rule
checks. The DSN basic geometry/net-pin audit passes for both cold inputs. Final
native readback preserves all 15 footprints and 33 original pads, and all 15
reference labels remain visible with no other-courtyard overlap. All 31 pinned
source files are unchanged. There are six/five library metadata warnings for
edited footprints at 80%/70%, respectively; the shared native policy retains
these separately from design findings.

The annotation repair clears all 18 findings on the previously routed 80% ECC83
board without changing copper, pads, component placement, project rules, or
non-silkscreen records. Native verification reports zero opens, ERC, design DRC
and parity findings. Edited library footprints retain separate metadata warnings.

The first DRC-clean proposal still put several reference labels inside other
components' courtyards. A silkscreen-only rendering exposed this, and an
independent geometry check measured seven such conflicts in the input, five
after the first repair, and zero after adding courtyard obstacles for labels.
This is a concrete example of native findings plus targeted rendering plus
local geometric queries identifying a problem that native DRC alone missed.

## Program behavior

```sh
pcb-maker repair-kicad-silkscreen SOURCE_DIRECTORY BOARD_ID OUTPUT_DIRECTORY
```

The command copies the project into a fresh output directory, runs a bounded
annotation proposal, independently checks the protected design, and invokes
native verification with an automatic preview. Cold boards with open connections
are supported; `silkscreen_complete` is separate from full native completion.

- Straight front-silkscreen strokes are clipped against the rectangular outline,
  exposed-pad bounds, and other footprints' strokes. Shorter strokes have
  priority over long outlines. Widths are retained; fully unprintable portions
  may be removed. Curves and non-front graphics are retained.
- Reference text keeps its content, font size, stroke thickness, visibility and
  identity. The search tries positions on a 0.5 mm grid within 8 mm of the old
  location, with the original and perpendicular text angles. It uses native
  text bounds and excludes pads, other text, graphics, and other courtyards.
- The geometric margins are 0.10 mm between annotations/obstacles and 0.15 mm
  at the outline. Segment sampling uses a half-step distance guard, so checking
  interval midpoints does not overlook a narrow crossing. Native verification
  remains the final gate.

The worker loads and serializes only a scratch project. It extracts modified
front-silkscreen records and patches them into the untouched source text.
The Rust wrapper independently compares the parsed non-front-silk structure
and label properties, and checks project bytes and the source file. It rejects
unresolved references, native annotation findings, failed workers and failed
preservation checks. Each result retains `silkscreen-proposal.json`,
`silkscreen-repair.json`, logs, native reports and a preview.

## Controls and limits

All 109 KiCad library tests pass. The new structural guard test rejects changes
to copper, pad positions, footprint placement, label content and visibility,
while allowing a reference pose change. The prototype's independent audit also
compares copper and placement fingerprints and all non-silk lexical tokens.

At 70% area, the next native test identified two strokes over exposed pads.
Adding solder-mask openings to stroke clipping removes both. The placement
then has zero ERC/design DRC/parity findings and 20 expected pre-routing opens.

The experiment runner originally classified library-mismatch warnings from
edited silkscreen as physical errors even though the shared native policy
tracks them separately. It now uses the same exact warning classification and
asserts that its design-finding count matches the native verification summary.
Other warnings and mismatch errors remain design findings.

The implementation currently targets rectangular outlines and front annotations.
Noncircular courtyards and pad openings use conservative bounding rectangles.
It does not infer the semantic meaning of every graphic or optimize reference
visibility after parts are assembled. Original fabrication-layer drawings remain
unchanged. A native-clean annotation result is not an electrical design review.

The final 70% silkscreen view exposes a remaining quality gap: one 1.0 mm stroke
of C1's board-edge polarity mark is shortened to 0.55 mm (source UUID
`323ffad9-f9ea-4acb-a4e3-53d65e59c610`). The native gate accepts the shortened
mark, but important symbols should be recognized and relocated as intact groups.
The protected-group follow-up below addresses this particular mark; native
area admission is still not proof of general semantic marking quality.

## Protected mark follow-up

The worker now recognizes isolated two-stroke centered crosses, including
rotated crosses, and tries rigid translations within 2 mm on a 0.1 mm grid.
It preserves both stroke identities, lengths, widths, relative geometry and
nearest owning-pad association. Other drawings, pads, board edges and other
component courtyards constrain the move. Protected groups become obstacles
to subsequent line clipping. If relocation fails, the intact mark remains
and annotation admission fails with `unresolved_marks`.

On the completed 70% board, restoring C1's original mark and then running the
production command moves the cross (-0.7, -0.5) mm. Both arms remain 1.0 mm.
The resulting whole board has zero native ERC, design DRC, parity and opens,
with seven separately recorded library metadata warnings. Independent native
readback confirms rigid motion and a lexical audit confirms every non-front-
silkscreen token is identical. The same command on the original cold 70%
placement also passes annotation admission: zero design findings and 20
expected pre-routing opens.

Seven geometric controls cover edge relocation, rotated relocation, an
already legal rotated cross, unequal arms, a T junction, an attached third
line, and impossible relocation. Each run emits a marking-only SVG. Two Rust
guard tests pass, including rejection of arm shortening, separate arm motion,
stroke-width changes and protected board/label changes.

This recognizes a geometric shape, not electrical polarity or arbitrary
symbols. Unequal crosses and other unrecognized straight graphics still use
the existing clipping policy. The first recognizer's bounding-box isolation
check missed C1 next to its circular outline; guarded stroke-distance checks
resolved that false negative. The initial prototype native verification lacked
copied library tables; the production project-copy result above includes them.

Reproduce the geometric controls:

```sh
python3 experiments/whole-board/mark_group_controls.py build/mark-controls-NEW
```

Evidence: [native and token audit](../../build/ecc83-mark-groups-2026-09-08/audit.json),
[marking view](../../build/ecc83-mark-groups-2026-09-08/result/silkscreen.svg),
[seven controls](../../build/ecc83-mark-groups-2026-09-08/controls-02/controls.svg),
[cold production result](../../build/ecc83-mark-groups-2026-09-08/cold-result/silkscreen-repair.json).

## Artifacts

- [Complete pipeline, both routers and automatic previews](../../build/ecc83-clean-area-final-2026-09-08/summary.html)
- [Post-terminal invariant and association audit](../../build/ecc83-clean-area-final-2026-09-08/validation.json)
- [30% smaller routed board](../../build/ecc83-clean-area-final-2026-09-08/area-01/comparison/pcb-maker/result/preview.svg)

- [Initial protected repair audit](../../build/ecc83-silkscreen-2026-09-08/audit.json)
- [Independent label-association comparison](../../build/ecc83-silkscreen-2026-09-08/label-association-audit.json)
- [Silkscreen after courtyard-aware label placement](../../build/ecc83-silkscreen-2026-09-08/courtyard-silkscreen.png)
- [70% mask-clipping control](../../build/ecc83-silk-mask-control-2026-09-08/result/preview.svg)

Run the area pipeline with annotation repair before both routers:

```sh
python3 experiments/whole-board/area_probe.py build/clean-area-RUN \
  --placement-policy benchmarks/real/ecc83-pp/placement-policy.json \
  --placement-attempts 32 --ratios 0.8 0.7 --repair-silkscreen --route
```

Full area credit still requires the complete pipeline's native routing and
placement admission. Geometry-only packing and open boards receive none.
The two final cases above meet that contract. More seeds, lower area targets,
placement-quality search, product constraints and other board families remain.
