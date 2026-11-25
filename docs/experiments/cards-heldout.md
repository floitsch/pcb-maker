# Do the why-open cards hold on held-out cases? (2026-11-25)

A held-out test of `whyopen.py`, unchanged (blob 67daf0c3), against `evaluation-representations.md`'s
12 of 12. Data: `experiments/eval-representations/heldout/`; transcripts, cards, judged boards:
`build/scratch/cards-heldout/` (not in git).

## Verdict

**The 12 of 12 does not generalise.** On 12 new items the card arm was right 8 times and so was the raw
arm; the generator's own verdict was right 4 times, all on cause B. **The card wins on cost, not on
correctness**: 47k tokens, 3.3 tool calls and 49 s per case against 69k, 9.0 and 195 s. Card subjects
were right where they overrode the verdict from the reach numbers and the picture (h1, h4, h7) and wrong
where they followed it (h5, h6, h9). Every miss of either arm is on an item whose truth is A or E.

## Selection (written before any listing: `heldout/selection-rule.txt`)

Rows of the 2026-11-24 nightly's sweep and designer stages with 1 to 6 KiCad unconnected items, minus the
study's 12 boards (any tier) and the held-out boards; sorted by name, every k-th (k = n // 6, from k // 2),
6 per stage, a board name once, item 0. The rule also excluded the window census's boards unless a stage
kept fewer than 6; the designer stage kept none, so **the 6 designer cases were in the study's 117-item
run** (not among the cases it was iterated on). whyopen.py ran without a crash on all 12.

## Ground truth (written before the subjects ran: `heldout/truth.json`)

From KiCad's DRC on hand-drawn copper, the router's reports and logs, pcbnew geometry and two raster
checks of our own. The study's classes (A single-net path, B routed copper, C input, D pour, E long) and
scoring rule (the letter must match); totals also without truths under 60 % confidence (h0, h5).

| case | board (stage) | net | truth (conf.) | card verdict | card arm | raw arm | tokens card / raw | s card / raw |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| h0 | 0xCB-1337 pcb-panel (sweep) | PORT+ | A (45) | path rejected | A ok | B x | 45k / 76k | 39 / 210 |
| h1 | protoesp (sweep) | CH_SDA | E (80) | end walled, C | E ok | A x | 47k / 60k | 60 / 164 |
| h2 | vaio_re (sweep) | +3V3E | B (70) | B | B ok | B ok | 44k / 74k | 25 / 299 |
| h3 | MIDAS-MK1.1 (sweep) | U106-DS | A (80) | C | B x | B x | 53k / 70k | 114 / 207 |
| h4 | rp2040 motor ctrl (sweep) | M1C_PWM_L | B (60) | end walled, C | B ok | B ok | 52k / 67k | 67 / 266 |
| h5 | Sisu_SSE-9 (sweep) | +3.3V | A (55) | B | B x | B x | 45k / 64k | 34 / 98 |
| h6 | eurorack-pmod (designer) | D18-A | A (75) | path rejected | C x | A ok | 45k / 78k | 46 / 188 |
| h7 | laptop motherboard (designer) | CC_INT_L | C (85) | long, E | C ok | C ok | 50k / 55k | 55 / 66 |
| h8 | Quanta75 (designer) | +1V1 | B (75) | B | B ok | B ok | 47k / 70k | 66 / 223 |
| h9 | poly_corne right (designer) | D-C | A (80) | long, E | E x | A ok | 47k / 70k | 29 / 213 |
| h10 | allwinner a13 module (designer) | ddr-dqs0- | B (80) | B | B ok | B ok | 46k / 79k | 32 / 225 |
| h11 | laptop power (designer) | HIDRV1 | B (85) | B | B ok | B ok | 43k / 66k | 26 / 185 |

