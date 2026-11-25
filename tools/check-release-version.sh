#!/usr/bin/env bash
# Usage: tools/check-release-version.sh <tag>
# Strips one leading "v" and exits 1 unless the workspace version (Cargo.toml,
# [workspace.package]), package.json and the hub image tag pinned in crates/fernjo-hub/deploy
# (compose.yaml and fernjo-hub.container) all equal it. The tag is only ever matched
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
# The hub image tag pinned in a deploy file: the text after the image name on its first image
# line, without quotes or a trailing comment. Each entry is <file>:<sed prefix of the line>.
image='ghcr\.io\/benjo-code\/fernjo-hub:'
for entry in 'compose.yaml:^[[:space:]]*image:[[:space:]]*' 'fernjo-hub.container:^Image='; do
  name="${entry%%:*}"
  prefix="${entry#*:}"
  file="$root/crates/fernjo-hub/deploy/$name"
  if [ ! -f "$file" ]; then
    echo "error: $name is missing (expected in crates/fernjo-hub/deploy)" >&2
    status=1
    continue
  fi
  pin="$(sed -n "s/${prefix}\"\{0,1\}${image}\([^\"[:space:]#]*\).*\$/\1/p" "$file" | head -n 1)"
  if [ -z "$pin" ]; then
    echo "error: $name has no hub image line (ghcr.io/benjo-code/fernjo-hub:<tag>)" >&2
    status=1
  elif [ "$pin" != "$want" ]; then
    echo "error: $name pins image tag '$pin' but the release version is '$want'" >&2
    status=1
  fi
done
[ "$status" -eq 0 ] && echo "version $want matches Cargo.toml, package.json, compose.yaml and fernjo-hub.container"
exit "$status"
