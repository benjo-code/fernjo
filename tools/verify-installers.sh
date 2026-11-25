#!/usr/bin/env bash
# Usage: tools/verify-installers.sh <version> <dir>
#        tools/verify-installers.sh <version> --artifacts <artifacts-dir> <out-dir>
# The first form exits 1 unless <dir> holds exactly the expected installers, naming each missing
# or unexpected file.
# The second form is what the release workflow's publish job runs on the downloaded artifacts,
# one sub-folder per artifact (`installers-<matrix name>`). It exits 1 unless <artifacts-dir> holds
# exactly the four platform folders and each holds exactly its own platform's installers, so one
# build job cannot replace another platform's installer or slip in an extra artifact. Only then
# does it copy them into <out-dir> (which must not exist yet) and run the first form on it.
# The names follow Tauri's bundler output; this script is the one place that defines them (the
# Windows name is confirmed by a real build, the others are Tauri's documented pattern and get
# confirmed by the first release workflow run).
set -euo pipefail

usage() {
  echo "usage: verify-installers.sh <version> <dir>" >&2
  echo "       verify-installers.sh <version> --artifacts <artifacts-dir> <out-dir>" >&2
  exit 2
}

if [ "$#" -ne 2 ] && { [ "$#" -ne 4 ] || [ "$2" != "--artifacts" ]; }; then
  usage
fi
version="$1"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "error: '$version' is not a valid version" >&2
  exit 1
fi

# Matrix names of the build job and the installers each one uploads.
platforms=(windows macos-arm64 macos-x64 linux)
platform_installers() {
  case "$1" in
    windows) echo "fernjo_${version}_x64-setup.exe" ;;
    macos-arm64) echo "fernjo_${version}_aarch64.dmg" ;;
    macos-x64) echo "fernjo_${version}_x64.dmg" ;;
    linux) echo "fernjo_${version}_amd64.AppImage fernjo_${version}_amd64.deb" ;;
  esac
}

expected=()
for p in "${platforms[@]}"; do
  read -ra files <<<"$(platform_installers "$p")"
  expected+=("${files[@]}")
done

# check_exact <-f|-d> <dir> <name...>: every name present as a regular file (-f) or a folder (-d),
# not a symlink, and nothing else in <dir>.
check_exact() {
  local kind="$1" dir="$2" status=0 name base path found
  shift 2
  for name in "$@"; do
    if [ -L "$dir/$name" ] || ! test "$kind" "$dir/$name"; then
      echo "missing: ${dir##*/}/$name"
      status=1
    fi
  done
  shopt -s nullglob dotglob
  for path in "$dir"/*; do
    base="$(basename "$path")"
    found=0
    for name in "$@"; do
      [ "$base" = "$name" ] && found=1
    done
    if [ "$found" -eq 0 ]; then
      echo "unexpected: ${dir##*/}/$base"
      status=1
    fi
  done
  shopt -u nullglob dotglob
  return "$status"
}

if [ "$#" -eq 4 ]; then
  artifacts="$3"
  out="$4"
  if [ ! -d "$artifacts" ]; then
    echo "error: '$artifacts' is not a directory" >&2
    exit 1
  fi
  if [ -e "$out" ]; then
    echo "error: '$out' already exists" >&2
    exit 1
  fi
  folders=()
  for p in "${platforms[@]}"; do folders+=("installers-$p"); done
  status=0
  check_exact -d "$artifacts" "${folders[@]}" || status=1
  for p in "${platforms[@]}"; do
    d="$artifacts/installers-$p"
    if [ -d "$d" ] && [ ! -L "$d" ]; then
      read -ra files <<<"$(platform_installers "$p")"
      check_exact -f "$d" "${files[@]}" || status=1
    fi
  done
  if [ "$status" -ne 0 ]; then
    echo "error: the artifacts do not match the expected per-platform installers" >&2
    exit 1
  fi
  mkdir -p "$out"
  for p in "${platforms[@]}"; do
    read -ra files <<<"$(platform_installers "$p")"
    for f in "${files[@]}"; do
      cp -- "$artifacts/installers-$p/$f" "$out/$f"
    done
  done
  dir="$out"
else
  dir="$2"
fi

if [ ! -d "$dir" ]; then
  echo "error: '$dir' is not a directory" >&2
  exit 1
fi

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
