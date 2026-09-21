# Complete cold PIC routing after explicit junction materialization

The full adapted-rule PIC programmer benchmark now routes all 34 nets from
zero copper with unchanged placement and zero native ERC, design DRC, parity
or connectivity findings. The previous current-program baseline stopped after
one net because KiCad rejected a dangling GND track. Freerouting still leaves
one native open in the matched comparison.

| Result | Completed nets | Native opens | Track mm | Vias |
| --- | ---: | ---: | ---: | ---: |
| Previous pcb-maker baseline | 1 / 34 | 114 | 127.767 | 0 |
| Corrected pcb-maker | 34 / 34 | 0 | 1867.509 | 20 |
| Freerouting | not reported as a net count | 1 | 2081.421 | 0 |

Both tools use the same cold board and fixed poses. This PIC manifest flattens
the source's POWER class to Default: the result proves complete routing under
those declared adapted rules, not equivalence to the original electrical rules.
The corrected run gives both competitors a 600-second process cap. pcb-maker
finishes in 201.334 seconds, including its per-net native checks, and uses
3160599 A* expansions. The prior PIC run failed native admission rather than
timing out; increasing the cap did not repair that failure.

## Diagnosis and controlled repair

The GND branch endpoint at (113.91, 85.89) lies exactly on an earlier GND
diagonal from (124.91, 74.89) to approximately (110.454189, 89.345811). The
grid tree recognizes this attachment, but the stored parent track remained
unsplit. KiCad reports the adjacent 0.5 mm branch segment as `track_dangling`,
despite the network's electrical connectivity being correct.

A control replays the saved candidate. Its paired repair changes only the
parent's segmentation by inserting the attachment coordinate. Native results:

- Control: one dangling-track finding, 75 expected remaining opens.
- Split parent: zero design findings, the same 75 remaining opens.

An independent read-only pcbnew audit normalizes exact integer centerline
intervals by net, layer and width. It proves identical native copper support
and identical component poses. This is not a workaround that deletes a
difficult route or ignores a warning.

The six-terminal generated regression separately fails before the production
fix at an unsplit contact (8.625, 8.625). After the fix, it contains explicit
vertices at every planar contact. Its stored segment count changes from seven
to ten while A* expansions remain 109. Before/after SVGs are retained.

## Production change

`crates/pcb-kicad/src/lib.rs` now normalizes shared-tree branch intersections
after all branches have been selected, and regenerates supplemental native
copper from that normalized graph. Search policy and route selection are
unchanged. The existing normalizer handles same-layer crossings and collinear
contacts without conflating crossings on different layers.

A related guard preserves single-point branches representing terminals already
reached by the tree. Such branches add no segment but still carry contact
evidence. A second regression covers that case. All 106 KiCad library tests and
seven competitive-benchmark library tests pass. The replay viewer's JavaScript
passes syntax checking; interactive browser playback was not exercised.

The complete paired run uses executable SHA-256
`834f13f937d3a6573d1ed2698a9f71690778206f6622cf86be1fb6384b259e97`.
The final executable, including the single-point guard, is
`0afcc312e689cdde726a646a0c845be28ba806ea05567253d754a3a90fef1de7`.
The final replay regenerates every selected candidate and checks both candidate
equality and byte-identical resulting boards at all 34 steps, then runs a fresh
native verification of the final complete project. Per-step renders are reused
only after exact board equality is established.

## Evidence and next work

- [Controlled junction repair and native geometry audit](../../build/pic-junction-contact-2026-09-08/index.html).
- [Generated regression, before](../../build/pic-junction-contact-2026-09-08/regression/before.svg)
  and [after](../../build/pic-junction-contact-2026-09-08/regression/after.svg).
- [Full paired benchmark](../../build/whole-board-pic-junction-fix-2026-09-08/index.html).
- [Final executable replay and routing playback](../../build/whole-board-pic-final-replay-2026-09-08/index.html).

The benchmark has now converted a local native finding into a generic program
repair and a complete-board result. The next priorities are preserving original
per-net electrical rules and admitting the known human placements into a faithful
placement model, so that smaller complete boards become a valid objective.
The broader placement/router goal remains incomplete.
