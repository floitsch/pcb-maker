# Discovering via repairs without supplied coordinates

`discover-kicad-vias BOARD OUTPUT CONFIG` scans via-bearing native connections,
imports supported copper trees, finds short A–B–A layer excursions, and writes
ranked candidates/configurations for `refine-kicad-vias`. A configuration
contains `routing`, `maximum_excursion_mm`, `maximum_actions`, and
`via_penalty_mm`. No target net, via coordinate or source candidate is required.

The scanner ranks exact geometric blockers by semantic owner, records other
constraints independently, and deduplicates shared/reversed paths. Via labels
use the same native UUID prefixes as verification previews. A read-only scan
automatically renders its copied source and explicitly records that it did
not run native verification.

The ECC83 ten-via board yielded five opportunities. All five were tested;
two produced independently complete eight-via boards. The shorter selected
result has 395.661088 mm of stored track. A second scan tested all three
remaining opportunities and retained that same board. This is bounded search
evidence, not proof that further via reduction is impossible.

PIC exposes a coverage gap: five of twelve via-bearing nets are rejected by
the copper importer because through-hole pads have copper contacts on both
layers. The seven supported nets yielded no short excursion. The scanner
reports partial coverage and does not call the board free of opportunities.

The experiment also exposed a baseline bug: importing a route can trim copper
inside pads. Coupled refinement now natively verifies and scores a copy of the
**original** board, and preserves that copy as its no-improvement fallback.
Imported normalization cannot silently replace the original fallback.

- [Automatic ECC83 discovery and selected repair](../../build/via-discovery-2026-09-08/ecc83-automatic/index.html)
- [Second-pass unchanged result](../../build/via-discovery-2026-09-08/ecc83-second-pass/report.json)
- [PIC coverage](../../build/via-discovery-2026-09-08/pic-final/discovery.json)

The later [demand and net-order experiments](2026-09-08-routing-demand-and-order.md)
improve routing from the empty board much further. Repair and prevention are
complementary tools; the local repair result is no longer the best ECC83 route.
