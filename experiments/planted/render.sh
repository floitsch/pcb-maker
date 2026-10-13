#!/bin/sh
# Copyright (C) 2026 Toit contributors.
# Renders a board's copper, courtyards and outline to a PNG.
#   experiments/planted/render.sh board.kicad_pcb out.png [layers]
layers=${3:-F.Cu,B.Cu,In1.Cu,In2.Cu,Edge.Cuts,F.Courtyard}
tmp=$(mktemp -d)
kicad-cli pcb export svg --mode-single --page-size-mode 2 --exclude-drawing-sheet --layers "$layers" -o "$tmp/b.svg" "$1" >/dev/null 2>&1
rsvg-convert -b white -w 1400 "$tmp/b.svg" -o "$2"
rm -rf "$tmp"
