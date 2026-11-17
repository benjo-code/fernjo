#!/usr/bin/env bash
# Usage: tools/smoke-installed.sh <installers-dir> [macos-arch: arm64|x86_64]
# Installs or mounts the built installers of this runner's OS and proves they are usable.
# Linux: the .deb installs, links, declares WebKitGTK and starts under Xvfb; the AppImage extracts.
# macOS: the .dmg mounts and holds a fernjo.app binary of the expected architecture.
# Prints one `ok:` or `FAIL:` line per check and exits 1 on the first failure.
# CI only: it runs apt-get install and mounts images, so it refuses outside GitHub Actions.
set -uo pipefail

if [ "${GITHUB_ACTIONS:-}" != "true" ]; then
  echo "FAIL: refusing to run outside GitHub Actions (GITHUB_ACTIONS is not 'true'); this script installs packages and mounts images"
  exit 1
fi
if [ "$#" -lt 1 ]; then
  echo "usage: smoke-installed.sh <installers-dir> [macos-arch]" >&2
  exit 2
fi

dir="$(cd "$1" && pwd)"
want_arch="${2:-}"
work="$(mktemp -d)"
mounts=()
cleanup() {
  local m
  for m in "${mounts[@]:-}"; do
    if [ -n "$m" ]; then hdiutil detach "$m" -force >/dev/null 2>&1; fi
  done
  rm -rf "$work"
}
trap cleanup EXIT

ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; exit 1; }

# The single file matching the glob arguments; fails when there is none or more than one.
one_file() {
  local matches=( "$@" )
  [ -e "${matches[0]}" ] || fail "no file matches $*"
  [ "${#matches[@]}" -eq 1 ] || fail "expected one file for $*, got ${#matches[@]}"
  REPLY_FILE="${matches[0]}"
}

smoke_linux() {
  local deb appimage depends rc
  one_file "$dir"/fernjo_*_amd64.deb
  deb="$REPLY_FILE"
  one_file "$dir"/fernjo_*_amd64.AppImage
  appimage="$REPLY_FILE"

  depends="$(dpkg-deb --info "$deb" | grep -E '^ *Depends:' || true)"
  case "$depends" in
    *libwebkit2gtk-4.1-0*) ok "deb Depends: lists libwebkit2gtk-4.1-0" ;;
    *) fail "deb has no libwebkit2gtk-4.1-0 in Depends (got: ${depends:-none})" ;;
  esac

  sudo apt-get install -y "$deb" >"$work/apt.log" 2>&1 || { tail -30 "$work/apt.log"; fail "apt-get install of $deb"; }
  ok "apt-get install $(basename "$deb")"

  [ -x /usr/bin/fernjo ] || fail "/usr/bin/fernjo missing after install"
  if ldd /usr/bin/fernjo | grep -q 'not found'; then
    ldd /usr/bin/fernjo | grep 'not found'
    fail "ldd /usr/bin/fernjo reports unresolved libraries"
  fi
  ok "ldd /usr/bin/fernjo resolves every library"

  sudo apt-get install -y xvfb >"$work/xvfb.log" 2>&1 || { tail -30 "$work/xvfb.log"; fail "install xvfb"; }
  mkdir "$work/data"
  # timeout exits 124 when the app is still running after 15 s; any other code means it died.
  FERNJO_DATA_DIR="$work/data" xvfb-run -a timeout 15 /usr/bin/fernjo >"$work/app.log" 2>&1
  rc=$?
  if [ "$rc" -ne 124 ]; then
    cat "$work/app.log"
    fail "fernjo exited with $rc within 15 s under Xvfb (expected still running, timeout exit 124)"
  fi
  ok "fernjo still running after 15 s under Xvfb"

  chmod +x "$appimage"
  [ -x "$appimage" ] || fail "AppImage is not executable"
  ok "AppImage is executable"
  (cd "$work" && "$appimage" --appimage-extract >"$work/extract.log" 2>&1) \
    || { tail -30 "$work/extract.log"; fail "AppImage --appimage-extract"; }
  [ -d "$work/squashfs-root" ] || fail "--appimage-extract produced no squashfs-root"
  ok "AppImage --appimage-extract succeeds"
}

smoke_macos() {
  local dmg mnt bin archs
  [ -n "$want_arch" ] || fail "macOS needs the expected architecture as second argument (arm64 or x86_64)"
  one_file "$dir"/fernjo_*.dmg
  dmg="$REPLY_FILE"
  mnt="$work/mnt"
  mkdir "$mnt"
  hdiutil attach "$dmg" -nobrowse -readonly -mountpoint "$mnt" >"$work/attach.log" 2>&1 \
    || { cat "$work/attach.log"; fail "hdiutil attach $dmg"; }
  mounts+=("$mnt")
  ok "hdiutil attach $(basename "$dmg")"

  bin="$mnt/fernjo.app/Contents/MacOS/fernjo"
  [ -x "$bin" ] || { ls -la "$mnt"; fail "$bin missing in the mounted image"; }
  ok "fernjo.app/Contents/MacOS/fernjo exists"

  archs="$(lipo -archs "$bin")" || fail "lipo -archs $bin"
  [ "$archs" = "$want_arch" ] || fail "lipo -archs is '$archs', expected '$want_arch'"
  ok "lipo -archs is $archs"
}

case "$(uname -s)" in
  Linux) smoke_linux ;;
  Darwin) smoke_macos ;;
  *) fail "unsupported OS $(uname -s); Windows uses smoke-installed.ps1" ;;
esac
