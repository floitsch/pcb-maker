# What a board costs to make, and what layout changes about it

Fab and assembly prices as of 2026-09-29, and the layout decisions that
move them. Almost all numbers are live quotes from the fabs' own
calculators (queried without a login, no coupons, net of VAT and shipping
unless noted); OSH Park and AdvancedPCB publish fixed prices; [3P] marks
third-party figures. Prices change often: re-snapshot before relying on a
number. (The query scripts use undocumented calculator endpoints and are
kept out of the repository.)

## The short version

- **Asian fabs (JLCPCB, PCBWay, Seeed) price by cliffs and add-on fees.**
  Below a size limit a board costs a flat few dollars; each special
  process adds a fixed fee that often exceeds the board.
- **European pool fabs (Aisler, Eurocircuits, Multi-CB, Würth, Beta
  LAYOUT) price by area and lead time**; design class, layer count and
  finish multiply the price.
- **Most of the money is in not crossing thresholds**, not in shaving
  area: board sides over 100/102 mm, via-in-pad, a second assembly side,
  drills under 0.3 mm, tracks under 6 mil.

## Reference job: 50 × 50 mm, FR4 1.6 mm, 1 oz, green, 6/6 mil, 0.3 mm drill

| Fab (finish, lead time) | 2L q5 | 2L q10 | 2L q100 | 4L q5 | 4L q10 | 4L q100 | Shipping to DE |
|---|---|---|---|---|---|---|---|
| JLCPCB (leaded HASL; 2 d / 3-4 d) | $4 web, $2 in the JLCONE app | $5 | $46.10 | $8 ($2 JLCONE) | $8 | $53.10 | ~$9 standard, $22 DHL [3P] |
| PCBWay (leaded HASL; 24 h / 4-5 d) | $5 | $5 | $61.53 | $25.97 | $50.85 | $113.97 | $5.37 Global Direct, $28.80 DHL |
| Seeed Fusion (HASL; 3-6 d) | $9.90 | $9.90 | $45.65 | $39.90 | $39.90 | $135.42 | n/a |
| Aisler (ENIG, multiples of 3; 10 WD) | €28.56 (6) | €43.11 (12) | €261.44 (102) | €33.54 | €53.07 | €346.13 | free |
| Eurocircuits (lead-free; 3 WD proto, 5-7 WD pool) | €68.20 | €90.10 | €275 | €101.40 | €134 | €401 | €0 |
| Multi-CB "Saving" (HAL lead-free; 12-13 WD) | €16.10 | €22.90 | €83 | €30.65 | €46.50 | €147 | €6.95 |
| Multi-CB Standard (4-5 WD) | €39.90 | €72.90 | €215 | €79.10 | €125.70 | €371 | €6.95 |
| Würth WEdirekt pool (10 WD) | €63.70 | €93.60 | €337 | €95.40 | €140.20 | €504 | €5.90 |
| Beta LAYOUT PCB-POOL (6 WD, test and stencil incl.) | €99.68 | €141.07 | €482.20 | €166.75 | €233.76 | €751.50 | not found |
| OSH Park (ENIG, US) | $38.75 (6) | $77.50 (12) | $387.50 | $77.50 | $155 | $790.50 | free, slow |
| AdvancedPCB $33/$66 specials | $165 | $330 | quote | $330 | $660 | quote | customs extra |

- JLC: lead-free HASL +$1.20, ENIG ~$21. PCBWay: the $5 needs leaded HASL
  (lead-free $17.65 at q10). Aisler's 2L HASL rate needs ≥0.2 mm track,
  ≥0.15 mm gap, ≥0.3 mm drill; 6-mil rules force ENIG.
- Lead time dominates in Europe: Multi-CB 2L q10 €22.90 at 12 WD, €160 at
  2 WD; Beta €141 at 6 WD, €456 at 2 WD.
- Multi-CB, WEdirekt and Beta LAYOUT reportedly sell to businesses only
  (not double-checked).
- "tstronic": TSTRONIC (Gdańsk) is an assembly contract manufacturer, not
  a board fab, and publishes no prices.

## Base price and size cliffs

