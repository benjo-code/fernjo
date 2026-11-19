#!/usr/bin/env bash
# Usage: tools/check-no-build-paths.sh <binary>
# Exits 1 (naming each pattern) when the file contains a CI build path: the cargo registry
# or a GitHub runner home directory. Exit 2 for a missing file or wrong usage.
set -uo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: check-no-build-paths.sh <binary>" >&2
  exit 2
fi
bin="$1"
if [ ! -f "$bin" ]; then
  echo "error: no such file: $bin" >&2
  exit 2
fi

found=0
for pattern in '.cargo/registry' '.cargo\registry' 'runneradmin' '/home/runner' '/Users/runner'; do
  n="$(grep -a -c -F -e "$pattern" "$bin")"
  if [ "${n:-0}" -gt 0 ]; then
    echo "error: $bin contains build path '$pattern' ($n matching lines)" >&2
    found=1
  fi
done
exit "$found"
