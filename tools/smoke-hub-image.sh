#!/usr/bin/env bash
# Usage: [ENGINE=docker|podman] [SMOKE_TIMEOUT=<seconds>] tools/smoke-hub-image.sh <image>
# Smoke test of a built hub image, run by release.yml before anything is pushed. On a fresh
# named volume, as the image's own user 10001:10001 and with the hardening of the deploy files
# (read-only root file system, no capabilities):
#   1. `init --data /data` prints the hub secret (43 base64url characters);
#   2. `serve` starts with 127.0.0.1 published on a free host port and says it is listening;
#   3. GET /v1/info with the secret answers {"protocol":1,"maxBytes":67108864}, without it 401;
#   4. `stop` ends the container in under 5 s (the hub handles SIGTERM, about a second) with
#      exit status 0 (an unhandled SIGTERM would take the engine's 7-10 s and end with 137).
# Prints `smoke: ok` and exits 0, or names the failing step, shows the container's log and
# exits 1. Always removes its container and volume. Exit 2: wrong arguments. A call that hangs
# is cut off after SMOKE_TIMEOUT seconds (default 60). This runs in a public CI log on a
# throwaway volume: the secret is never printed on success. In Git Bash on Windows run it with
# MSYS_NO_PATHCONV=1, or the shell rewrites /data in the engine arguments.
set -euo pipefail

engine="${ENGINE:-docker}"
case "$engine" in
  docker | podman) ;;
  *)
    echo "error: ENGINE must be docker or podman, not '$engine'" >&2
    exit 2
    ;;
esac
if [ "$#" -ne 1 ]; then
  echo "usage: smoke-hub-image.sh <image>" >&2
  exit 2
fi
image="$1"
limit="${SMOKE_TIMEOUT:-60}"

suffix="$$-$(date +%s)"
name="fernjo-hub-smoke-$suffix"
volume="$name"
step="start"
init_output=""

cleanup() {
  local code=$?
  trap - EXIT
  if [ "$code" -ne 0 ]; then
    echo "smoke: FAILED at step: $step" >&2
    if [ -n "$init_output" ]; then
      echo "--- init output ---" >&2
      printf '%s\n' "$init_output" >&2
    fi
    echo "--- container log ---" >&2
    timeout 10 "$engine" logs "$name" >&2 2>&1 || true
  fi
  timeout 30 "$engine" rm -f "$name" >/dev/null 2>&1 || true
  timeout 30 "$engine" volume rm -f "$volume" >/dev/null 2>&1 || true
  if [ "$code" -ne 0 ]; then exit 1; fi
}
trap cleanup EXIT

run_limited() { timeout "$limit" "$@"; }
# The hardening of the deploy files, so the smoke test would notice a write outside /data.
common=(--user 10001:10001 --read-only --cap-drop ALL --security-opt no-new-privileges -v "$volume:/data")

step="create the volume"
run_limited "$engine" volume create "$volume" >/dev/null

step="init"
init_output="$(run_limited "$engine" run --rm "${common[@]}" "$image" init --data /data 2>&1)"
secret="$(printf '%s\n' "$init_output" | sed -n 's/^Hub secret (shown once, store it safely): \([A-Za-z0-9_-]\{43\}\)$/\1/p')"
if [ -z "$secret" ]; then
  echo "init did not print a 43-character hub secret" >&2
  exit 1
fi

step="serve"
run_limited "$engine" run -d --name "$name" -p 127.0.0.1::8787 "${common[@]}" "$image" >/dev/null

step="wait for the hub to listen"
listening=0
for _ in $(seq 1 20); do
  if run_limited "$engine" logs "$name" 2>&1 | grep -Fq "fernjo-hub listening on"; then
    listening=1
    break
  fi
  if [ "$(run_limited "$engine" inspect --format '{{.State.Running}}' "$name")" != "true" ]; then
    echo "the container stopped before it listened" >&2
    exit 1
  fi
  sleep 0.5
done
if [ "$listening" -ne 1 ]; then
  echo "the hub did not say it listens within 10 s" >&2
  exit 1
fi

step="find the published port"
mapping="$(run_limited "$engine" port "$name" 8787/tcp | head -n 1)"
port="${mapping##*:}"
if ! [[ "$port" =~ ^[0-9]+$ ]]; then
  echo "no host port found in '$mapping'" >&2
  exit 1
fi

step="GET /v1/info with the secret"
# The secret goes to curl through its config on stdin, not through its argument list.
body="$(printf 'header = "Authorization: Bearer %s"\n' "$secret" |
  curl -fsS --max-time 10 -K - "http://127.0.0.1:$port/v1/info")"
if [ "$body" != '{"protocol":1,"maxBytes":67108864}' ]; then
  echo "unexpected /v1/info answer: $body" >&2
  exit 1
fi

step="GET /v1/info without the secret"
# No -o /dev/null: curl for Windows fails with exit 23 on it; the 401 body is empty anyway.
answer="$(curl -sS --max-time 10 -w '%{http_code}' "http://127.0.0.1:$port/v1/info")"
code="${answer: -3}"
if [ "$code" != "401" ]; then
  echo "expected 401 without a secret, got $code" >&2
  exit 1
fi

step="stop"
before=$SECONDS
run_limited "$engine" stop "$name" >/dev/null
elapsed=$((SECONDS - before))
if [ "$elapsed" -ge 5 ]; then
  echo "stopping took $elapsed s (expected about one second)" >&2
  exit 1
fi
exit_code="$(run_limited "$engine" inspect --format '{{.State.ExitCode}}' "$name")"
if [ "$exit_code" != "0" ]; then
  echo "the container exited with status $exit_code, expected 0" >&2
  exit 1
fi

step="done"
echo "smoke: ok"
