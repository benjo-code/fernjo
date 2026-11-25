#!/usr/bin/env bash
# Usage: tools/hub-image-tags.sh <tag> <image>
# Prints the image references a release tag publishes, one per line: <image>:X.Y.Z,
# <image>:X.Y and <image>:latest. Exits 1 unless the tag is a plain vMAJOR.MINOR.PATCH (so a
# stray tag such as v0.3.1-rc.1 never moves `latest`) and 2 on a wrong argument count. The tag
# is only ever matched against a regex and split as text, never evaluated.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: hub-image-tags.sh <tag> <image>" >&2
  exit 2
fi
tag="$1"
image="$2"
if ! [[ "$tag" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "error: tag is not a valid version (expected vMAJOR.MINOR.PATCH)" >&2
  exit 1
fi
version="${tag#v}"
minor="${version%.*}"

echo "$image:$version"
echo "$image:$minor"
echo "$image:latest"
