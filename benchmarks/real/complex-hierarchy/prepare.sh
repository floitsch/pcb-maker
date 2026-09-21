#!/usr/bin/env bash

# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.

set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <pcb-maker-binary> <output-directory>" >&2
  exit 2
fi

binary=$1
output=$2
source_directory=$(cd "$(dirname "$0")/../external/kicad-complex-hierarchy" && pwd)

mkdir -p "$output"
cp -a "$source_directory/." "$output/"
"$binary" normalize-kicad-footprint-links \
  "$output" complex_hierarchy "$output/footprint-link-normalization.json"
"$binary" strip-kicad-copper \
  "$output/complex_hierarchy.kicad_pcb" \
  "$output/complex_hierarchy.kicad_pcb" \
  "$output/copper-strip.json"
