#!/usr/bin/env bash
# Usage: tools/verify-installers.sh <version> <dir>
# Exits 1 unless <dir> holds exactly the expected installers, naming each missing or
# unexpected file. The names follow Tauri's bundler output; this list is the one place
# that defines them (the Windows name is confirmed by a real build, the others are
# Tauri's documented pattern and get confirmed by the first release workflow run).
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: verify-installers.sh <version> <dir>" >&2
  exit 2
fi
version="$1"
dir="$2"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "error: '$version' is not a valid version" >&2
  exit 1
fi
if [ ! -d "$dir" ]; then
  echo "error: '$dir' is not a directory" >&2
  exit 1
fi

expected=(
  "fernjo_${version}_x64-setup.exe"
  "fernjo_${version}_aarch64.dmg"
  "fernjo_${version}_x64.dmg"
  "fernjo_${version}_amd64.AppImage"
  "fernjo_${version}_amd64.deb"
)

status=0
for name in "${expected[@]}"; do
  if [ ! -f "$dir/$name" ]; then
    echo "missing: $name"
    status=1
  fi
done

shopt -s nullglob dotglob
for path in "$dir"/*; do
  base="$(basename "$path")"
  found=0
  for name in "${expected[@]}"; do
    [ "$base" = "$name" ] && found=1
  done
  if [ "$found" -eq 0 ]; then
    echo "unexpected: $base"
    status=1
  fi
done
[ "$status" -eq 0 ] && echo "all ${#expected[@]} installers present in $dir"
exit "$status"
