#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <pcb-maker-binary> <output-directory>" >&2
  exit 2
fi

binary=$1
output=$2
source_directory=$(cd "$(dirname "$0")/../external/kicad-interf-u" && pwd)

mkdir -p "$output"
cp -a "$source_directory/." "$output/"
"$binary" normalize-kicad-footprint-links \
  "$output" interf_u "$output/footprint-link-normalization.json"
"$binary" strip-kicad-copper \
  "$output/interf_u.kicad_pcb" \
  "$output/interf_u.kicad_pcb" \
  "$output/copper-strip.json"