Truths in brief: h0 the report names the wrong panel copy (shared pad UUIDs); on the open copy two 0.41 mm
vias in the pads pass DRC. h1 58.7 mm, never routed, both ends free. h2 SW_LID boxes U6.1. h3 a 0.35/0.25
via (the router's own via-in-pad size on that BGA) and 0.5 mm of B.Cu pass DRC. h4 U15.6's track turns under
pad 7. h5 a 0.25 mm via in U4.26 into the In3 pour passes DRC. h6 a track in the strip along the board edge
(edge clearance 0) and two class vias pass DRC. h7 centre ball of a 0.4 mm WLCSP: no track or allowed via
fits. h8 USB1_D+ and +3V3 close U1.50. h9 a 31 mm path at the router's narrowed width passes DRC. h10
dogbone vias of ddr-d4/dqs0+ and dqs1- seal both ends. h11 /SW1 and /BTST1 close U4.31.

## Totals

| arm | correct | truth conf. >= 60 | conf. right / wrong | tokens | tool uses | s |
| --- | ---: | ---: | --- | ---: | ---: | ---: |
| generator verdict alone | 4 / 12 | 4 / 10 | - | - | - | - |
| card | 8 / 12 | 7 / 10 | 69 / 70 | 47k | 3.3 | 49 |
| raw | 8 / 12 | 8 / 10 | 80 / 64 | 69k | 9.0 | 195 |

By truth: B 5/5 in both arms; A 1/5 card, 2/5 raw; C 1/1 both; E 1/1 card, 0/1 raw. The card subjects'
confidence does not separate right from wrong; the raw subjects' does. Two readings of the classes move
the numbers (not rescored): if a via smaller than the class's or in a pad does not count (h0, h3, h5 become
B), raw is 11/12 and card 9/12; if A and E are one answer for a long item with free ends (h1, h9), both 9/12.

## Where the card was wrong or misleading

- **h3 C, h5 B, h0 rejected: it tries only the class via, never one in a pad.** All three close through a
  small via or a via in a pad. At h3 it says "no path even without other tracks" with no reason.
- **h1, h4 `end_walled_by_fixed_objects` is wrong.** The stripped search lists routed tracks as fixed walls
  (CH_VCC, M1B_PWM_H), at h1 also a drilled hole although the PTH pad is open on B.Cu; at h4 a 0.2 mm track
  between 0.4 mm-pitch pads keeps exactly the clearance, which the raster counts as blocked. Both card
  subjects saw the contradiction and overrode it.
- **h6 `raster_path_rejected_by_kicad` misled.** The raster path crossed the board edge (3
  copper_edge_clearance); a hand path in the same strip passes. The card subject read "rejected" as "no
  legal path" and answered C at 68 %; the raw subject drew the path and got A.
- **h7, h9 `long` is no cause.** Beyond 20 mm only the ends are checked. At h7 it read the rules as track
  0.0 / clearance 0.0 (the widths come from custom rules), so every reach search is null; the subject found
  the WLCSP wall in the picture. At h9 a legal path exists and the card subject took "long".
- **h0 the wrong copy.** It worked where KiCad's text points, on a copy already joined: its path changed
  nothing (3 open before and after) and its vias broke the drill rule.
- **Right on B** (h2, h8, h10, h11), read in 30 s; the nets it names are not always the walling ones.

## Causes the generator cannot name

1. A path through a via smaller than the class's or a via in a pad (h0, h3, h5).
2. For ends over 20 mm apart, anything but a walled end: a long legal path (A) or congestion on the way (B).
3. Anything where the widths come from custom rules it does not read (h7); a legal path next to the
   raster's rejected one, for want of a retry (h6); a KiCad item naming the wrong copy of a panel (h0).

## Conclusion

The card does not beat raw on correctness (8 against 8; 7 against 8 on confident truths); its verdict
alone is right a third of the time. It wins on cost: 32 % fewer tokens, 37 % of the tool calls, 25 % of the time. On B it is as good as raw and cheaper; on A it is worse, because its search cannot find
the small-via, edge-strip and long paths that raw subjects found by drawing copper or running a maze.
Before a port: search with every via the router may use and vias in pads, retry with a margin before
calling a path rejected, search across long items, read custom rules, and show the verdict as evidence.

Caveats: one subject per cell (Opus, the study's prompts), 12 cases; truth by one investigator with three
forks, under class definitions that overlap (A/E for long items, A/B for small vias); verdict right on 3 of
the 6 designer cases (seen in the 117-item run) and 1 of the 6 sweep cases (never seen).