| Fab | Base price | Cliff |
|---|---|---|
| JLCPCB | Flat if both sides ≤102 mm: 2L $4 (q5) / $5 (q10), 4L $8; batch pricing from ~q40 | Above 102 mm area-based: 102×103 $9.10, 150×150 $14.90 (2L q5), but 10×103 still $4.60. 4L jumps $8 → $31.60 at 103×103 |
| PCBWay | Flat $5 (2L) if both sides ≤100 mm and q ≤10; else ≈ $37 + $99/m² (2L), $77 + $150/m² (4L), fitted at q100 | 100×101 at q10: $34.27, ~7× |
| Seeed | Flat $9.90 (2L) / $39.90 (4L) to 100×100 at q ≤10 | 100×101 at q10: $31.84 |
| Aisler | €14 job + rate × area(cm²) × 3⌈q/3⌉; rates 2L HASL €0.067, 2L ENIG €0.097, 4L €0.13 per cm²; castellations free | Non-standard options (mask colour, copper, 0.8 mm, 6+ layers): "Beautiful Boards+", ~5-7× |
| Eurocircuits | ≈ €58 + €0.13/cm² per board (2L pool, q10, 7 WD) | Pool vs whole panel: red mask, 10+ layers, drill class E-F, pattern class 9+ |
| Multi-CB | ≈ €12 + €0.04/cm² of total area (Saving, 2L); 4L ≈ 2× | 90 µm track/space leaves the pool |
| WEdirekt | Roughly flat below 40 cm² per board | At 40 cm² per board +85% (2L q10 €99 → €183) |
| OSH Park | $5 per sq in per 3 boards (2L), $10 (4L), $15 (6L) | Linear in the bounding rectangle [3P] |
| AdvancedPCB | $33/board (2L, ≤60 sq in, min 3); $66 (4L, ≤30 sq in, min 4) | 6/6 mil, 10 mil hole, ≤35 holes/sq in, no slots or cutouts |

## Option surcharges (2L, q5-10)

| Option | JLCPCB | PCBWay | Seeed | Aisler | Eurocircuits | Multi-CB (Std / Saving) | WEdirekt |
|---|---|---|---|---|---|---|---|
| Via drill <0.3 mm | 0.25/0.2 mm +$17; 0.15 +$34 | free to 0.2; 0.15 +$200 | 0.25 +$30; 0.2 +$50 | ENIG/4L tier (≥0.25) | free to 0.25; class E +13% | free to 0.2 | <0.25 +9% |
| Track/space <6 mil | none ≥3.5 mil; 3-3.5 mil +20% (4-8L) | free to 4/4; 3/3 +$140 | 5/5 +$18; 4/4 +$56 | HASL needs 8/6 mil; ENIG/4L 5/5 | 125 µm free; 100 µm +40% | free to 100 µm; 90 µm ×2.2 | 125 µm +4.5%; 100 µm +12% |
| Via-in-pad (filled, capped) | 2L +$50; 4L +$17 | +$95-106 | n/a | n/a | +€67 | ×3.2 / ×5.5 | ×4 |
| Blind/buried vias (4L) | not found | ~$430 vs $241 | $526 vs $40 | no | n/a | €280-290 vs €126 | €425 vs €140 |
| Castellated holes | +$38-42 | +$20 | +$50 | free | +€15 | ×2.3 / ×3.4 | whole panel |
| Edge plating | +$51 | +$20 | n/a | n/a | +€15 | ×2.3 | whole panel |
| Impedance control | 4L +$33 | 2L +$58; 4L +$49 | +$65 | no | separate | ×2.6 | n/a |
| 2 oz copper | $38 vs $4 | $145 | error | BB+ €91 vs €19 | ×2.5 | +10% | +9% |
| Non-green mask | free (slower) | mostly free; matte/purple +$33 | black free | BB+ ~×7 | +55-85% | ×2.3 / +10% | n/a |
| Thickness other than 0.8-1.6 mm | 2.0 mm +$31 | 2.0 +$52 | 0.8 +$12; 2.0 +$41 | BB+ | 1.0 mm +60% | 1.0 forces ENIG | +22-27% |
| Each extra design | +$4; a panel loses the flat price | +$12-20 | +$12 | | | | |

No price difference was found between V-score and tab routing.

## Assembly (excluding parts)

| Fab | Fixed per order | Per unique part | Per SMD joint/placement | THT | Second side | Constraints |
|---|---|---|---|---|---|---|
| JLC Economic | $8.18 setup + $1.53 stencil | $3.07 per *extended* part (basic and preferred-extended free) | $0.0016/joint | $3.58/order + $0.0164/joint | not possible | 2-50 pcs; 0402+; parts ≥0.3 mm from the edge; no rails needed |
| JLC Standard | $25.56 setup + $8.21 stencil per side | $1.53 per part | $0.0016 → $0.0012 | as Economic | +$25.56 + $8.21 | board ≥70×70 mm or panel; rails and fiducials; parts ≥2.5 mm from the edge; handling ≥$14.93 |
| PCBWay | quote engine; $29 promo for q ≤10, ≤50 SMD, no BGA/QFP, ≤5 THT | ~$3.3-3.9 | ~$0.010 (q100) | ~$0.10/placement | +11-13% | panel if under 50×100 mm; rails if copper <3.5 mm from the edge |
| Eurocircuits (q5) | €255.60 for 20 unique / 50 placements | ~€5.8 | ~€0.11 | ~€1.1 | +€55 | 5 mm border |
| Beta LAYOUT (q10) | €199 without parts | | ~€0.30 | ~€1.97 | +€106 | 1-50 boards |
| Aisler | current rates unpublished | €7.50 (2022) | €0.04 (2022) | yes | yes | stencil €5 + €0.095/cm² per side |

