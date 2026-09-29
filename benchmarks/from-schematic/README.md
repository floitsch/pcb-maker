# Schematic to board

The whole way an agent (or a person) takes: a KiCad schematic in, a board
out, with nothing but the starter files `import-kicad-netlist` writes.

```sh
benchmarks/from-schematic/run.py build/from-schematic --jobs 2
```

For each KiCad demo whose footprints the libraries have:

1. `import-kicad-netlist` makes the board from the schematic, on as many
   copper layers as the designer used, and writes `constraints.json` (every
   part placed, the outline sized from the parts, plug-in connectors on an
   edge) and `layout.json`.
2. `layout-kicad-board` lays it out with that `layout.json`, unchanged.
3. The designer's own board is scored for comparison.

A board passes when every connection is routed and KiCad finds no
unconnected item, no DRC error and no schematic parity issue. DRC warnings
(a library footprint's silkscreen on its own pads) and the schematic's own
ERC findings are reported apart: they are the library's and the
schematic's, not the layout's.

Left out: the simulation demos (no footprints), jetson, One-Air-Max,
RoyalBlue54L and Q17ng (footprints their projects do not ship), and
vme-wren (1507 parts).

Results: [docs/benchmarks.md](../../docs/benchmarks.md#schematic-to-board).
