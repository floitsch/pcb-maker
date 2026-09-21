#!/bin/sh
# Copyright (C) 2026 Toit contributors.
# Downloads the open-source boards referenced by corpus.json.
set -e
cd "$(dirname "$0")/../real/external"
mkdir -p _downloads && cd _downloads
if [ ! -d kicad-src ]; then
  git clone -q --depth 1 --filter=blob:none --sparse https://gitlab.com/kicad/code/kicad.git kicad-src
  (cd kicad-src && git sparse-checkout set demos)
fi
