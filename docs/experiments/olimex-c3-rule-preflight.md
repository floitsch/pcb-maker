# Olimex ESP32-C3 rule-compatibility preflight

Date: 2026-09-03. Retained run: sequence 231.

The pinned Olimex ESP32-C3 DevKit Lipo Rev C is the third ordinary KiCad
project accepted structurally by the cold adapter. It has 61 footprints, 34
routable named nets, 140 native connection items, 768 inherited tracks, 88
vias, 160 copper/teardrop zones, and one retained rule area. Its single Default
net class uses 0.127 mm width/clearance and 0.7/0.4 mm vias.

Freerouting reports 148 initial items and zero remaining after 19 passes, but
that result is not admissible. KiCad reports two open items, 17 newly
introduced DRC findings, and one new parity finding. Nine of the new DRC
findings are traces inside the 1.85 mm local clearance of mounting-hole pads.
That footprint/pad-local constraint is not represented by the uniform
net-class geometry supplied through the current DSN adapter. The source's
large pre-existing warning set remains visible and is not counted as a router
regression.

The imported result preserves placement at the declared 0.0001 mm comparison
grid; two footprints are normalized by only 0.00004 mm in raw coordinates.
The failure is therefore not component motion. Front/back inspection shows a
dense plausible route, but native geometry remains authoritative.

This board must not enter the Freerouting solve-rate denominator until a
capability preflight either proves that all active local constraints survive
DSN/SES exchange or lowers them into explicit obstacles with an equivalence
test. pcb-maker was deliberately not run: its current raster config likewise
has no per-pad trace-clearance representation. Spending A* budget would not
test the identified missing rule.

Authoritative report:
`build/sequence-231-m0-olimex-c3-freerouting/competitive-result.json`.
