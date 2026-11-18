#!/usr/bin/env bash
# Usage: tools/check-release-version.sh <tag>
# Strips one leading "v" and exits 1 unless the workspace version (Cargo.toml,
# [workspace.package]) and package.json both equal it. The tag is only ever matched
# against a regex and compared as text, never evaluated.
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: check-release-version.sh <tag>" >&2
  exit 2
fi
tag="$1"
if ! [[ "$tag" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "error: tag is not a valid version (expected vMAJOR.MINOR.PATCH)" >&2
  exit 1
fi
want="${tag#v}"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cargo="$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p;}' "$root/Cargo.toml" | head -n 1)"
pkg="$(sed -n 's/^[[:space:]]*"version":[[:space:]]*"\([^"]*\)".*/\1/p' "$root/package.json" | head -n 1)"

status=0
if [ "$cargo" != "$want" ]; then
  echo "error: Cargo.toml [workspace.package] version '$cargo' does not match tag version '$want'" >&2
  status=1
fi
if [ "$pkg" != "$want" ]; then
  echo "error: package.json version '$pkg' does not match tag version '$want'" >&2
  status=1
fi
[ "$status" -eq 0 ] && echo "version $want matches Cargo.toml and package.json"
exit "$status"