## What layout can do about it, most money first

1. **Avoid special processes the router can cause**: via-in-pad ($17-106),
   blind/buried vias ($190-490), castellations and edge plating ($15-50).
   Each can exceed the whole board price.
2. **Assemble one side.** At JLC it allows Economic assembly and saves at
   least $60; elsewhere €55-106 or 13%.
3. **Keep both sides ≤100 mm** (≤102 at JLC; <40 cm² per board at
   WEdirekt). Crossing costs 2-7× the bare board, per side length, not
   area. At area-priced fabs cost is linear in the bounding rectangle,
   €0.04-0.13 per cm² per board.
4. **Stay in the free rule tier: 6/6 mil track/space, 0.3 mm via drill.**
   Crossing costs +$17 (JLC 0.25 mm drill), +$18 (Seeed 5 mil), ENIG at
   Aisler, +40% (Eurocircuits 100 µm), 2× (Beta 0.2 mm drill).
5. **Choose layers per fab.** At JLC 4L costs only +$3-7 while ≤102 mm: a
   smaller 4L board can be cheaper. Elsewhere 4L is +$21-46 or +35-100%.
6. **Fewer unique parts, JLC basic parts**: $1.5-3.1 (JLC), ~$3.5 (PCBWay),
   ~€5.8 (Eurocircuits) each.
7. **SMD over THT**: a THT placement costs ~10× an SMD one (small at JLC,
   €1.1-2 each in Europe).
8. **One design per order**, not a customer panel, at JLC and PCBWay.
9. **Edge rules**: parts ≥2.5 mm from the edge at JLC Standard, copper
   ≥3.5 mm at PCBWay, or rails are needed.

## Cost model

Total = board + options + assembly, from a per-fab profile. W, H: the
outline's bounding box in mm; A = W·H; q: quantity; L: layers; S:
assembled sides; U: unique parts; J: joints; P: placements.

- **Board**
  - JLC: max(W, H) ≤102 and q ≤10 → 2L $4 (q5) / $5 (q10), 4L $8; else a
    lookup table (2L q5 ≈ $4 + $100/m² of total area).
  - PCBWay: max(W, H) ≤100 and q ≤10 → 2L $5, 4L $26 (q5) / $51 (q10);
    else ≈ $37 + $99/m²·A·q (2L), $77 + $150/m² (4L).
  - Aisler: €14 + r·(A/100)·3⌈q/3⌉; r = 0.067 (2L within the HASL rules),
    0.097 (2L ENIG), 0.13 (4L).
  - Eurocircuits: €58 + €0.13·(A/100)·q (2L, q≈10); ×1.47 for 4L, ×1.4 for
    100 µm.
  - Multi-CB Saving: €12 + €0.04·(A/100)·q (2L), ×2 for 4L.
  - WEdirekt: ≈ €94 below 40 cm² per board, ≈ €183 above (2L q10).
  - OSH Park: $0.775/cm² × A/100 × ⌈q/3⌉ (2L), ×2 for 4L.
- **Options**: add each surcharge above that the board triggers: smallest
  drill, narrowest track/space, via-in-pad, blind/buried vias,
  castellations or edge plating, number of designs.
- **Assembly**
  - JLC Economic (one side, 0402+): 9.71 + 0.0016·J_smd·q + 3.07·U_ext +
    [THT]·(3.58 + 0.0164·J_tht·q).
  - JLC Standard: 33.77·S + 1.53·U + 0.0016·J_smd·q + THT term + ≥14.93.
  - PCBWay (q100): ≈ 3.5·U + 0.01·P_smd·q + 0.10·P_tht·q, ×1.13 for two
    sides; $29 when the promo applies.
  - Eurocircuits (q5): ≈ €110 + 5.8·U + 0.11·P + 1.1·P_tht + 55·[S = 2].
  - Beta: ≈ €199 + 0.30·P_smd·q + 1.97·P_tht + 106·[S = 2].

For the placer: a smooth area term for the gradient, and large step
penalties just before each cliff (the 100/102 mm side, WEdirekt's 40 cm²,
the drill and track tiers, a second assembly side). Parts cost is left out
and often dominates an assembled hobby board.
