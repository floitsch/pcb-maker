#!/usr/bin/env bash

# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.

set -euo pipefail

source_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_root="$(cd "$source_root/../.." && pwd)"
manifest="$source_root/sources.json"

if [[ $# -ne 1 ]]; then
  echo "Usage: bash benchmarks/real/fetch.sh BENCHMARK-ID" >&2
  jq -r '.sources[].id' "$manifest" >&2
  exit 2
fi

benchmark_id="$1"
entry="$(jq -c --arg id "$benchmark_id" '.sources[] | select(.id == $id)' "$manifest")"
if [[ -z "$entry" ]]; then
  echo "Unknown benchmark '$benchmark_id'" >&2
  exit 2
fi

destination="$source_root/external/$benchmark_id"
if [[ -d "$destination" ]]; then
  echo "$benchmark_id is already present; verifying checksums"
else
  temporary="$(mktemp -d)"
  trap 'rm -rf -- "$temporary"' EXIT
  archive="$temporary/source.tar.gz"
  archive_url="$(jq -r '.archive_url' <<<"$entry")"
  archive_sha="$(jq -r '.archive_sha256' <<<"$entry")"
  curl -L --fail --retry 3 --output "$archive" "$archive_url"
  printf '%s  %s\n' "$archive_sha" "$archive" | sha256sum --check --status
  mkdir -p "$destination"
  strip="$(jq -r '.archive_strip_components' <<<"$entry")"
  subpath="$(jq -r '.archive_subpath // empty' <<<"$entry")"
  if [[ -n "$subpath" ]]; then
    tar -xzf "$archive" --strip-components="$strip" -C "$destination" "$subpath"
  else
    tar -xzf "$archive" --strip-components="$strip" -C "$destination"
  fi
fi

failure=0
while IFS=$'\t' read -r relative expected; do
  target="$destination/$relative"
  if [[ ! -f "$target" ]]; then
    echo "Missing $target" >&2
    failure=1
    continue
  fi
  actual="$(sha256sum "$target" | cut -d ' ' -f 1)"
  if [[ "$actual" != "$expected" ]]; then
    echo "Checksum mismatch for $target" >&2
    failure=1
  fi
done < <(jq -r '.files | to_entries[] | [.key, .value] | @tsv' <<<"$entry")

if [[ "$failure" -ne 0 ]]; then exit 1; fi
echo "Fetched and verified $benchmark_id in ${destination#"$project_root/"}"
